//! The artistic effects: each one a filter of the pixels of an image.
//!
//! Every filter measures in thousandths of the longer side of the image,
//! so that an effect looks the same on a small copy of a picture (the
//! gallery) as on the picture itself. Their one setting is in Excel's
//! units (`ArtisticKind::amount_range`). None of them is Office's own
//! code: each does what the name of the effect says, by the usual means
//! (Gaussian blurs, edge detection, the Kuwahara filter, quantising,
//! noise), and how close it comes to Office's rendering is for the eye
//! to judge.

use super::picture::Bitmap;
use super::render::gaussian;
use opensheet_engine::picture_style::{Artistic, ArtisticKind};

/// The three colours of an image as planes of numbers from 0 to 1: red,
/// green, blue.
struct Planes {
    w: usize,
    h: usize,
    rgb: [Vec<f32>; 3],
}

impl Planes {
    fn of(bitmap: &Bitmap) -> Planes {
        let (w, h) = (bitmap.width as usize, bitmap.height as usize);
        let plane = |channel: usize| {
            bitmap
                .data
                .chunks_exact(4)
                .map(|pixel| pixel[channel] as f32 / 255.0)
                .collect::<Vec<f32>>()
        };
        // Stored blue first.
        Planes {
            w,
            h,
            rgb: [plane(2), plane(1), plane(0)],
        }
    }

    fn grey(w: usize, h: usize, plane: Vec<f32>) -> Planes {
        Planes {
            w,
            h,
            rgb: [plane.clone(), plane.clone(), plane],
        }
    }

    fn luma(&self) -> Vec<f32> {
        (0..self.w * self.h)
            .map(|i| 0.299 * self.rgb[0][i] + 0.587 * self.rgb[1][i] + 0.114 * self.rgb[2][i])
            .collect()
    }

    fn blurred(&self, sigma: f32) -> Planes {
        let mut rgb = self.rgb.clone();
        for plane in &mut rgb {
            gaussian_clamped(plane, self.w, self.h, sigma);
        }
        Planes {
            w: self.w,
            h: self.h,
            rgb,
        }
    }

    fn map(&mut self, f: impl Fn(f32) -> f32) {
        for plane in &mut self.rgb {
            plane.iter_mut().for_each(|v| *v = f(*v));
        }
    }
}

/// A Gaussian blur whose edges repeat the image instead of darkening
/// towards nothing: the plane is blurred with what lies beyond its edges
/// taken to be like its border.
fn gaussian_clamped(plane: &mut Vec<f32>, w: usize, h: usize, sigma: f32) {
    if sigma < 0.3 || w == 0 || h == 0 {
        return;
    }
    // Blurring a plane of ones the same way tells how much of each
    // kernel fell inside the image.
    let mut weight = vec![1.0f32; w * h];
    gaussian(plane, w, h, sigma);
    gaussian(&mut weight, w, h, sigma);
    for (v, k) in plane.iter_mut().zip(weight) {
        *v /= k.max(1e-4);
    }
}

/// A number from 0 to 1 that depends on two whole numbers only: the
/// grain of film, the tooth of paper.
fn hash(x: i64, y: i64, seed: u32) -> f32 {
    let mut v = (x as u32).wrapping_mul(0x85EB_CA6B)
        ^ (y as u32).wrapping_mul(0xC2B2_AE35)
        ^ seed.wrapping_mul(0x27D4_EB2F);
    v ^= v >> 15;
    v = v.wrapping_mul(0x2C1B_3C6D);
    v ^= v >> 12;
    v = v.wrapping_mul(0x297A_2D39);
    v ^= v >> 15;
    (v & 0xFF_FFFF) as f32 / 16_777_215.0
}

