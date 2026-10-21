//! A picture with what its style adds around and over its pixels: its
//! border, its bevel, its rotation in space, its shadow, its glow and its
//! reflection, all made in pixels so that the screen and the page show
//! the same thing. A shadow is the outline of the picture blurred with a
//! Gaussian kernel, not a stack of translucent copies.
//!
//! Everything is worked out in the picture's own frame, before it is
//! turned on the sheet: the result is a bitmap larger than the frame and
//! centred on it, which is then turned like a plain picture.

use super::picture::Bitmap;
use opensheet_engine::Color;
use opensheet_engine::drawing::{Effects, Glow, Outline, Shadow};
use opensheet_engine::picture_style::{Bevel, Reflection, Rotation3d};

/// The largest number of pixels a picture is composed at; a larger one
/// is composed from a smaller copy.
const MAX_AREA: usize = 2_400_000;

/// What a picture wears.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dress {
    pub outline: Option<Outline>,
    pub effects: Effects,
    /// The turn of the picture on the sheet, degrees clockwise: a shadow
    /// falls where the sheet says, however the picture is turned.
    pub turn: f32,
}

impl Dress {
    /// True when the picture is drawn from its own pixels alone (soft
    /// edges are made in them).
    pub fn is_plain(&self) -> bool {
        let effects = &self.effects;
        self.outline.is_none()
            && effects.shadow.is_none()
            && effects.glow.is_none()
            && effects.reflection.is_none()
            && effects.bevel.is_none()
            && effects.rotation.is_none()
    }

    /// The dress as numbers: two dresses with the same numbers give the
    /// same bitmap.
    pub fn bits(&self) -> Vec<u32> {
        let mut bits = Vec::new();
        let mut real = |values: &[f32]| bits.extend(values.iter().map(|v| v.to_bits()));
        match &self.outline {
            Some(outline) => real(&[
                1.0,
                outline.color.value() as f32,
                outline.width,
                outline.dash.pattern().len() as f32,
            ]),
            None => real(&[0.0]),
        }
        let effects = &self.effects;
        match &effects.shadow {
            Some(s) => real(&[
                1.0,
                s.color.value() as f32,
                s.transparency,
                s.size,
                s.blur,
                s.angle,
                s.distance,
                s.squash,
                s.skew,
            ]),
            None => real(&[0.0]),
        }
        match &effects.glow {
            Some(glow) => real(&[
                1.0,
                glow.color.value() as f32,
                glow.transparency,
                glow.radius,
            ]),
            None => real(&[0.0]),
        }
        match &effects.reflection {
            Some(r) => real(&[1.0, r.transparency, r.size, r.distance, r.blur]),
            None => real(&[0.0]),
        }
        match &effects.bevel {
            Some(bevel) => real(&[1.0, bevel.preset as u8 as f32, bevel.width, bevel.height]),
            None => real(&[0.0]),
        }
        match &effects.rotation {
            Some(r) => real(&[1.0, r.x, r.y, r.z, r.perspective]),
            None => real(&[0.0]),
        }
        real(&[self.turn]);
        bits
    }
}

/// Pixels with their colours multiplied by their alpha (blue, green, red,
/// alpha, 0 to 1 each), and where the first one is from the top-left
/// corner of the picture's frame.
#[derive(Clone)]
struct Layer {
    w: usize,
    h: usize,
    ox: i32,
    oy: i32,
    px: Vec<[f32; 4]>,
}

impl Layer {
    fn new(w: usize, h: usize, ox: i32, oy: i32) -> Layer {
        Layer {
            w,
            h,
            ox,
            oy,
            px: vec![[0.0; 4]; w * h],
        }
    }

    /// One colour through a mask.
    fn tinted(
        w: usize,
        h: usize,
        ox: i32,
        oy: i32,
        mask: &[f32],
        color: Color,
        strength: f32,
    ) -> Layer {
        let value = color.value();
        let ink = [
            (value & 0xFF) as f32 / 255.0,
            ((value >> 8) & 0xFF) as f32 / 255.0,
            ((value >> 16) & 0xFF) as f32 / 255.0,
        ];
        let px = mask
            .iter()
            .map(|m| {
                let a = (m * strength).clamp(0.0, 1.0);
                [ink[0] * a, ink[1] * a, ink[2] * a, a]
            })
            .collect();
        Layer { w, h, ox, oy, px }
    }

    /// The colour between pixels, blended from the four around the point
    /// (pixel centres at whole coordinates); nothing outside.
    fn sample(&self, x: f32, y: f32) -> [f32; 4] {
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
            if weight <= 0.0 || px < 0 || py < 0 || px >= self.w as i64 || py >= self.h as i64 {
                continue;
            }
            let p = self.px[py as usize * self.w + px as usize];
            for c in 0..4 {
                sum[c] += p[c] * weight;
            }
        }
        sum
    }

    /// Paint `top` over this layer, where they overlap.
    fn over(&mut self, top: &Layer) {
        for y in 0..top.h {
            let ty = y as i64 + top.oy as i64 - self.oy as i64;
            if ty < 0 || ty >= self.h as i64 {
                continue;
            }
            for x in 0..top.w {
                let tx = x as i64 + top.ox as i64 - self.ox as i64;
                if tx < 0 || tx >= self.w as i64 {
                    continue;
                }
                let s = top.px[y * top.w + x];
                if s[3] <= 0.0 {
                    continue;
                }
                let d = &mut self.px[ty as usize * self.w + tx as usize];
                for c in 0..4 {
                    d[c] = s[c] + d[c] * (1.0 - s[3]);
                }
            }
        }
    }

    /// The layer as pixels whose colours are not multiplied by alpha.
    fn into_bitmap(self) -> Bitmap {
        let mut data = Vec::with_capacity(self.px.len() * 4);
        for p in &self.px {
            let a = p[3].clamp(0.0, 1.0);
            if a <= 0.0 {
                data.extend_from_slice(&[0; 4]);
            } else {
                let byte = |v: f32| ((v / a).clamp(0.0, 1.0) * 255.0).round() as u8;
                data.extend_from_slice(&[
                    byte(p[0]),
                    byte(p[1]),
                    byte(p[2]),
                    (a * 255.0).round() as u8,
                ]);
            }
        }
        Bitmap {
            width: self.w as u32,
            height: self.h as u32,
            data,
        }
    }

    /// The box of what is not transparent: x0, y0, x1, y1 in the frame's
    /// coordinates, the last two excluded.
    fn inked(&self) -> Option<(i32, i32, i32, i32)> {
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        for y in 0..self.h {
            for x in 0..self.w {
                if self.px[y * self.w + x][3] > 0.02 {
                    (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
                }
            }
        }
        (x1 > x0 && y1 > y0).then(|| {
            (
                x0 as i32 + self.ox,
                y0 as i32 + self.oy,
                x1 as i32 + self.ox,
                y1 as i32 + self.oy,
            )
        })
    }
}

// ---- blur --------------------------------------------------------------------

/// Blur one plane of `w` by `h` values with a Gaussian of deviation
/// `sigma` pixels; beyond the plane there is nothing. A wide blur is
/// done on a reduced copy of the plane, which a Gaussian allows: the
/// detail it would lose is the detail the blur removes.
pub fn gaussian(plane: &mut Vec<f32>, w: usize, h: usize, sigma: f32) {
    if sigma < 0.3 || w == 0 || h == 0 {
        return;
    }
    let factor = (sigma / 3.0).floor() as usize;
    if factor < 2 {
        blur_exact(plane, w, h, sigma);
        return;
    }
    let (sw, sh) = (w.div_ceil(factor), h.div_ceil(factor));
    let mut small = vec![0.0f32; sw * sh];
    for y in 0..h {
        for x in 0..w {
            small[(y / factor) * sw + x / factor] += plane[y * w + x];
        }
    }
    let area = (factor * factor) as f32;
    small.iter_mut().for_each(|v| *v /= area);
    blur_exact(&mut small, sw, sh, sigma / factor as f32);
    // Back to full size, each value between the four reduced ones
    // around it.
    let at = |x: i64, y: i64| {
        if x < 0 || y < 0 || x >= sw as i64 || y >= sh as i64 {
            0.0
        } else {
            small[y as usize * sw + x as usize]
        }
    };
    for y in 0..h {
        let fy = (y as f32 + 0.5) / factor as f32 - 0.5;
        let (y0, ty) = (fy.floor(), fy - fy.floor());
        for x in 0..w {
            let fx = (x as f32 + 0.5) / factor as f32 - 0.5;
            let (x0, tx) = (fx.floor(), fx - fx.floor());
            let (x0, y0) = (x0 as i64, y0 as i64);
            plane[y * w + x] = at(x0, y0) * (1.0 - tx) * (1.0 - ty)
                + at(x0 + 1, y0) * tx * (1.0 - ty)
                + at(x0, y0 + 1) * (1.0 - tx) * ty
                + at(x0 + 1, y0 + 1) * tx * ty;
        }
    }
}

/// The Gaussian itself: rows, then columns, with the kernel cut at three
/// deviations.
fn blur_exact(plane: &mut [f32], w: usize, h: usize, sigma: f32) {
    let radius = (sigma * 3.0).ceil().max(1.0) as i64;
    let mut kernel: Vec<f32> = (-radius..=radius)
        .map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp())
        .collect();
    let total: f32 = kernel.iter().sum();
    kernel.iter_mut().for_each(|k| *k /= total);
    let mut row = vec![0.0f32; w.max(h)];
    for y in 0..h {
        let line = &mut plane[y * w..(y + 1) * w];
        for (x, out) in row[..w].iter_mut().enumerate() {
            let mut sum = 0.0;
            for (i, k) in kernel.iter().enumerate() {
                let sx = x as i64 + i as i64 - radius;
                if sx >= 0 && sx < w as i64 {
                    sum += line[sx as usize] * k;
                }
            }
            *out = sum;
        }
        line.copy_from_slice(&row[..w]);
    }
    for x in 0..w {
        for (y, out) in row[..h].iter_mut().enumerate() {
            let mut sum = 0.0;
            for (i, k) in kernel.iter().enumerate() {
                let sy = y as i64 + i as i64 - radius;
                if sy >= 0 && sy < h as i64 {
                    sum += plane[sy as usize * w + x] * k;
                }
            }
            *out = sum;
        }
        for y in 0..h {
            plane[y * w + x] = row[y];
        }
    }
}

fn blur_layer(layer: &mut Layer, sigma: f32) {
    if sigma < 0.3 {
        return;
    }
    for c in 0..4 {
        let mut plane: Vec<f32> = layer.px.iter().map(|p| p[c]).collect();
        gaussian(&mut plane, layer.w, layer.h, sigma);
        for (p, v) in layer.px.iter_mut().zip(plane) {
            p[c] = v;
        }
    }
}

// ---- distances ---------------------------------------------------------------