/// `hash` made smooth: its values at the corners of cells `cell` pixels
/// wide, blended between them.
fn smooth_noise(x: f32, y: f32, cell: f32, seed: u32) -> f32 {
    let (fx, fy) = (x / cell, y / cell);
    let (x0, y0) = (fx.floor(), fy.floor());
    let ease = |t: f32| t * t * (3.0 - 2.0 * t);
    let (tx, ty) = (ease(fx - x0), ease(fy - y0));
    let (x0, y0) = (x0 as i64, y0 as i64);
    let top = hash(x0, y0, seed) * (1.0 - tx) + hash(x0 + 1, y0, seed) * tx;
    let bottom = hash(x0, y0 + 1, seed) * (1.0 - tx) + hash(x0 + 1, y0 + 1, seed) * tx;
    top * (1.0 - ty) + bottom * ty
}

/// How strong the edges of a plane are at each pixel (Sobel).
fn edges(plane: &[f32], w: usize, h: usize) -> Vec<f32> {
    let at = |x: i64, y: i64| {
        plane[y.clamp(0, h as i64 - 1) as usize * w + x.clamp(0, w as i64 - 1) as usize]
    };
    let mut out = vec![0.0f32; w * h];
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            let gx = at(x + 1, y - 1) + 2.0 * at(x + 1, y) + at(x + 1, y + 1)
                - at(x - 1, y - 1)
                - 2.0 * at(x - 1, y)
                - at(x - 1, y + 1);
            let gy = at(x - 1, y + 1) + 2.0 * at(x, y + 1) + at(x + 1, y + 1)
                - at(x - 1, y - 1)
                - 2.0 * at(x, y - 1)
                - at(x + 1, y - 1);
            out[y as usize * w + x as usize] = (gx * gx + gy * gy).sqrt();
        }
    }
    out
}