/// For each pixel, how far the nearest pixel of the set is (between
/// pixel centres; 0 in the set). An exact Euclidean distance, by rows
/// then by columns.
pub fn distance_to(set: &[bool], w: usize, h: usize) -> Vec<f32> {
    const FAR: f64 = 1e12;
    let n = w.max(h);
    let (mut v, mut z) = (vec![0usize; n], vec![0.0f64; n + 1]);
    let (mut f, mut d) = (vec![0.0f64; n], vec![0.0f64; n]);
    // The lower envelope of the parabolas standing on `f`.
    let mut pass = |f: &[f64], d: &mut [f64]| {
        let n = f.len();
        let mut k = 0;
        v[0] = 0;
        z[0] = f64::NEG_INFINITY;
        z[1] = f64::INFINITY;
        for q in 1..n {
            loop {
                let p = v[k];
                let s = ((f[q] + (q * q) as f64) - (f[p] + (p * p) as f64))
                    / (2.0 * q as f64 - 2.0 * p as f64);
                if s <= z[k] && k > 0 {
                    k -= 1;
                } else {
                    k += 1;
                    v[k] = q;
                    z[k] = s;
                    z[k + 1] = f64::INFINITY;
                    break;
                }
            }
        }
        k = 0;
        for (q, out) in d.iter_mut().enumerate().take(n) {
            while z[k + 1] < q as f64 {
                k += 1;
            }
            let p = v[k];
            *out = ((q as f64 - p as f64) * (q as f64 - p as f64)) + f[p];
        }
    };
    let mut grid: Vec<f64> = set
        .iter()
        .map(|inside| if *inside { 0.0 } else { FAR })
        .collect();
    for x in 0..w {
        for y in 0..h {
            f[y] = grid[y * w + x];
        }
        pass(&f[..h], &mut d[..h]);
        for y in 0..h {
            grid[y * w + x] = d[y];
        }
    }
    for y in 0..h {
        f[..w].copy_from_slice(&grid[y * w..(y + 1) * w]);
        pass(&f[..w], &mut d[..w]);
        grid[y * w..(y + 1) * w].copy_from_slice(&d[..w]);
    }
    grid.into_iter()
        .map(|squared| squared.min(FAR).sqrt() as f32)
        .collect()
}

/// How far inside the outline each pixel is (negative outside): the
/// distance to the edge, which runs between pixels.
fn signed_distance(inside: &[bool], w: usize, h: usize) -> Vec<f32> {
    let outside: Vec<bool> = inside.iter().map(|v| !v).collect();
    let to_outside = distance_to(&outside, w, h);
    let to_inside = distance_to(inside, w, h);
    inside
        .iter()
        .enumerate()
        .map(|(i, is_in)| {
            if *is_in {
                to_outside[i] - 0.5
            } else {
                0.5 - to_inside[i]
            }
        })
        .collect()
}

// ---- the parts of a dress ----------------------------------------------------

/// Light the edges of the object as a bevel cut into them: `depth` is
/// how far inside the outer edge each pixel is. The edge rises along the
/// profile of the bevel; each pixel is lit by how its slope faces a
/// light above and to the left of the picture, with a highlight where it
/// mirrors that light. The flat face is left as it is.
fn shade(layer: &mut Layer, depth: &[f32], bevel: &Bevel, k: f32) {
    let width = (bevel.width * 4.0 / 3.0 * k).max(1.0);
    let height = bevel.height * 4.0 / 3.0 * k;
    if height <= 0.0 {
        return;
    }
    let (w, h) = (layer.w, layer.h);
    // The distances of a mask of pixels step by whole pixels along a
    // slanted or round edge: smoothed, the slopes do not show the steps.
    let mut depth = depth.to_vec();
    gaussian(&mut depth, w, h, 1.0);
    let rise = |d: f32| height * bevel.preset.profile(d / width);
    let norm = |v: [f32; 3]| {
        let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
        [v[0] / length, v[1] / length, v[2] / length]
    };
    let light = norm([-0.45, -0.65, 0.62]);
    let half = norm([light[0], light[1], light[2] + 1.0]);
    let flat_shine = half[2].powi(24);
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            let i = y * w + x;
            let d = depth[i];
            if layer.px[i][3] <= 0.0 || d > width + 1.0 {
                continue;
            }
            let slope = rise(d + 0.5) - rise(d - 0.5);
            let gx = (depth[i + 1] - depth[i - 1]) / 2.0;
            let gy = (depth[i + w] - depth[i - w]) / 2.0;
            let normal = norm([-slope * gx, -slope * gy, 1.0]);
            let facing = normal[0] * light[0] + normal[1] * light[1] + normal[2] * light[2];
            let lit = (1.0 + 0.85 * (facing - light[2]) / light[2]).clamp(0.25, 1.9);
            let mirror = (normal[0] * half[0] + normal[1] * half[1] + normal[2] * half[2]).max(0.0);
            let shine = 0.5 * (mirror.powi(24) - flat_shine);
            let p = &mut layer.px[i];
            for c in 0..3 {
                p[c] = (p[c] * lit + shine * p[3]).clamp(0.0, p[3]);
            }
        }
    }
}