fn smoothstep(low: f32, high: f32, v: f32) -> f32 {
    let t = ((v - low) / (high - low).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The Kuwahara filter, which paints: each pixel takes the mean colour
/// of the calmest of the four squares of side `radius` that meet at it,
/// so that flat areas smooth out and edges stay sharp. Sums over squares
/// come from tables of running sums, whatever the radius.
fn kuwahara(source: &Planes, radius: usize) -> Planes {
    let (w, h) = (source.w, source.h);
    let radius = radius.max(1);
    let luma = source.luma();
    // Running sums with a row and a column of noughts in front.
    let table = |values: &dyn Fn(usize) -> f64| {
        let mut sums = vec![0.0f64; (w + 1) * (h + 1)];
        for y in 0..h {
            let mut row = 0.0;
            for x in 0..w {
                row += values(y * w + x);
                sums[(y + 1) * (w + 1) + x + 1] = sums[y * (w + 1) + x + 1] + row;
            }
        }
        sums
    };
    let sums: Vec<Vec<f64>> = (0..3)
        .map(|c| table(&|i| source.rgb[c][i] as f64))
        .collect();
    let light = table(&|i| luma[i] as f64);
    let squares = table(&|i| (luma[i] * luma[i]) as f64);
    let total = |sums: &[f64], x0: usize, y0: usize, x1: usize, y1: usize| {
        sums[y1 * (w + 1) + x1] - sums[y0 * (w + 1) + x1] - sums[y1 * (w + 1) + x0]
            + sums[y0 * (w + 1) + x0]
    };
    let mut out = Planes {
        w,
        h,
        rgb: [vec![0.0; w * h], vec![0.0; w * h], vec![0.0; w * h]],
    };
    for y in 0..h {
        for x in 0..w {
            let mut best = (f64::INFINITY, (0, 0, 1, 1));
            for (left, up) in [(true, true), (false, true), (true, false), (false, false)] {
                let (x0, x1) = if left {
                    (x.saturating_sub(radius), x + 1)
                } else {
                    (x, (x + radius + 1).min(w))
                };
                let (y0, y1) = if up {
                    (y.saturating_sub(radius), y + 1)
                } else {
                    (y, (y + radius + 1).min(h))
                };
                let count = ((x1 - x0) * (y1 - y0)) as f64;
                let mean = total(&light, x0, y0, x1, y1) / count;
                let variance = total(&squares, x0, y0, x1, y1) / count - mean * mean;
                if variance < best.0 {
                    best = (variance, (x0, y0, x1, y1));
                }
            }
            let (x0, y0, x1, y1) = best.1;
            let count = ((x1 - x0) * (y1 - y0)) as f64;
            for c in 0..3 {
                out.rgb[c][y * w + x] = (total(&sums[c], x0, y0, x1, y1) / count) as f32;
            }
        }
    }
    out
}

/// The strokes of a pencil across the image: streaks along one diagonal,
/// lighter and darker, a little uneven along their length.
fn hatching(x: usize, y: usize, unit: f32, seed: u32) -> f32 {
    let across = (x as f32 + y as f32) / (1.6 * unit).max(1.0);
    let along = (x as f32 - y as f32) / (22.0 * unit).max(1.0);
    hash(across.floor() as i64, along.floor() as i64, seed)
}

/// What an effect makes of an image. Transparent pixels stay as
/// transparent as they were.
pub fn apply(source: &Bitmap, artistic: &Artistic) -> Bitmap {
    let planes = Planes::of(source);
    let (w, h) = (planes.w, planes.h);
    if w == 0 || h == 0 {
        return source.clone();
    }
    // A thousandth of the longer side, in pixels.
    let unit = (w.max(h) as f32 / 1000.0).max(0.05);
    let (low, high, _) = artistic.kind.amount_range();
    let amount = artistic.amount.clamp(low, high);
    // The setting from 0 to 1.
    let share = (amount - low) / (high - low).max(1e-6);
    let made = match artistic.kind {
        ArtisticKind::Blur => planes.blurred((0.35 * amount * unit).max(0.3)),
        ArtisticKind::PencilGrayscale | ArtisticKind::PencilSketch => {
            // A pencil drawing: the image divided by a blurred copy of
            // itself leaves its edges dark on white (a "colour dodge" of
            // the negative), which is then shaded with strokes.
            let grey = planes.luma();
            let sketch = artistic.kind == ArtisticKind::PencilSketch;
            let mut soft = grey.clone();
            gaussian_clamped(
                &mut soft,
                w,
                h,
                (2.0 + 6.0 * if sketch { 0.3 } else { share }) * unit,
            );
            let out: Vec<f32> = (0..w * h)
                .map(|i| {
                    let dodge = (grey[i] / soft[i].max(0.02)).min(1.0);
                    let stroke = hatching(i % w, i / w, unit, 7);
                    if sketch {
                        // Lines only, pressed harder or softer, with
                        // strokes in the darkest parts.
                        let line = dodge.powf(2.0 + 10.0 * share);
                        let shade = if grey[i] < 0.35 {
                            0.82 + 0.18 * stroke
                        } else {
                            1.0
                        };
                        (line * shade).clamp(0.0, 1.0)
                    } else {
                        // The tones of the image stay, drawn in strokes.
                        let tone = 0.45 * dodge + 0.55 * grey[i];
                        (tone * (0.86 + 0.14 * stroke) + 0.06).clamp(0.0, 1.0)
                    }
                })
                .collect();
            Planes::grey(w, h, out)
        }
        ArtisticKind::LineDrawing => {
            // Where a finely blurred copy is darker than a widely
            // blurred one there is a line: black on white.
            let grey = planes.luma();
            let (mut fine, mut wide) = (grey.clone(), grey);
            let width = (0.6 + 2.4 * share) * unit;
            gaussian_clamped(&mut fine, w, h, width.max(0.5));
            gaussian_clamped(&mut wide, w, h, (width * 2.6).max(1.2));
            let out = (0..w * h)
                .map(|i| smoothstep(-0.035, -0.004, fine[i] - wide[i]))
                .collect();
            Planes::grey(w, h, out)
        }
        ArtisticKind::ChalkSketch => {
            // Chalk on a grey board: the edges in white, broken by the
            // grain of the board, and the light parts dusted.
            let grey = planes.luma();
            let mut soft = grey.clone();
            gaussian_clamped(&mut soft, w, h, 1.2 * unit);
            let strong = edges(&soft, w, h);
            let press = 0.5 + 0.5 * share;
            let out = (0..w * h)
                .map(|i| {
                    let (x, y) = (i % w, i / w);
                    let grain = 0.55
                        + 0.45
                            * hash(
                                (x as f32 / unit.max(1.0)) as i64,
                                (y as f32 / unit.max(1.0)) as i64,
                                11,
                            );
                    let line = smoothstep(0.08, 0.5, strong[i] * 2.2) * grain * press;
                    let dust =
                        smoothstep(0.55, 1.0, grey[i]) * 0.35 * hatching(x, y, unit, 3) * press;
                    (0.27 + 0.73 * (line + dust).min(1.0)).clamp(0.0, 1.0)
                })
                .collect();
            Planes::grey(w, h, out)
        }
        ArtisticKind::PaintBrush => kuwahara(
            &planes,
            ((1.5 + 1.2 * amount) * unit).round().max(1.0) as usize,
        ),
        ArtisticKind::GlowDiffused => {
            // A soft copy laid over the image the way light adds up.
            let soft = planes.blurred((3.0 + 1.4 * amount) * unit);
            let mut out = planes.rgb.clone();
            for c in 0..3 {
                for (v, glow) in out[c].iter_mut().zip(&soft.rgb[c]) {
                    *v = 1.0 - (1.0 - *v) * (1.0 - 0.55 * glow);
                }
            }
            Planes { w, h, rgb: out }
        }
        ArtisticKind::LightScreen => {
            // Seen through a screen of small lit squares: each lighter
            // towards one corner, with a pale line between them.
            let cell = ((4.0 + 2.2 * amount) * unit).max(3.0);
            let mut out = planes.rgb.clone();
            for i in 0..w * h {
                let (x, y) = ((i % w) as f32, (i / w) as f32);
                let (u, v) = ((x / cell).fract(), (y / cell).fract());
                let lit = 0.10 + 0.22 * (1.0 - (u + v) / 2.0);
                let rule = if u < 1.2 / cell || v < 1.2 / cell {
                    0.25
                } else {
                    0.0
                };
                let add = (lit + rule).min(0.6);
                for plane in &mut out {
                    plane[i] += (1.0 - plane[i]) * add;
                }
            }
            Planes { w, h, rgb: out }
        }
        ArtisticKind::FilmGrain => {
            // Noise in the light of the image, in clumps the size of
            // the grain.
            let grain = (0.5 + 2.5 * share) * unit;
            let mut noise: Vec<f32> = (0..w * h)
                .map(|i| hash((i % w) as i64, (i / w) as i64, 5) - 0.5)
                .collect();
            gaussian_clamped(&mut noise, w, h, grain);
            // Blurring flattens noise: bring it back to its strength.
            let spread = (noise.iter().map(|n| n * n).sum::<f32>() / (w * h) as f32)
                .sqrt()
                .max(1e-5);
            let mut out = planes.rgb.clone();
            for plane in &mut out {
                for (v, n) in plane.iter_mut().zip(&noise) {
                    *v = (*v + n / spread * 0.045).clamp(0.0, 1.0);
                }
            }
            Planes { w, h, rgb: out }
        }
        ArtisticKind::MosaicBubbles => {
            // Round tiles, each of the mean colour of what it covers,
            // lit from above, on that colour darkened.
            let cell = ((5.0 + 0.3 * amount) * unit).max(3.0);
            let (columns, rows) = (
                (w as f32 / cell).ceil() as usize,
                (h as f32 / cell).ceil() as usize,
            );
            let mut sums = vec![[0.0f32; 4]; columns * rows];
            for i in 0..w * h {
                let tile = &mut sums
                    [((i / w) as f32 / cell) as usize * columns + ((i % w) as f32 / cell) as usize];
                for c in 0..3 {
                    tile[c] += planes.rgb[c][i];
                }
                tile[3] += 1.0;
            }
            let mut out = planes.rgb.clone();
            for i in 0..w * h {
                let (x, y) = ((i % w) as f32, (i / w) as f32);
                let (column, row) = ((x / cell) as usize, (y / cell) as usize);
                let tile = sums[row * columns + column];
                let (dx, dy) = (
                    x + 0.5 - (column as f32 + 0.5) * cell,
                    y + 0.5 - (row as f32 + 0.5) * cell,
                );
                let from_centre = (dx * dx + dy * dy).sqrt() / (cell / 2.0);
                let inside = 1.0 - smoothstep(0.86, 1.0, from_centre);
                let shine = (1.0
                    - ((dx + cell * 0.18).powi(2) + (dy + cell * 0.18).powi(2)).sqrt()
                        / (cell * 0.45))
                    .max(0.0)
                    * 0.28;
                for c in 0..3 {
                    let mean = tile[c] / tile[3].max(1.0);
                    let bubble = (mean * (1.0 - 0.18 * from_centre.min(1.0)) + shine).min(1.0);
                    out[c][i] = bubble * inside + mean * 0.62 * (1.0 - inside);
                }
            }
            Planes { w, h, rgb: out }
        }
        ArtisticKind::Glass => {
            // Seen through uneven glass: every pixel shows a neighbour,
            // which one decided by smooth noise.
            let reach = (1.0 + 7.0 * share) * unit;
            let cell = (4.0 * unit).max(2.0);
            let at = |plane: &[f32], x: f32, y: f32| {
                plane[(y.round().clamp(0.0, h as f32 - 1.0)) as usize * w
                    + x.round().clamp(0.0, w as f32 - 1.0) as usize]
            };
            let mut out = planes.rgb.clone();
            for i in 0..w * h {
                let (x, y) = ((i % w) as f32, (i / w) as f32);
                let dx = (smooth_noise(x, y, cell, 21) - 0.5) * 2.0 * reach;
                let dy = (smooth_noise(x, y, cell, 22) - 0.5) * 2.0 * reach;
                for c in 0..3 {
                    out[c][i] = at(&planes.rgb[c], x + dx, y + dy);
                }
            }
            Planes { w, h, rgb: out }
        }
        ArtisticKind::Cutout => {
            // Cut out of sheets of coloured paper: smoothed, then each
            // colour brought to one of a few shades.
            let levels = (amount.round() + 2.0).max(2.0);
            let mut out = kuwahara(&planes, (3.0 * unit).round().max(1.0) as usize);
            out.map(|v| ((v * levels).floor().min(levels - 1.0) + 0.5) / levels);
            out
        }
        ArtisticKind::Photocopy => {
            // A photocopy: dark where the image is darker than what
            // surrounds it, white elsewhere, in grey toner.
            let grey = planes.luma();
            let mut around = grey.clone();
            gaussian_clamped(&mut around, w, h, (9.0 - 0.7 * amount).max(1.5) * unit);
            let out = (0..w * h)
                .map(|i| {
                    let darker = around[i] - grey[i];
                    let toner =
                        smoothstep(0.01, 0.09, darker) + 0.35 * smoothstep(0.75, 0.25, grey[i]);
                    1.0 - 0.82 * toner.min(1.0)
                })
                .collect();
            Planes::grey(w, h, out)
        }
        ArtisticKind::GlowEdges => {
            // The edges of each colour, lit on black.
            let soft = planes.blurred((0.4 + 0.25 * amount) * unit);
            let mut rgb = [Vec::new(), Vec::new(), Vec::new()];
            for c in 0..3 {
                rgb[c] = edges(&soft.rgb[c], w, h)
                    .into_iter()
                    .map(|e| (e * 2.6).min(1.0))
                    .collect();
            }
            Planes { w, h, rgb }
        }
        ArtisticKind::Marker => {
            // Felt pens: flat areas of a few strong colours with a dark
            // line where they meet.
            let flat = kuwahara(
                &planes,
                ((1.0 + 0.04 * amount) * unit).round().max(1.0) as usize,
            );
            let strong = edges(&flat.luma(), w, h);
            let mut out = flat;
            let light = out.luma();
            for c in 0..3 {
                for i in 0..w * h {
                    // More colour, in six shades.
                    let vivid = (light[i] + (out.rgb[c][i] - light[i]) * 1.35).clamp(0.0, 1.0);
                    let shade = ((vivid * 6.0).floor().min(5.0) + 0.5) / 6.0;
                    out.rgb[c][i] = shade * (1.0 - 0.6 * smoothstep(0.25, 0.7, strong[i]));
                }
            }
            out
        }
    };
    // Back to pixels, with as much of the image as the effect lets
    // through.
    let through = artistic.transparency.clamp(0.0, 1.0);
    let mut data = source.data.clone();
    for (i, pixel) in data.chunks_exact_mut(4).enumerate() {
        for (c, at) in [(0usize, 2usize), (1, 1), (2, 0)] {
            let value = made.rgb[c][i] * (1.0 - through) + planes.rgb[c][i] * through;
            pixel[at] = (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    Bitmap {
        width: source.width,
        height: source.height,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn photo() -> Bitmap {
        super::super::styles::sample_photo(160, 120)
    }

    fn mean(bitmap: &Bitmap) -> [f32; 3] {
        let mut sum = [0.0f32; 3];
        for pixel in bitmap.data.chunks_exact(4) {
            for c in 0..3 {
                sum[c] += pixel[c] as f32;
            }
        }
        sum.map(|v| v / (bitmap.width * bitmap.height) as f32)
    }

    /// How much neighbouring pixels differ, on average.
    fn roughness(bitmap: &Bitmap) -> f32 {
        let mut sum = 0.0;
        for y in 0..bitmap.height {
            for x in 1..bitmap.width {
                let (a, b) = (bitmap.offset(x - 1, y), bitmap.offset(x, y));
                sum += (bitmap.data[a + 1] as f32 - bitmap.data[b + 1] as f32).abs();
            }
        }
        sum / (bitmap.width * bitmap.height) as f32
    }

    fn colours(bitmap: &Bitmap) -> usize {
        bitmap
            .data
            .chunks_exact(4)
            .map(|p| (p[0], p[1], p[2]))
            .collect::<std::collections::HashSet<_>>()
            .len()
    }

    #[test]
    fn every_effect_changes_the_image_and_keeps_its_size_and_alpha() {
        let mut source = photo();
        source.data[3] = 0;
        let mut seen = std::collections::HashSet::new();
        for kind in ArtisticKind::ALL {
            let made = apply(&source, &Artistic::preset(kind));
            assert_eq!(
                (made.width, made.height),
                (source.width, source.height),
                "{kind:?}"
            );
            assert_ne!(made.data, source.data, "{kind:?}");
            assert_eq!(made.data[3], 0, "{kind:?}");
            assert!(
                made.data.chunks_exact(4).skip(1).all(|p| p[3] == 255),
                "{kind:?}"
            );
            // The same twice: nothing in a filter is left to chance.
            assert_eq!(
                apply(&source, &Artistic::preset(kind)).data,
                made.data,
                "{kind:?}"
            );
            assert!(seen.insert(made.data), "{kind:?} looks like another effect");
        }
        // Wholly let through, an effect leaves the image as it was.
        let through = Artistic {
            transparency: 1.0,
            ..Artistic::preset(ArtisticKind::Cutout)
        };
        assert_eq!(apply(&source, &through).data, source.data);
    }

    #[test]
    fn effects_do_what_their_names_say() {
        let source = photo();
        let made = |kind: ArtisticKind, amount: f32| {
            apply(
                &source,
                &Artistic {
                    kind,
                    transparency: 0.0,
                    amount,
                },
            )
        };
        let grey = |bitmap: &Bitmap| {
            bitmap
                .data
                .chunks_exact(4)
                .all(|p| p[0] == p[1] && p[1] == p[2])
        };
        // A blur smooths, the more the larger its radius, and keeps the
        // light of the image.
        let (little, much) = (
            made(ArtisticKind::Blur, 10.0),
            made(ArtisticKind::Blur, 60.0),
        );
        assert!(roughness(&little) < roughness(&source) && roughness(&much) < roughness(&little));
        assert!((mean(&much)[1] - mean(&source)[1]).abs() < 4.0);
        // Grain roughens.
        assert!(roughness(&made(ArtisticKind::FilmGrain, 40.0)) > roughness(&source) * 1.5);
        // Pencils, chalk, photocopies and line drawings have no colour.
        for kind in [
            ArtisticKind::PencilGrayscale,
            ArtisticKind::PencilSketch,
            ArtisticKind::LineDrawing,
            ArtisticKind::ChalkSketch,
            ArtisticKind::Photocopy,
        ] {
            assert!(grey(&made(kind, kind.amount_range().2)), "{kind:?}");
        }
        // A line drawing and a sketch are mostly white paper; chalk is
        // mostly board; glowing edges mostly black.
        assert!(mean(&made(ArtisticKind::LineDrawing, 6.0))[0] > 200.0);
        assert!(mean(&made(ArtisticKind::PencilSketch, 14.0))[0] > 170.0);
        assert!(mean(&made(ArtisticKind::ChalkSketch, 4.0))[0] < 130.0);
        assert!(mean(&made(ArtisticKind::GlowEdges, 5.0))[1] < 70.0);
        // A cutout has few shades, fewer when fewer are asked for.
        let (few, more) = (
            colours(&made(ArtisticKind::Cutout, 0.0)),
            colours(&made(ArtisticKind::Cutout, 6.0)),
        );
        assert!(few <= 8 && few < more && more <= 512, "{few} {more}");
        // A light screen and a diffused glow lighten.
        assert!(mean(&made(ArtisticKind::LightScreen, 3.0))[1] > mean(&source)[1] + 8.0);
        assert!(mean(&made(ArtisticKind::GlowDiffused, 5.0))[1] > mean(&source)[1] + 8.0);
        // Painting flattens the grain of the photograph.
        assert!(roughness(&made(ArtisticKind::PaintBrush, 2.0)) < roughness(&source));
    }

    #[test]
    fn painting_keeps_edges_and_flattens_what_lies_between() {
        // Left half dark, right half light, with noise.
        let (w, h) = (40usize, 20usize);
        let mut planes = Planes {
            w,
            h,
            rgb: [vec![0.0; w * h], vec![0.0; w * h], vec![0.0; w * h]],
        };
        for i in 0..w * h {
            let value = if i % w < 20 { 0.2 } else { 0.8 }
                + (hash((i % w) as i64, (i / w) as i64, 1) - 0.5) * 0.1;
            for plane in &mut planes.rgb {
                plane[i] = value;
            }
        }
        let painted = kuwahara(&planes, 3);
        // No pixel is a mix of the two halves.
        assert!(
            painted.rgb[0]
                .iter()
                .all(|v| (*v - 0.2).abs() < 0.06 || (*v - 0.8).abs() < 0.06)
        );
        // And the noise is less.
        let spread = |values: &[f32]| values.iter().take(10).map(|v| (v - 0.2).abs()).sum::<f32>();
        assert!(spread(&painted.rgb[0][5 * w..]) < spread(&planes.rgb[0][5 * w..]));
        // A blur near an edge of the image does not darken towards it.
        let mut flat = vec![0.5f32; 30 * 30];
        gaussian_clamped(&mut flat, 30, 30, 4.0);
        assert!(flat.iter().all(|v| (*v - 0.5).abs() < 1e-3));
    }
}