/// The eight numbers of the projection that takes the corners `from` to
/// the corners `to`.
fn homography(from: [(f64, f64); 4], to: [(f64, f64); 4]) -> Option<[f64; 8]> {
    let mut m = [[0.0f64; 9]; 8];
    for i in 0..4 {
        let ((x, y), (u, v)) = (from[i], to[i]);
        m[2 * i] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
        m[2 * i + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
    }
    for col in 0..8 {
        let pivot = (col..8).max_by(|a, b| m[*a][col].abs().total_cmp(&m[*b][col].abs()))?;
        if m[pivot][col].abs() < 1e-10 {
            return None;
        }
        m.swap(col, pivot);
        for row in 0..8 {
            if row != col {
                let factor = m[row][col] / m[col][col];
                for c in col..9 {
                    m[row][c] -= factor * m[col][c];
                }
            }
        }
    }
    Some(std::array::from_fn(|i| m[i][8] / m[i][i]))
}

/// The object as a camera sees it once it is turned in space: its four
/// corners are projected, and every pixel of the result looks up where
/// it comes from.
fn warp(layer: &Layer, frame: (usize, usize), rotation: &Rotation3d) -> Layer {
    let (fw, fh) = (frame.0 as f32, frame.1 as f32);
    let corners = [
        (0.0, 0.0),
        (layer.w as f64, 0.0),
        (layer.w as f64, layer.h as f64),
        (0.0, layer.h as f64),
    ];
    let seen = corners.map(|(x, y)| {
        let (u, v) = (
            (x as f32 + layer.ox as f32) / fw,
            (y as f32 + layer.oy as f32) / fh,
        );
        let (px, py, _) = rotation.project(fw, fh, u, v);
        ((px + fw / 2.0) as f64, (py + fh / 2.0) as f64)
    });
    // Never more than a few times the frame, whatever the camera.
    let reach = 3.0 * fw.max(fh) as f64;
    let clamp = |v: f64, middle: f64| v.clamp(middle - reach, middle + reach);
    let x0 = seen
        .iter()
        .map(|p| clamp(p.0, fw as f64 / 2.0))
        .fold(f64::INFINITY, f64::min)
        .floor();
    let y0 = seen
        .iter()
        .map(|p| clamp(p.1, fh as f64 / 2.0))
        .fold(f64::INFINITY, f64::min)
        .floor();
    let x1 = seen
        .iter()
        .map(|p| clamp(p.0, fw as f64 / 2.0))
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil();
    let y1 = seen
        .iter()
        .map(|p| clamp(p.1, fh as f64 / 2.0))
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil();
    let Some(back) = homography(seen, corners) else {
        return layer.clone();
    };
    let mut out = Layer::new(
        (x1 - x0).max(1.0) as usize,
        (y1 - y0).max(1.0) as usize,
        x0 as i32,
        y0 as i32,
    );
    for y in 0..out.h {
        for x in 0..out.w {
            let (px, py) = (x as f64 + 0.5 + x0, y as f64 + 0.5 + y0);
            let depth = back[6] * px + back[7] * py + 1.0;
            if depth.abs() < 1e-9 {
                continue;
            }
            let sx = (back[0] * px + back[1] * py + back[2]) / depth;
            let sy = (back[3] * px + back[4] * py + back[5]) / depth;
            if sx < -1.0 || sy < -1.0 || sx > layer.w as f64 + 1.0 || sy > layer.h as f64 + 1.0 {
                continue;
            }
            out.px[y * out.w + x] = layer.sample(sx as f32 - 0.5, sy as f32 - 0.5);
        }
    }
    out
}

/// The shadow the object casts: its outline scaled, squashed and leant
/// as the shadow says, moved to where it falls, and blurred.
fn shadow_layer(object: &Layer, shadow: &Shadow, turn: f32, k: f32) -> Option<Layer> {
    let (bx0, by0, bx1, by1) = object.inked()?;
    let px = |points: f32| points * 4.0 / 3.0 * k;
    let sigma = px(shadow.blur) / 2.0;
    let (sin, cos) = (shadow.angle - turn).to_radians().sin_cos();
    let (dx, dy) = (px(shadow.distance) * cos, px(shadow.distance) * sin);
    let (sx, sy) = (
        shadow.size.max(0.01),
        shadow.size.max(0.01)
            * if shadow.squash == 0.0 {
                1.0
            } else {
                shadow.squash
            },
    );
    let lean = shadow.skew.to_radians().tan();
    // What stays where it is when the shadow is scaled: the middle of
    // the object, or its foot for a shadow lying on the ground.
    let anchor = if shadow.lies() {
        let ax = if shadow.skew > 0.0 {
            bx1 as f32
        } else if shadow.skew < 0.0 {
            bx0 as f32
        } else {
            (bx0 + bx1) as f32 / 2.0
        };
        (ax, by1 as f32)
    } else {
        ((bx0 + bx1) as f32 / 2.0, (by0 + by1) as f32 / 2.0)
    };
    let forward = |x: f32, y: f32| {
        let (rx, ry) = (x - anchor.0, (y - anchor.1) * sy);
        (anchor.0 + rx * sx + lean * ry + dx, anchor.1 + ry + dy)
    };
    let corners = [
        forward(bx0 as f32, by0 as f32),
        forward(bx1 as f32, by0 as f32),
        forward(bx1 as f32, by1 as f32),
        forward(bx0 as f32, by1 as f32),
    ];
    let room = (sigma * 3.0).ceil() + 1.0;
    let x0 = (corners.iter().map(|p| p.0).fold(f32::INFINITY, f32::min) - room).floor();
    let y0 = (corners.iter().map(|p| p.1).fold(f32::INFINITY, f32::min) - room).floor();
    let x1 = (corners
        .iter()
        .map(|p| p.0)
        .fold(f32::NEG_INFINITY, f32::max)
        + room)
        .ceil();
    let y1 = (corners
        .iter()
        .map(|p| p.1)
        .fold(f32::NEG_INFINITY, f32::max)
        + room)
        .ceil();
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    if w == 0 || h == 0 || w * h > 4 * MAX_AREA {
        return None;
    }
    let mut mask = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            // Back from the shadow to the object.
            let (px, py) = (x as f32 + 0.5 + x0 - dx, y as f32 + 0.5 + y0 - dy);
            let ry = py - anchor.1;
            let rx = (px - anchor.0 - lean * ry) / sx;
            let (ox, oy) = (anchor.0 + rx, anchor.1 + ry / sy);
            mask[y * w + x] =
                object.sample(ox - object.ox as f32 - 0.5, oy - object.oy as f32 - 0.5)[3];
        }
    }
    gaussian(&mut mask, w, h, sigma);
    Some(Layer::tinted(
        w,
        h,
        x0 as i32,
        y0 as i32,
        &mask,
        shadow.color,
        1.0 - shadow.transparency.clamp(0.0, 1.0),
    ))
}

/// A halo around the object: full at its edge, gone `radius` away.
fn glow_layer(object: &Layer, glow: &Glow, k: f32) -> Option<Layer> {
    let radius = glow.radius * 4.0 / 3.0 * k;
    if radius <= 0.0 {
        return None;
    }
    let pad = radius.ceil() as usize + 2;
    let (w, h) = (object.w + 2 * pad, object.h + 2 * pad);
    let mut solid = vec![false; w * h];
    for y in 0..object.h {
        for x in 0..object.w {
            solid[(y + pad) * w + x + pad] = object.px[y * object.w + x][3] > 0.5;
        }
    }
    let mut mask: Vec<f32> = distance_to(&solid, w, h)
        .into_iter()
        .map(|d| {
            let t = (1.0 - d / radius).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        })
        .collect();
    gaussian(&mut mask, w, h, (radius / 6.0).min(4.0));
    Some(Layer::tinted(
        w,
        h,
        object.ox - pad as i32,
        object.oy - pad as i32,
        &mask,
        glow.color,
        1.0 - glow.transparency.clamp(0.0, 1.0),
    ))
}

/// The object seen again under itself, upside down: the rows nearest
/// its foot first, fading out over the part of it that is reflected.
fn reflection_layer(object: &Layer, reflection: &Reflection, k: f32) -> Option<Layer> {
    let (_, by0, _, by1) = object.inked()?;
    let px = |points: f32| points * 4.0 / 3.0 * k;
    let reach = ((by1 - by0) as f32 * reflection.size.clamp(0.0, 1.0)).round() as usize;
    if reach == 0 {
        return None;
    }
    let sigma = px(reflection.blur) / 2.0;
    let room = (sigma * 3.0).ceil() as usize;
    let gap = px(reflection.distance).round() as i32;
    let strength = 1.0 - reflection.transparency.clamp(0.0, 1.0);
    let mut out = Layer::new(
        object.w + 2 * room,
        reach + 2 * room,
        object.ox - room as i32,
        by1 + gap - room as i32,
    );
    for row in 0..reach {
        // The row of the object this far above its foot.
        let source = by1 - 1 - row as i32 - object.oy;
        if source < 0 || source >= object.h as i32 {
            continue;
        }
        let fade = strength * (1.0 - row as f32 / reach as f32);
        for x in 0..object.w {
            let p = object.px[source as usize * object.w + x];
            out.px[(row + room) * out.w + x + room] =
                [p[0] * fade, p[1] * fade, p[2] * fade, p[3] * fade];
        }
    }
    blur_layer(&mut out, sigma);
    Some(out)
}

/// The shadow and the glow of something that is not made of pixels (a
/// shape): `cover` says how much of each pixel of a canvas `w` by `h`
/// it covers, 0 to 1. The result holds the shadow and the glow alone,
/// to be painted under the shape, centred on the canvas, which takes
/// the returned share of its width and height. `None` when there is
/// nothing to paint.
pub fn under(
    cover: &[f32],
    w: usize,
    h: usize,
    effects: &Effects,
    turn: f32,
    k: f32,
) -> Option<(Bitmap, (f32, f32))> {
    let object = Layer {
        w,
        h,
        ox: 0,
        oy: 0,
        px: cover
            .iter()
            .map(|a| [0.0, 0.0, 0.0, a.clamp(0.0, 1.0)])
            .collect(),
    };
    let mut layers: Vec<Layer> = Vec::new();
    layers.extend(
        effects
            .shadow
            .and_then(|shadow| shadow_layer(&object, &shadow, turn, k)),
    );
    layers.extend(effects.glow.and_then(|glow| glow_layer(&object, &glow, k)));
    if layers.is_empty() {
        return None;
    }
    let (mut pad_x, mut pad_y) = (0i32, 0i32);
    for layer in &layers {
        pad_x = pad_x
            .max(-layer.ox)
            .max(layer.ox + layer.w as i32 - w as i32);
        pad_y = pad_y
            .max(-layer.oy)
            .max(layer.oy + layer.h as i32 - h as i32);
    }
    let mut all = Layer::new(
        w + 2 * pad_x as usize,
        h + 2 * pad_y as usize,
        -pad_x,
        -pad_y,
    );
    for layer in &layers {
        all.over(layer);
    }
    let share = (w as f32 / all.w as f32, h as f32 / all.h as f32);
    Some((all.into_bitmap(), share))
}

/// How much of each pixel of a canvas `w` by `h` outlines cover: the
/// inside of the closed ones when `filled`, and a line `stroke` pixels
/// wide along all of them.
pub fn cover_of(
    lines: &[(Vec<(f32, f32)>, bool)],
    w: usize,
    h: usize,
    filled: bool,
    stroke: f32,
) -> Vec<f32> {
    let mut cover = vec![0.0f32; w * h];
    if filled {
        let mut crossings: Vec<f32> = Vec::new();
        for y in 0..h {
            let level = y as f32 + 0.5;
            crossings.clear();
            for (points, closed) in lines {
                if !closed || points.len() < 3 {
                    continue;
                }
                for i in 0..points.len() {
                    let (a, b) = (points[i], points[(i + 1) % points.len()]);
                    if (a.1 <= level) != (b.1 <= level) {
                        crossings.push(a.0 + (level - a.1) / (b.1 - a.1) * (b.0 - a.0));
                    }
                }
            }
            crossings.sort_by(f32::total_cmp);
            for pair in crossings.chunks_exact(2) {
                let (from, to) = (
                    (pair[0] - 0.5).ceil().max(0.0) as usize,
                    ((pair[1] - 0.5).floor() as i64).min(w as i64 - 1),
                );
                for x in from as i64..=to {
                    cover[y * w + x as usize] = 1.0;
                }
            }
        }
    }
    if stroke > 0.0 {
        let half = stroke / 2.0;
        for (points, closed) in lines {
            let count = if *closed {
                points.len()
            } else {
                points.len().saturating_sub(1)
            };
            for i in 0..count {
                let (a, b) = (points[i], points[(i + 1) % points.len()]);
                let reach = half + 1.0;
                let (x0, x1) = (
                    (a.0.min(b.0) - reach).floor().max(0.0) as usize,
                    ((a.0.max(b.0) + reach).ceil() as i64).min(w as i64 - 1),
                );
                let (y0, y1) = (
                    (a.1.min(b.1) - reach).floor().max(0.0) as usize,
                    ((a.1.max(b.1) + reach).ceil() as i64).min(h as i64 - 1),
                );
                let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                let length = (dx * dx + dy * dy).max(1e-9);
                for y in y0 as i64..=y1 {
                    for x in x0 as i64..=x1 {
                        let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                        let along = (((px - a.0) * dx + (py - a.1) * dy) / length).clamp(0.0, 1.0);
                        let distance = ((px - a.0 - along * dx).powi(2)
                            + (py - a.1 - along * dy).powi(2))
                        .sqrt();
                        let at = y as usize * w + x as usize;
                        cover[at] = cover[at].max((half - distance + 0.5).clamp(0.0, 1.0));
                    }
                }
            }
        }
    }
    cover
}

/// The picture dressed. `body` is the picture as its frame shows it
/// (cropped, adjusted, cut to its outline, flipped); `mask` says which of
/// its pixels are inside that outline (all of them when not given); `k`
/// is how many of its pixels make a pixel of the sheet at 100 %. The
/// result is centred on the frame; `frame_share` of the returned pair
/// says how much of its width and height the frame takes.
pub fn compose(
    body: &Bitmap,
    mask: Option<&[bool]>,
    dress: &Dress,
    k: f32,
) -> (Bitmap, (f32, f32)) {
    // Too many pixels to dress: a smaller copy is.
    if body.width as usize * body.height as usize > MAX_AREA && body.width > 2 && body.height > 2 {
        let half = body.halved();
        let small_mask = mask.map(|mask| {
            let (w, h) = (half.width as usize, half.height as usize);
            (0..w * h)
                .map(|i| {
                    mask[((i / w * 2).min(body.height as usize - 1)) * body.width as usize
                        + (i % w * 2).min(body.width as usize - 1)]
                })
                .collect::<Vec<bool>>()
        });
        return compose(&half, small_mask.as_deref(), dress, k / 2.0);
    }
    let (w, h) = (body.width as usize, body.height as usize);
    let px = |points: f32| points * 4.0 / 3.0 * k;
    let border = dress
        .outline
        .map_or(0.0, |outline| px(outline.width).max(1.0));
    let edged = dress.outline.is_some() || dress.effects.bevel.is_some();
    let pad = if edged {
        (border / 2.0).ceil() as usize + 1
    } else {
        0
    };
    let mut object = Layer::new(w + 2 * pad, h + 2 * pad, -(pad as i32), -(pad as i32));
    for y in 0..h {
        for x in 0..w {
            let at = (y * w + x) * 4;
            let a = body.data[at + 3] as f32 / 255.0;
            object.px[(y + pad) * object.w + x + pad] = [
                body.data[at] as f32 / 255.0 * a,
                body.data[at + 1] as f32 / 255.0 * a,
                body.data[at + 2] as f32 / 255.0 * a,
                a,
            ];
        }
    }
    if edged {
        let mut inside = vec![false; object.w * object.h];
        for y in 0..h {
            for x in 0..w {
                inside[(y + pad) * object.w + x + pad] = mask.is_none_or(|mask| mask[y * w + x]);
            }
        }
        let mut depth = signed_distance(&inside, object.w, object.h);
        if let Some(outline) = &dress.outline {
            // The line lies astride the edge, half inside, half outside.
            let value = outline.color.value();
            let ink = [
                (value & 0xFF) as f32 / 255.0,
                ((value >> 8) & 0xFF) as f32 / 255.0,
                ((value >> 16) & 0xFF) as f32 / 255.0,
            ];
            for (p, d) in object.px.iter_mut().zip(&depth) {
                let cover = (border / 2.0 - d.abs() + 0.5).clamp(0.0, 1.0);
                if cover > 0.0 {
                    for c in 0..3 {
                        p[c] = ink[c] * cover + p[c] * (1.0 - cover);
                    }
                    p[3] = cover + p[3] * (1.0 - cover);
                }
            }
            depth.iter_mut().for_each(|d| *d += border / 2.0);
        }
        if let Some(bevel) = &dress.effects.bevel {
            shade(&mut object, &depth, bevel, k);
        }
    }
    if let Some(rotation) = dress
        .effects
        .rotation
        .filter(|rotation| !rotation.is_flat() || rotation.perspective > 0.0)
    {
        object = warp(&object, (w, h), &rotation);
    }
    let mut under: Vec<Layer> = Vec::new();
    under.extend(
        dress
            .effects
            .shadow
            .and_then(|shadow| shadow_layer(&object, &shadow, dress.turn, k)),
    );
    under.extend(
        dress
            .effects
            .glow
            .and_then(|glow| glow_layer(&object, &glow, k)),
    );
    under.extend(
        dress
            .effects
            .reflection
            .and_then(|reflection| reflection_layer(&object, &reflection, k)),
    );
    // Room for everything, as much on one side of the frame as on the
    // other: the result turns about the middle of the frame.
    let (mut pad_x, mut pad_y) = (0i32, 0i32);
    for layer in under.iter().chain([&object]) {
        pad_x = pad_x
            .max(-layer.ox)
            .max(layer.ox + layer.w as i32 - w as i32);
        pad_y = pad_y
            .max(-layer.oy)
            .max(layer.oy + layer.h as i32 - h as i32);
    }
    let mut all = Layer::new(
        w + 2 * pad_x as usize,
        h + 2 * pad_y as usize,
        -pad_x,
        -pad_y,
    );
    for layer in under.iter().chain([&object]) {
        all.over(layer);
    }
    let share = (w as f32 / all.w as f32, h as f32 / all.h as f32);
    (all.into_bitmap(), share)
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::picture_style::BevelPreset;

    fn grey(width: u32, height: u32) -> Bitmap {
        Bitmap {
            width,
            height,
            data: [128, 128, 128, 255].repeat((width * height) as usize),
        }
    }

    fn plain() -> Dress {
        Dress {
            outline: None,
            effects: Effects::default(),
            turn: 0.0,
        }
    }

    fn pixel(bitmap: &Bitmap, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * bitmap.width + x) * 4) as usize;
        [
            bitmap.data[at],
            bitmap.data[at + 1],
            bitmap.data[at + 2],
            bitmap.data[at + 3],
        ]
    }

    #[test]
    fn distances_are_euclidean() {
        // One pixel of the set in the middle of a 7 by 5 grid.
        let mut set = vec![false; 35];
        set[2 * 7 + 3] = true;
        let d = distance_to(&set, 7, 5);
        assert_eq!(d[2 * 7 + 3], 0.0);
        assert_eq!(d[2 * 7], 3.0);
        assert!((d[0] - (9.0f32 + 4.0).sqrt()).abs() < 1e-4);
        // Nothing in the set: far everywhere, and no panic.
        assert!(distance_to(&[false; 6], 3, 2).iter().all(|d| *d > 1000.0));
        // Inside a 4 by 4 block the edge is half a pixel from the rim.
        let inside: Vec<bool> = (0..36)
            .map(|i| (1..5).contains(&(i % 6)) && (1..5).contains(&(i / 6)))
            .collect();
        let signed = signed_distance(&inside, 6, 6);
        assert_eq!(signed[6 + 1], 0.5);
        assert_eq!(signed[2 * 6 + 2], 1.5);
        assert_eq!(signed[0], 0.5 - 2.0f32.sqrt());
    }

    #[test]
    fn a_gaussian_keeps_the_sum_and_spreads_it() {
        for sigma in [1.5f32, 12.0] {
            let (w, h) = (120usize, 90usize);
            let mut plane = vec![0.0f32; w * h];
            for y in 40..50 {
                for x in 55..65 {
                    plane[y * w + x] = 1.0;
                }
            }
            gaussian(&mut plane, w, h, sigma);
            let sum: f32 = plane.iter().sum();
            assert!((sum - 100.0).abs() < 2.0, "{sigma}: {sum}");
            // Symmetric about the block, and falling off as a Gaussian
            // does: at one deviation from the edge of a wide block a
            // step is down to about 16 %.
            assert!((plane[45 * w + 50] - plane[45 * w + 69]).abs() < 0.02);
            assert!(plane[45 * w + 60] > plane[45 * w + 70]);
            assert!(plane[45 * w + 70] >= plane[45 * w + 80]);
        }
        // A step blurred with deviation 2: the edge runs between two
        // pixels, so the first pixel inside is a quarter of a deviation
        // in (60 %) and the third one and a quarter (89 %).
        let (w, h) = (80usize, 40usize);
        let mut step: Vec<f32> = (0..w * h)
            .map(|i| if i % w >= 40 { 1.0 } else { 0.0 })
            .collect();
        gaussian(&mut step, w, h, 2.0);
        assert!(
            (step[20 * w + 40] - 0.5987).abs() < 0.01,
            "{}",
            step[20 * w + 40]
        );
        assert!(
            (step[20 * w + 42] - 0.8944).abs() < 0.01,
            "{}",
            step[20 * w + 42]
        );
        assert!(
            (step[20 * w + 37] - 0.1056).abs() < 0.01,
            "{}",
            step[20 * w + 37]
        );
    }

    #[test]
    fn a_plain_dress_gives_the_picture_back() {
        let body = grey(20, 10);
        let (out, share) = compose(&body, None, &plain(), 1.0);
        assert_eq!(out, body);
        assert_eq!(share, (1.0, 1.0));
    }

    #[test]
    fn a_border_lies_astride_the_edge() {
        let mut dress = plain();
        // 6 points are 8 pixels: 4 outside, 4 inside.
        dress.outline = Some(Outline::solid(Color(0xFF0000), 6.0));
        let (out, share) = compose(&grey(40, 30), None, &dress, 1.0);
        assert_eq!((out.width, out.height), (50, 40));
        assert!((share.0 - 0.8).abs() < 1e-6);
        // Blue first: red is the third byte.
        assert_eq!(pixel(&out, 1, 20), [0, 0, 255, 255]);
        assert_eq!(pixel(&out, 8, 20), [0, 0, 255, 255]);
        assert_eq!(pixel(&out, 10, 20), [128, 128, 128, 255]);
        assert_eq!(pixel(&out, 0, 20)[3], 0);
    }

    #[test]
    fn a_shadow_falls_where_it_is_told_and_fades() {
        let mut dress = plain();
        dress.effects.shadow = Some(Shadow {
            color: Color(0x000000),
            transparency: 0.0,
            size: 1.0,
            blur: 6.0,
            angle: 0.0,
            distance: 15.0,
            ..Shadow::default()
        });
        let (out, _) = compose(&grey(40, 40), None, &dress, 1.0);
        // To the right: 20 pixels away, blurred with a deviation of 4.
        let left = (out.width - 40) / 2;
        let middle = out.height / 2;
        let right_edge = left + 40;
        assert_eq!(pixel(&out, left + 20, middle), [128, 128, 128, 255]);
        let near = pixel(&out, right_edge + 5, middle);
        assert_eq!(&near[..3], &[0, 0, 0]);
        assert!(near[3] > 240, "{near:?}");
        // At the shadow's own edge half of it is left; beyond, less.
        let at_edge = pixel(&out, right_edge + 20, middle)[3];
        assert!((110..=145).contains(&at_edge), "{at_edge}");
        assert!(pixel(&out, right_edge + 26, middle)[3] < 40);
        // Nothing on the other side.
        assert_eq!(pixel(&out, 2, middle)[3], 0);
        // The picture turned a quarter: the shadow still falls to the
        // right of the sheet, which is above the picture as it lies.
        dress.turn = 90.0;
        let (turned, _) = compose(&grey(40, 40), None, &dress, 1.0);
        let (cx, top) = (turned.width / 2, (turned.height - 40) / 2);
        assert!(pixel(&turned, cx, top - 5)[3] > 240);
    }

    #[test]
    fn a_reflection_mirrors_the_foot_of_the_picture() {
        // White above, black below.
        let mut body = grey(20, 20);
        for y in 0..20u32 {
            for x in 0..20u32 {
                let at = ((y * 20 + x) * 4) as usize;
                let level = if y < 10 { 255 } else { 0 };
                body.data[at..at + 3].copy_from_slice(&[level; 3]);
            }
        }
        let mut dress = plain();
        dress.effects.reflection = Some(Reflection {
            transparency: 0.5,
            size: 0.5,
            distance: 0.0,
            blur: 0.0,
        });
        let (out, _) = compose(&body, None, &dress, 1.0);
        let top = (out.height - 20) / 2;
        assert_eq!(top, 10);
        // Right under the picture: its last row, at half strength.
        let first = pixel(&out, 10, top + 20);
        assert_eq!(&first[..3], &[0, 0, 0]);
        assert!((120..=130).contains(&first[3]));
        // Fainter further down, and gone after half the height.
        assert!(pixel(&out, 10, top + 26)[3] < first[3]);
        assert!(pixel(&out, 10, top + 29)[3] > 0);
        assert_eq!(out.height, 40);
        assert_eq!(pixel(&out, 10, 0)[3], 0);
    }

    #[test]
    fn a_bevel_lights_the_edges_and_leaves_the_face() {
        let mut dress = plain();
        dress.effects.bevel = Some(Bevel {
            preset: BevelPreset::Angle,
            width: 6.0,
            height: 6.0,
        });
        let (out, _) = compose(&grey(60, 60), None, &dress, 1.0);
        let off = (out.width - 60) / 2;
        // The face keeps its colour; the edge towards the light is
        // lighter, the one away from it darker.
        assert_eq!(pixel(&out, off + 30, off + 30), [128, 128, 128, 255]);
        let (top, bottom) = (
            pixel(&out, off + 30, off + 3)[0],
            pixel(&out, off + 30, off + 56)[0],
        );
        assert!(top > 150 && bottom < 105, "{top} {bottom}");
        let (lit, dark) = (
            pixel(&out, off + 3, off + 30)[0],
            pixel(&out, off + 56, off + 30)[0],
        );
        assert!(lit > 135 && dark < 120, "{lit} {dark}");
    }

    #[test]
    fn a_turn_in_space_projects_the_corners() {
        let mut dress = plain();
        // 60 degrees about the vertical axis, seen in parallel: half as
        // wide, as tall.
        dress.effects.rotation = Some(Rotation3d {
            x: 60.0,
            y: 0.0,
            z: 0.0,
            perspective: 0.0,
        });
        let (out, _) = compose(&grey(80, 40), None, &dress, 1.0);
        assert_eq!((out.width, out.height), (80, 40));
        assert_eq!(pixel(&out, 40, 20), [128, 128, 128, 255]);
        assert_eq!(pixel(&out, 25, 20)[3], 255);
        assert_eq!(pixel(&out, 15, 20)[3], 0);
        // In perspective the near side is taller than the far side.
        dress.effects.rotation = Some(Rotation3d {
            x: 50.0,
            y: 0.0,
            z: 0.0,
            perspective: 60.0,
        });
        let (seen, _) = compose(&grey(80, 80), None, &dress, 1.0);
        let column = |x: u32| {
            (0..seen.height)
                .filter(|y| pixel(&seen, x, *y)[3] > 128)
                .count()
        };
        let inked: Vec<u32> = (0..seen.width).filter(|x| column(*x) > 0).collect();
        let (first, last) = (inked[1], inked[inked.len() - 2]);
        assert!(
            column(first).abs_diff(column(last)) > 8,
            "{} {}",
            column(first),
            column(last)
        );
        // A homography that maps a square onto itself changes nothing.
        let square = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        let same = homography(square, square).unwrap();
        assert!((same[0] - 1.0).abs() < 1e-9 && same[1].abs() < 1e-9 && same[6].abs() < 1e-9);
    }

    #[test]
    fn a_shape_casts_a_blurred_shadow_of_its_outline() {
        // A filled triangle, and the same as a line only.
        let triangle = vec![(vec![(10.0, 50.0), (30.0, 10.0), (50.0, 50.0)], true)];
        let filled = cover_of(&triangle, 60, 60, true, 0.0);
        assert_eq!(
            (
                filled[40 * 60 + 30],
                filled[15 * 60 + 12],
                filled[55 * 60 + 30]
            ),
            (1.0, 0.0, 0.0)
        );
        let hollow = cover_of(&triangle, 60, 60, false, 3.0);
        assert_eq!(hollow[40 * 60 + 30], 0.0);
        assert!(hollow[49 * 60 + 30] > 0.9 && hollow[30 * 60 + 20] > 0.5);
        let mut effects = Effects::default();
        assert!(under(&filled, 60, 60, &effects, 0.0, 1.0).is_none());
        effects.shadow = Some(Shadow {
            color: Color(0x0000FF),
            transparency: 0.5,
            blur: 3.0,
            angle: 90.0,
            distance: 6.0,
            ..Shadow::default()
        });
        let (shadow, share) = under(&filled, 60, 60, &effects, 0.0, 1.0).unwrap();
        // Below the triangle (8 pixels down), in the shadow's colour at
        // half strength; the triangle itself is not in the bitmap.
        let (off_x, off_y) = ((shadow.width - 60) / 2, (shadow.height - 60) / 2);
        assert!(share.1 < 1.0);
        let inside = pixel(&shadow, off_x + 30, off_y + 48);
        assert_eq!(&inside[..3], &[255, 0, 0]);
        assert!((120..=130).contains(&inside[3]), "{inside:?}");
        assert!(pixel(&shadow, off_x + 30, off_y + 56)[3] > 60);
        assert_eq!(pixel(&shadow, off_x + 30, off_y + 12)[3], 0);
        // The edge of the shadow is soft: neither nothing nor all.
        let edge = pixel(&shadow, off_x + 30, off_y + 58)[3];
        assert!(edge > 20 && edge < 110, "{edge}");
    }

    #[test]
    fn dresses_are_told_apart() {
        let mut dress = plain();
        assert!(dress.is_plain());
        let bare = dress.bits();
        dress.effects.glow = Some(Glow {
            color: Color(0x00FF00),
            transparency: 0.4,
            radius: 5.0,
        });
        assert!(!dress.is_plain());
        assert_ne!(dress.bits(), bare);
        // A glow reaches its radius and no further.
        let (out, _) = compose(&grey(30, 30), None, &dress, 1.0);
        let off = (out.width - 30) / 2;
        let near = pixel(&out, off - 2, out.height / 2);
        assert_eq!(&near[..3], &[0, 255, 0]);
        assert!(near[3] > 40);
        assert_eq!(pixel(&out, 0, out.height / 2)[3], 0);
        assert!(off >= 7);
    }
}
