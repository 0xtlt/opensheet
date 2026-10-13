//! Pictures and shapes floating over the cells (`Sheet::drawings`, in
//! drawing order: the last one is on top).
//!
//! A drawing is placed like a chart: a cell holding its top-left corner,
//! an offset inside that cell and a size, in pixels at 100 %. It also
//! says how it follows the cells ([`Placement`]); one that is sized with
//! the cells is pinned to a second cell under its bottom-right corner, as
//! in a file. Where a drawing is on the sheet is therefore always asked of
//! `Sheet::drawing_frame`, which knows the column widths and row heights.
//!
//! Shapes are described, not drawn: [`ShapePreset::path`] gives the
//! outline of each preset as a few path commands that a screen and a PDF
//! writer can both follow.

use crate::ast::StructuralEdit;
use crate::features::CellShift;
use crate::locale::Locale;
use crate::refs::{MAX_COLS, MAX_ROWS};
use crate::style::{Color, Font, ThemeSlot};
use crate::validation::localized;
use crate::workbook::{Sheet, Workbook};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

// ---- pictures ------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Gif,
    Bmp,
}

impl ImageFormat {
    /// The extension of the part in a file (`xl/media/image1.png`).
    pub fn extension(self) -> &'static str {
        match self {
            ImageFormat::Png => "png",
            ImageFormat::Jpeg => "jpeg",
            ImageFormat::Gif => "gif",
            ImageFormat::Bmp => "bmp",
        }
    }

    /// The content type of the part.
    pub fn mime(self) -> &'static str {
        match self {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::Gif => "image/gif",
            ImageFormat::Bmp => "image/bmp",
        }
    }
}

/// What the header of an image file says.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageInfo {
    pub format: ImageFormat,
    /// Width and height in pixels.
    pub pixels: (u32, u32),
    /// Pixels per inch, horizontally and vertically; 96 when the file
    /// does not say.
    pub dpi: (f32, f32),
}

impl ImageInfo {
    /// The size the picture has when inserted, in pixels at 100 %: its
    /// physical size, as Excel shows it.
    pub fn natural_size(&self) -> (f32, f32) {
        (
            self.pixels.0 as f32 * 96.0 / self.dpi.0,
            self.pixels.1 as f32 * 96.0 / self.dpi.1,
        )
    }
}

fn be16(data: &[u8], at: usize) -> Option<u32> {
    Some(u16::from_be_bytes(data.get(at..at + 2)?.try_into().ok()?) as u32)
}

fn be32(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

fn le16(data: &[u8], at: usize) -> Option<u32> {
    Some(u16::from_le_bytes(data.get(at..at + 2)?.try_into().ok()?) as u32)
}

fn le32(data: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

/// A resolution worth believing, else the 96 of the screen.
fn dpi(x: f32, y: f32) -> (f32, f32) {
    let sane = |v: f32| v.is_finite() && (1.0..=10_000.0).contains(&v);
    if sane(x) && sane(y) { (x, y) } else { (96.0, 96.0) }
}

fn png_info(data: &[u8]) -> Option<ImageInfo> {
    let pixels = (be32(data, 16)?, be32(data, 20)?);
    let mut resolution = (96.0, 96.0);
    // Chunks: length, type, data, checksum. The resolution, when given,
    // comes before the image data.
    let mut at = 8;
    while let (Some(length), Some(kind)) = (be32(data, at), data.get(at + 4..at + 8)) {
        match kind {
            b"pHYs" if data.get(at + 16) == Some(&1) => {
                let per_metre = (be32(data, at + 8)? as f32, be32(data, at + 12)? as f32);
                resolution = dpi(per_metre.0 * 0.0254, per_metre.1 * 0.0254);
            }
            b"IDAT" | b"IEND" => break,
            _ => {}
        }
        at += 12 + length as usize;
    }
    Some(ImageInfo {
        format: ImageFormat::Png,
        pixels,
        dpi: resolution,
    })
}

fn jpeg_info(data: &[u8]) -> Option<ImageInfo> {
    let mut resolution = (96.0, 96.0);
    let mut at = 2;
    loop {
        // Segments: 0xFF, a marker, then (for most) a length that counts
        // itself and the data.
        while *data.get(at)? == 0xff && *data.get(at + 1)? == 0xff {
            at += 1;
        }
        if *data.get(at)? != 0xff {
            return None;
        }
        let marker = *data.get(at + 1)?;
        at += 2;
        if matches!(marker, 0x01 | 0xd0..=0xd8) {
            continue;
        }
        let length = be16(data, at)? as usize;
        match marker {
            // Start of frame: sample size, height, width.
            0xc0..=0xcf if !matches!(marker, 0xc4 | 0xc8 | 0xcc) => {
                return Some(ImageInfo {
                    format: ImageFormat::Jpeg,
                    pixels: (be16(data, at + 5)?, be16(data, at + 3)?),
                    dpi: resolution,
                });
            }
            0xe0 if data.get(at + 2..at + 7) == Some(b"JFIF\0") => {
                let density = (be16(data, at + 10)? as f32, be16(data, at + 12)? as f32);
                match data.get(at + 9) {
                    Some(1) => resolution = dpi(density.0, density.1),
                    Some(2) => resolution = dpi(density.0 * 2.54, density.1 * 2.54),
                    _ => {}
                }
            }
            0xd9 | 0xda => return None,
            _ => {}
        }
        at += length.max(2);
    }
}

fn bmp_info(data: &[u8]) -> Option<ImageInfo> {
    let header = le32(data, 14)?;
    let (pixels, resolution) = if header == 12 {
        ((le16(data, 18)?, le16(data, 20)?), (96.0, 96.0))
    } else {
        // The height is negative when the rows run top to bottom.
        let size = (le32(data, 18)?.unsigned_abs(), le32(data, 22)?.unsigned_abs());
        let per_metre = (le32(data, 38).unwrap_or(0) as f32, le32(data, 42).unwrap_or(0) as f32);
        (size, dpi(per_metre.0 * 0.0254, per_metre.1 * 0.0254))
    };
    Some(ImageInfo {
        format: ImageFormat::Bmp,
        pixels,
        dpi: resolution,
    })
}

/// Recognise an image and read its size from its header, without decoding
/// it. `None` for anything but PNG, JPEG, GIF and BMP, and for images
/// without pixels.
pub fn image_info(data: &[u8]) -> Option<ImageInfo> {
    let info = if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        png_info(data)?
    } else if data.starts_with(&[0xff, 0xd8]) {
        jpeg_info(data)?
    } else if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        ImageInfo {
            format: ImageFormat::Gif,
            pixels: (le16(data, 6)?, le16(data, 8)?),
            dpi: (96.0, 96.0),
        }
    } else if data.starts_with(b"BM") {
        bmp_info(data)?
    } else {
        return None;
    };
    (info.pixels.0 > 0 && info.pixels.1 > 0).then_some(info)
}

/// A picture: the bytes of the image file exactly as inserted, and what
/// the Picture Format tab did to how it shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Picture {
    pub format: ImageFormat,
    /// Shared, so that undo snapshots do not copy images.
    pub data: Arc<[u8]>,
    /// Width and height of the image in pixels.
    pub pixels: (u32, u32),
    /// A vector picture (an icon, an SVG file): the SVG document. `data`
    /// is then the PNG image of it a file keeps beside it for
    /// applications that cannot draw vectors, and what is printed.
    pub svg: Option<Arc<[u8]>>,
    pub adjust: PictureAdjust,
}

impl Picture {
    /// `None` when the bytes are not an image of a supported format.
    pub fn new(data: impl Into<Arc<[u8]>>) -> Option<Picture> {
        let data = data.into();
        let info = image_info(&data)?;
        Some(Picture {
            format: info.format,
            data,
            pixels: info.pixels,
            svg: None,
            adjust: PictureAdjust::default(),
        })
    }

    /// A vector picture: an SVG document and the image that stands for
    /// it where vectors cannot be drawn. `None` when either is not what
    /// it should be.
    pub fn vector(svg: impl Into<Arc<[u8]>>, fallback: impl Into<Arc<[u8]>>) -> Option<Picture> {
        let svg = svg.into();
        crate::svg::svg_size(&svg)?;
        Some(Picture {
            svg: Some(svg),
            ..Picture::new(fallback)?
        })
    }

    /// The size the picture has when inserted, in pixels at 100 %: the
    /// size a vector picture asks for, the physical size of a bitmap.
    pub fn natural_size(&self) -> (f32, f32) {
        self.svg
            .as_deref()
            .and_then(crate::svg::svg_size)
            .or_else(|| image_info(&self.data).map(|info| info.natural_size()))
            .unwrap_or((self.pixels.0 as f32, self.pixels.1 as f32))
    }
}

/// "Recolor" of the Color gallery: the picture in other colours.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Recolor {
    #[default]
    None,
    /// Shades of grey (`a:grayscl`).
    Grayscale,
    /// Black or white, nothing between: pixels lighter than the
    /// threshold (0 to 1) are white (`a:biLevel`).
    BlackAndWhite(f32),
    /// Pale, as behind a text (`a:lum bright="70000" contrast="-70000"`).
    Washout,
    /// Shades between two colours, the dark one for black and the light
    /// one for white (`a:duotone`): sepia, and the accent colours.
    Duotone(Color, Color),
}

/// The two colours of the Sepia recolouring.
pub const SEPIA: (Color, Color) = (Color(0x000000), Color(0xD9C3A5));

/// How a picture shows, besides where: what the Crop, Transparency,
/// Corrections and Color commands change. The default leaves the image
/// as it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PictureAdjust {
    /// What is cut off the image at its left, top, right and bottom, as
    /// fractions of its width and height (`a:srcRect`); negative: room
    /// left empty around it.
    pub crop: [f32; 4],
    /// "Crop to Shape": the outline the picture is cut to (the geometry
    /// of the picture; a rectangle cuts nothing).
    pub shape: ShapePreset,
    /// 0 (opaque) to 1 (`a:alphaModFix`).
    pub transparency: f32,
    /// -1 to 1; 0 leaves the image as it is (`a:lum`).
    pub brightness: f32,
    pub contrast: f32,
    /// 0 (grey) to 4; 1 leaves the colours as they are.
    pub saturation: f32,
    /// Colour temperature in kelvins, 1 500 (cold, blue) to 11 500
    /// (warm, orange); 6 500 leaves the colours as they are.
    pub temperature: f32,
    pub recolor: Recolor,
}

/// The temperature that changes nothing.
pub const NEUTRAL_TEMPERATURE: f32 = 6500.0;

impl Default for PictureAdjust {
    fn default() -> PictureAdjust {
        PictureAdjust {
            crop: [0.0; 4],
            shape: ShapePreset::Rect,
            transparency: 0.0,
            brightness: 0.0,
            contrast: 0.0,
            saturation: 1.0,
            temperature: NEUTRAL_TEMPERATURE,
            recolor: Recolor::None,
        }
    }
}

impl PictureAdjust {
    /// True when the picture is cropped (to a part of the image, or to a
    /// shape).
    pub fn is_cropped(&self) -> bool {
        self.crop != [0.0; 4] || self.shape != ShapePreset::Rect
    }

    /// True when the colours of the image are changed.
    pub fn changes_pixels(&self) -> bool {
        let plain = PictureAdjust {
            crop: self.crop,
            shape: self.shape,
            ..PictureAdjust::default()
        };
        *self != plain
    }

    /// What the adjustments make of one pixel (red, green, blue and
    /// alpha, 0 to 1 each): recolouring first, then saturation and tone,
    /// then brightness and contrast, as Excel stacks them.
    pub fn apply(&self, rgba: [f32; 4]) -> [f32; 4] {
        let [mut r, mut g, mut b, a] = rgba;
        let luma = |r: f32, g: f32, b: f32| 0.299 * r + 0.587 * g + 0.114 * b;
        let (mut brightness, mut contrast) = (self.brightness, self.contrast);
        match self.recolor {
            Recolor::None => {}
            Recolor::Grayscale => {
                let grey = luma(r, g, b);
                (r, g, b) = (grey, grey, grey);
            }
            Recolor::BlackAndWhite(threshold) => {
                let level = if luma(r, g, b) >= threshold { 1.0 } else { 0.0 };
                (r, g, b) = (level, level, level);
            }
            Recolor::Washout => {
                brightness = (brightness + 0.7).clamp(-1.0, 1.0);
                contrast = (contrast - 0.7).clamp(-1.0, 1.0);
            }
            Recolor::Duotone(dark, light) => {
                let t = luma(r, g, b);
                let mix = |shift: u32| {
                    let (lo, hi) = (((dark.value() >> shift) & 0xFF) as f32 / 255.0, ((light.value() >> shift) & 0xFF) as f32 / 255.0);
                    lo + (hi - lo) * t
                };
                (r, g, b) = (mix(16), mix(8), mix(0));
            }
        }
        if self.saturation != 1.0 {
            let grey = luma(r, g, b);
            let s = self.saturation.max(0.0);
            (r, g, b) = (grey + (r - grey) * s, grey + (g - grey) * s, grey + (b - grey) * s);
        }
        if self.temperature != NEUTRAL_TEMPERATURE {
            // Warmer: more red, less blue; colder the other way. A fifth
            // at the ends of the scale.
            let warm = ((self.temperature - NEUTRAL_TEMPERATURE) / 5000.0).clamp(-1.0, 1.0) * 0.2;
            r += warm;
            b -= warm;
        }
        if contrast != 0.0 {
            // -1 flattens everything to mid grey, 1 doubles the spread.
            let stretch = |v: f32| (v - 0.5) * (1.0 + contrast) + 0.5;
            (r, g, b) = (stretch(r), stretch(g), stretch(b));
        }
        if brightness != 0.0 {
            // Towards white or towards black.
            let lift = |v: f32| if brightness >= 0.0 { v + (1.0 - v) * brightness } else { v * (1.0 + brightness) };
            (r, g, b) = (lift(r), lift(g), lift(b));
        }
        let alpha = a * (1.0 - self.transparency.clamp(0.0, 1.0));
        [r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0), alpha]
    }

    /// The frame the whole image would fill when the picture, cropped,
    /// fills `frame`: what the crop handles and panning work on.
    pub fn uncropped(&self, frame: Frame) -> Frame {
        let [l, t, r, b] = self.crop.map(|v| v as f64);
        let kept_w = (1.0 - l - r).max(1e-6);
        let kept_h = (1.0 - t - b).max(1e-6);
        let (width, height) = (frame.width / kept_w, frame.height / kept_h);
        Frame {
            x: frame.x - l * width,
            y: frame.y - t * height,
            width,
            height,
        }
    }

    /// The crop that shows the part `frame` of an image filling `image`.
    pub fn crop_between(image: Frame, frame: Frame) -> [f32; 4] {
        if image.width <= 0.0 || image.height <= 0.0 {
            return [0.0; 4];
        }
        [
            ((frame.x - image.x) / image.width) as f32,
            ((frame.y - image.y) / image.height) as f32,
            ((image.x + image.width - frame.x - frame.width) / image.width) as f32,
            ((image.y + image.height - frame.y - frame.height) / image.height) as f32,
        ]
    }
}

// ---- effects ---------------------------------------------------------------

/// An outer shadow (`a:outerShdw`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub color: Color,
    /// 0 (solid) to 1.
    pub transparency: f32,
    /// The shadow as a fraction of the size of what casts it (1 = the
    /// same size; Excel offers 1 % to 200 %).
    pub size: f32,
    /// Radius of the blur, points.
    pub blur: f32,
    /// Where the shadow falls: degrees clockwise from the right.
    pub angle: f32,
    /// How far, points.
    pub distance: f32,
}

impl Default for Shadow {
    /// Excel's "Offset: Bottom Right".
    fn default() -> Shadow {
        Shadow {
            color: Color::BLACK,
            transparency: 0.6,
            size: 1.0,
            blur: 4.0,
            angle: 45.0,
            distance: 3.0,
        }
    }
}

/// The nine outer presets of the Shadow gallery, in its order: where the
/// shadow falls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShadowPreset {
    BottomRight,
    Bottom,
    BottomLeft,
    Right,
    Center,
    Left,
    TopRight,
    Top,
    TopLeft,
}

impl ShadowPreset {
    pub const ALL: [ShadowPreset; 9] = [
        ShadowPreset::BottomRight,
        ShadowPreset::Bottom,
        ShadowPreset::BottomLeft,
        ShadowPreset::Right,
        ShadowPreset::Center,
        ShadowPreset::Left,
        ShadowPreset::TopRight,
        ShadowPreset::Top,
        ShadowPreset::TopLeft,
    ];

    /// The shadow Excel gives the preset.
    pub fn shadow(self) -> Shadow {
        let angle = match self {
            ShadowPreset::Right | ShadowPreset::Center => 0.0,
            ShadowPreset::BottomRight => 45.0,
            ShadowPreset::Bottom => 90.0,
            ShadowPreset::BottomLeft => 135.0,
            ShadowPreset::Left => 180.0,
            ShadowPreset::TopLeft => 225.0,
            ShadowPreset::Top => 270.0,
            ShadowPreset::TopRight => 315.0,
        };
        if self == ShadowPreset::Center {
            // All around: a little larger than the shape, not moved.
            return Shadow {
                size: 1.02,
                blur: 5.0,
                angle,
                distance: 0.0,
                ..Shadow::default()
            };
        }
        Shadow { angle, ..Shadow::default() }
    }

    pub fn name(self, locale: &Locale) -> &'static str {
        let names = match self {
            ShadowPreset::BottomRight => ["Offset: Bottom Right", "Décalage : bas à droite", "Desplazamiento: abajo derecha", "Offset: unten rechts", "Scostamento in basso a destra"],
            ShadowPreset::Bottom => ["Offset: Bottom", "Décalage : bas", "Desplazamiento: abajo", "Offset: unten", "Scostamento in basso"],
            ShadowPreset::BottomLeft => ["Offset: Bottom Left", "Décalage : bas à gauche", "Desplazamiento: abajo izquierda", "Offset: unten links", "Scostamento in basso a sinistra"],
            ShadowPreset::Right => ["Offset: Right", "Décalage : droite", "Desplazamiento: derecha", "Offset: rechts", "Scostamento a destra"],
            ShadowPreset::Center => ["Offset: Center", "Décalage : centre", "Desplazamiento: centro", "Offset: Mitte", "Scostamento al centro"],
            ShadowPreset::Left => ["Offset: Left", "Décalage : gauche", "Desplazamiento: izquierda", "Offset: links", "Scostamento a sinistra"],
            ShadowPreset::TopRight => ["Offset: Top Right", "Décalage : haut à droite", "Desplazamiento: arriba derecha", "Offset: oben rechts", "Scostamento in alto a destra"],
            ShadowPreset::Top => ["Offset: Top", "Décalage : haut", "Desplazamiento: arriba", "Offset: oben", "Scostamento in alto"],
            ShadowPreset::TopLeft => ["Offset: Top Left", "Décalage : haut à gauche", "Desplazamiento: arriba izquierda", "Offset: oben links", "Scostamento in alto a sinistra"],
        };
        localized(locale, names)
    }
}

/// A halo of one colour around a drawing (`a:glow`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Glow {
    pub color: Color,
    /// 0 (solid) to 1.
    pub transparency: f32,
    /// How far it reaches, points.
    pub radius: f32,
}

/// The effects of a picture or a shape that are drawn (`a:effectLst`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Effects {
    pub shadow: Option<Shadow>,
    pub glow: Option<Glow>,
    /// "Soft Edges": how far in from its edges the drawing fades out,
    /// points; 0 for sharp edges (`a:softEdge`).
    pub soft_edge: f32,
}

impl Effects {
    pub fn is_none(&self) -> bool {
        self.shadow.is_none() && self.glow.is_none() && self.soft_edge <= 0.0
    }
}

// ---- shapes --------------------------------------------------------------

/// One step of an outline. Coordinates are fractions of the frame: (0, 0)
/// is its top-left corner and (1, 1) its bottom-right one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathCommand {
    MoveTo(f32, f32),
    LineTo(f32, f32),
    /// A cubic Bézier curve: two control points, then the end point.
    CurveTo(f32, f32, f32, f32, f32, f32),
    Close,
}

/// The geometries of Excel's shape gallery that are supported (`prst` of
/// `<a:prstGeom>`), with their default proportions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShapePreset {
    Rect,
    RoundRect,
    Ellipse,
    /// Isosceles triangle.
    Triangle,
    RightTriangle,
    Diamond,
    Pentagon,
    Hexagon,
    RightArrow,
    LeftArrow,
    UpArrow,
    DownArrow,
    Star5,
    /// A straight line from the top-left to the bottom-right corner of
    /// the frame (flipping gives the other diagonal). Arrowheads are part
    /// of the outline.
    Line,
    /// A rectangular speech bubble; its tail points below the frame.
    Callout,
    /// An arrow-shaped band pointing right, notched on the left: the
    /// steps of a process (`chevron`). `Shape::adjust` is how deep its
    /// point is, as a fraction of the shorter side (0.5 when not given).
    Chevron,
    /// A trapezoid, its short side on top: the levels of a pyramid.
    /// `Shape::adjust` is how far in its top corners are, as a fraction
    /// of the shorter side (0.25 when not given).
    Trapezoid,
}

/// How far a quarter circle's control points sit from its ends.
const KAPPA: f32 = 0.552_284_8;

impl ShapePreset {
    pub const ALL: [ShapePreset; 17] = [
        ShapePreset::Rect,
        ShapePreset::RoundRect,
        ShapePreset::Ellipse,
        ShapePreset::Triangle,
        ShapePreset::RightTriangle,
        ShapePreset::Diamond,
        ShapePreset::Pentagon,
        ShapePreset::Hexagon,
        ShapePreset::RightArrow,
        ShapePreset::LeftArrow,
        ShapePreset::UpArrow,
        ShapePreset::DownArrow,
        ShapePreset::Star5,
        ShapePreset::Line,
        ShapePreset::Callout,
        ShapePreset::Chevron,
        ShapePreset::Trapezoid,
    ];

    /// The `prst` attribute. Excel writes a line with arrowheads as
    /// `straightConnector1`; both spellings are read as a line.
    pub fn ooxml_name(self) -> &'static str {
        match self {
            ShapePreset::Rect => "rect",
            ShapePreset::RoundRect => "roundRect",
            ShapePreset::Ellipse => "ellipse",
            ShapePreset::Triangle => "triangle",
            ShapePreset::RightTriangle => "rtTriangle",
            ShapePreset::Diamond => "diamond",
            ShapePreset::Pentagon => "pentagon",
            ShapePreset::Hexagon => "hexagon",
            ShapePreset::RightArrow => "rightArrow",
            ShapePreset::LeftArrow => "leftArrow",
            ShapePreset::UpArrow => "upArrow",
            ShapePreset::DownArrow => "downArrow",
            ShapePreset::Star5 => "star5",
            ShapePreset::Line => "line",
            ShapePreset::Callout => "wedgeRectCallout",
            ShapePreset::Chevron => "chevron",
            ShapePreset::Trapezoid => "trapezoid",
        }
    }

    pub fn from_ooxml_name(name: &str) -> Option<ShapePreset> {
        if name == "straightConnector1" {
            return Some(ShapePreset::Line);
        }
        ShapePreset::ALL.into_iter().find(|p| p.ooxml_name() == name)
    }

    /// A line has no inside: it is stroked, never filled.
    pub fn is_line(self) -> bool {
        self == ShapePreset::Line
    }

    /// Length of the head of a block arrow along its axis, as a fraction
    /// of the frame: half the shorter side, as Excel draws it.
    fn arrow_head(along: f32, across: f32) -> f32 {
        if along <= 0.0 {
            0.5
        } else {
            (along.min(across) * 0.5 / along).min(1.0)
        }
    }

    /// The outline of the shape in a frame of the given size. The size
    /// only sets the proportions (round corners stay round, arrow heads
    /// keep their angle): coordinates are fractions of the frame, to be
    /// multiplied by its width and height. Flips, then the rotation about
    /// the centre of the frame, are applied by whoever draws.
    pub fn path(self, width: f32, height: f32) -> Vec<PathCommand> {
        self.path_adjusted(width, height, None)
    }

    /// How far in the point of a chevron or the top corners of a
    /// trapezoid are, as a fraction of the width of the frame. `adjust`
    /// counts in shorter sides, as files do.
    fn inset(self, width: f32, height: f32, adjust: Option<f32>) -> f32 {
        let (default, most) = if self == ShapePreset::Chevron { (0.5, 1.0) } else { (0.25, 0.5) };
        if width <= 0.0 {
            return most;
        }
        let amount = adjust.filter(|a| a.is_finite()).unwrap_or(default).max(0.0);
        (amount * width.min(height) / width).min(most)
    }

    /// The outline of the shape with its adjustment (`Shape::adjust`):
    /// only chevrons and trapezoids have one here.
    pub fn path_adjusted(self, width: f32, height: f32, adjust: Option<f32>) -> Vec<PathCommand> {
        use PathCommand::{Close, CurveTo, LineTo, MoveTo};
        let polygon = |points: &[(f32, f32)]| -> Vec<PathCommand> {
            let mut path: Vec<PathCommand> = points
                .iter()
                .enumerate()
                .map(|(i, (x, y))| if i == 0 { MoveTo(*x, *y) } else { LineTo(*x, *y) })
                .collect();
            path.push(Close);
            path
        };
        // A star's points and a pentagon's corners lie on an ellipse a
        // little larger than the frame, so that the figure fills it.
        let (reach_x, reach_y, centre_y) = (0.5 * 1.051_46, 0.5 * 1.105_57, 0.5 * 1.105_57);
        let on_ellipse = |degrees: f32, scale: f32| {
            let angle = degrees.to_radians();
            (
                0.5 + angle.cos() * reach_x * scale,
                centre_y - angle.sin() * reach_y * scale,
            )
        };
        match self {
            ShapePreset::Rect => polygon(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]),
            ShapePreset::RoundRect => {
                let radius = width.min(height) / 6.0;
                let rx = if width > 0.0 { radius / width } else { 0.0 };
                let ry = if height > 0.0 { radius / height } else { 0.0 };
                let (kx, ky) = (rx * (1.0 - KAPPA), ry * (1.0 - KAPPA));
                vec![
                    MoveTo(rx, 0.0),
                    LineTo(1.0 - rx, 0.0),
                    CurveTo(1.0 - kx, 0.0, 1.0, ky, 1.0, ry),
                    LineTo(1.0, 1.0 - ry),
                    CurveTo(1.0, 1.0 - ky, 1.0 - kx, 1.0, 1.0 - rx, 1.0),
                    LineTo(rx, 1.0),
                    CurveTo(kx, 1.0, 0.0, 1.0 - ky, 0.0, 1.0 - ry),
                    LineTo(0.0, ry),
                    CurveTo(0.0, ky, kx, 0.0, rx, 0.0),
                    Close,
                ]
            }
            ShapePreset::Ellipse => {
                let k = 0.5 * KAPPA;
                vec![
                    MoveTo(1.0, 0.5),
                    CurveTo(1.0, 0.5 + k, 0.5 + k, 1.0, 0.5, 1.0),
                    CurveTo(0.5 - k, 1.0, 0.0, 0.5 + k, 0.0, 0.5),
                    CurveTo(0.0, 0.5 - k, 0.5 - k, 0.0, 0.5, 0.0),
                    CurveTo(0.5 + k, 0.0, 1.0, 0.5 - k, 1.0, 0.5),
                    Close,
                ]
            }
            ShapePreset::Triangle => polygon(&[(0.0, 1.0), (0.5, 0.0), (1.0, 1.0)]),
            ShapePreset::RightTriangle => polygon(&[(0.0, 1.0), (0.0, 0.0), (1.0, 1.0)]),
            ShapePreset::Diamond => polygon(&[(0.0, 0.5), (0.5, 0.0), (1.0, 0.5), (0.5, 1.0)]),
            ShapePreset::Pentagon => {
                let corners: Vec<(f32, f32)> = [162.0, 90.0, 18.0, 306.0, 234.0]
                    .iter()
                    .map(|a| on_ellipse(*a, 1.0))
                    .collect();
                polygon(&corners)
            }
            ShapePreset::Hexagon => {
                let inset = if width > 0.0 {
                    (width.min(height) * 0.25 / width).min(0.5)
                } else {
                    0.25
                };
                polygon(&[
                    (0.0, 0.5),
                    (inset, 0.0),
                    (1.0 - inset, 0.0),
                    (1.0, 0.5),
                    (1.0 - inset, 1.0),
                    (inset, 1.0),
                ])
            }
            ShapePreset::RightArrow | ShapePreset::LeftArrow => {
                let neck = 1.0 - ShapePreset::arrow_head(width, height);
                let right = [
                    (0.0, 0.25),
                    (neck, 0.25),
                    (neck, 0.0),
                    (1.0, 0.5),
                    (neck, 1.0),
                    (neck, 0.75),
                    (0.0, 0.75),
                ];
                let mirrored = self == ShapePreset::LeftArrow;
                polygon(&right.map(|(x, y)| if mirrored { (1.0 - x, y) } else { (x, y) }))
            }
            ShapePreset::UpArrow | ShapePreset::DownArrow => {
                let neck = 1.0 - ShapePreset::arrow_head(height, width);
                let down = [
                    (0.25, 0.0),
                    (0.25, neck),
                    (0.0, neck),
                    (0.5, 1.0),
                    (1.0, neck),
                    (0.75, neck),
                    (0.75, 0.0),
                ];
                let mirrored = self == ShapePreset::UpArrow;
                polygon(&down.map(|(x, y)| if mirrored { (x, 1.0 - y) } else { (x, y) }))
            }
            ShapePreset::Star5 => {
                // Points and hollows alternate; the hollows lie at 0.382
                // of the way out, where the lines of a pentagram cross.
                let corners: Vec<(f32, f32)> = (0..10)
                    .map(|i| on_ellipse(162.0 - 36.0 * i as f32, if i % 2 == 0 { 1.0 } else { 0.381_96 }))
                    .collect();
                polygon(&corners)
            }
            ShapePreset::Line => vec![MoveTo(0.0, 0.0), LineTo(1.0, 1.0)],
            ShapePreset::Callout => polygon(&[
                (0.0, 0.0),
                (1.0, 0.0),
                (1.0, 1.0),
                (5.0 / 12.0, 1.0),
                (0.5 - 0.208_33, 0.5 + 0.625),
                (2.0 / 12.0, 1.0),
                (0.0, 1.0),
            ]),
            ShapePreset::Chevron => {
                let d = self.inset(width, height, adjust);
                polygon(&[(0.0, 0.0), (1.0 - d, 0.0), (1.0, 0.5), (1.0 - d, 1.0), (0.0, 1.0), (d, 0.5)])
            }
            ShapePreset::Trapezoid => {
                let d = self.inset(width, height, adjust);
                polygon(&[(0.0, 1.0), (d, 0.0), (1.0 - d, 0.0), (1.0, 1.0)])
            }
        }
    }

    /// The part of the frame the text is laid out in, before its margins
    /// (`TEXT_MARGIN`): left, top, right and bottom as fractions of the
    /// frame of the given size.
    pub fn text_rect(self, width: f32, height: f32) -> [f32; 4] {
        self.text_rect_adjusted(width, height, None)
    }

    /// The text rectangle of the shape with its adjustment.
    pub fn text_rect_adjusted(self, width: f32, height: f32, adjust: Option<f32>) -> [f32; 4] {
        match self {
            ShapePreset::Rect | ShapePreset::Line | ShapePreset::Callout => [0.0, 0.0, 1.0, 1.0],
            ShapePreset::RoundRect => {
                // Clear of the round corners.
                let inset = width.min(height) / 6.0 * 0.292_89;
                let x = if width > 0.0 { inset / width } else { 0.0 };
                let y = if height > 0.0 { inset / height } else { 0.0 };
                [x, y, 1.0 - x, 1.0 - y]
            }
            ShapePreset::Ellipse => [0.146_45, 0.146_45, 0.853_55, 0.853_55],
            ShapePreset::Triangle => [0.25, 0.5, 0.75, 1.0],
            ShapePreset::RightTriangle => [1.0 / 12.0, 7.0 / 12.0, 7.0 / 12.0, 11.0 / 12.0],
            ShapePreset::Diamond => [0.25, 0.25, 0.75, 0.75],
            ShapePreset::Pentagon => [0.190_98, 0.236_07, 0.809_02, 1.0],
            ShapePreset::Hexagon => [0.125, 0.25, 0.875, 0.75],
            ShapePreset::RightArrow => [0.0, 0.25, 1.0 - ShapePreset::arrow_head(width, height) / 2.0, 0.75],
            ShapePreset::LeftArrow => [ShapePreset::arrow_head(width, height) / 2.0, 0.25, 1.0, 0.75],
            ShapePreset::UpArrow => [0.25, ShapePreset::arrow_head(height, width) / 2.0, 0.75, 1.0],
            ShapePreset::DownArrow => [0.25, 0.0, 0.75, 1.0 - ShapePreset::arrow_head(height, width) / 2.0],
            ShapePreset::Star5 => [0.309_02, 0.381_97, 0.690_98, 0.763_93],
            // Between the notch and the point; in the part of a trapezoid
            // that is as wide at the top as at the bottom, more or less.
            ShapePreset::Chevron => {
                let d = self.inset(width, height, adjust).min(0.4);
                [d, 0.0, 1.0 - d, 1.0]
            }
            ShapePreset::Trapezoid => {
                let d = self.inset(width, height, adjust) * 2.0 / 3.0;
                [d, 0.0, 1.0 - d, 1.0]
            }
        }
    }
}

/// Space kept between the text rectangle of a shape and its text, in
/// pixels at 100 %: left and right, top and bottom (0.1 and 0.05 inch).
pub const TEXT_MARGIN: (f32, f32) = (9.6, 4.8);

/// The dash of an outline (`val` of `<a:prstDash>`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Dash {
    #[default]
    Solid,
    /// Round dots.
    SysDot,
    /// Square dots.
    SysDash,
    Dash,
    DashDot,
    LongDash,
    LongDashDot,
    LongDashDotDot,
    Dot,
    SysDashDot,
    SysDashDotDot,
}

impl Dash {
    /// The first eight in the order of Excel's Dashes menu.
    pub const ALL: [Dash; 11] = [
        Dash::Solid,
        Dash::SysDot,
        Dash::SysDash,
        Dash::Dash,
        Dash::DashDot,
        Dash::LongDash,
        Dash::LongDashDot,
        Dash::LongDashDotDot,
        Dash::Dot,
        Dash::SysDashDot,
        Dash::SysDashDotDot,
    ];

    pub fn ooxml_name(self) -> &'static str {
        match self {
            Dash::Solid => "solid",
            Dash::SysDot => "sysDot",
            Dash::SysDash => "sysDash",
            Dash::Dash => "dash",
            Dash::DashDot => "dashDot",
            Dash::LongDash => "lgDash",
            Dash::LongDashDot => "lgDashDot",
            Dash::LongDashDotDot => "lgDashDotDot",
            Dash::Dot => "dot",
            Dash::SysDashDot => "sysDashDot",
            Dash::SysDashDotDot => "sysDashDotDot",
        }
    }

    pub fn from_ooxml_name(name: &str) -> Option<Dash> {
        Dash::ALL.into_iter().find(|d| d.ooxml_name() == name)
    }

    /// Lengths of the dashes and of the gaps between them, in multiples
    /// of the line width; empty for a solid line.
    pub fn pattern(self) -> &'static [f32] {
        match self {
            Dash::Solid => &[],
            Dash::SysDot => &[1.0, 1.0],
            Dash::SysDash => &[3.0, 1.0],
            Dash::Dash => &[4.0, 3.0],
            Dash::DashDot => &[4.0, 3.0, 1.0, 3.0],
            Dash::LongDash => &[8.0, 3.0],
            Dash::LongDashDot => &[8.0, 3.0, 1.0, 3.0],
            Dash::LongDashDotDot => &[8.0, 3.0, 1.0, 3.0, 1.0, 3.0],
            Dash::Dot => &[1.0, 3.0],
            Dash::SysDashDot => &[3.0, 1.0, 1.0, 1.0],
            Dash::SysDashDotDot => &[3.0, 1.0, 1.0, 1.0, 1.0, 1.0],
        }
    }
}

/// What a line ends with (`type` of `<a:headEnd>` / `<a:tailEnd>`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LineEnd {
    #[default]
    None,
    /// The filled arrowhead of Excel's arrows.
    Triangle,
    Stealth,
    Diamond,
    Oval,
    /// An open arrowhead (two strokes).
    Open,
}

impl LineEnd {
    pub const ALL: [LineEnd; 6] = [
        LineEnd::None,
        LineEnd::Triangle,
        LineEnd::Stealth,
        LineEnd::Diamond,
        LineEnd::Oval,
        LineEnd::Open,
    ];

    pub fn ooxml_name(self) -> &'static str {
        match self {
            LineEnd::None => "none",
            LineEnd::Triangle => "triangle",
            LineEnd::Stealth => "stealth",
            LineEnd::Diamond => "diamond",
            LineEnd::Oval => "oval",
            LineEnd::Open => "arrow",
        }
    }

    pub fn from_ooxml_name(name: &str) -> Option<LineEnd> {
        LineEnd::ALL.into_iter().find(|e| e.ooxml_name() == name)
    }
}

/// The line around a shape, or the line itself (`<a:ln>`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Outline {
    pub color: Color,
    /// Width in points.
    pub width: f32,
    pub dash: Dash,
    /// At the first point of the path (`headEnd`).
    pub start: LineEnd,
    /// At the last point of the path (`tailEnd`).
    pub end: LineEnd,
}

impl Outline {
    pub fn solid(color: Color, width: f32) -> Outline {
        Outline {
            color,
            width,
            dash: Dash::Solid,
            start: LineEnd::None,
            end: LineEnd::None,
        }
    }
}

/// Horizontal alignment of the text of a shape (`algn` of `<a:pPr>`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

impl TextAlign {
    pub const ALL: [TextAlign; 4] = [TextAlign::Left, TextAlign::Center, TextAlign::Right, TextAlign::Justify];

    pub fn ooxml_name(self) -> &'static str {
        match self {
            TextAlign::Left => "l",
            TextAlign::Center => "ctr",
            TextAlign::Right => "r",
            TextAlign::Justify => "just",
        }
    }

    pub fn from_ooxml_name(name: &str) -> Option<TextAlign> {
        TextAlign::ALL.into_iter().find(|a| a.ooxml_name() == name)
    }
}

/// Vertical position of the text in a shape (`anchor` of `<a:bodyPr>`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextAnchor {
    #[default]
    Top,
    Middle,
    Bottom,
}

impl TextAnchor {
    pub const ALL: [TextAnchor; 3] = [TextAnchor::Top, TextAnchor::Middle, TextAnchor::Bottom];

    pub fn ooxml_name(self) -> &'static str {
        match self {
            TextAnchor::Top => "t",
            TextAnchor::Middle => "ctr",
            TextAnchor::Bottom => "b",
        }
    }

    pub fn from_ooxml_name(name: &str) -> Option<TextAnchor> {
        TextAnchor::ALL.into_iter().find(|a| a.ooxml_name() == name)
    }
}

/// A shape: a preset geometry with a fill, an outline and text.
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub preset: ShapePreset,
    /// Inserted as a text box (`txBox` in the file): it behaves like any
    /// rectangle, the flag only keeps what it was.
    pub text_box: bool,
    /// `None`: no fill.
    pub fill: Option<Color>,
    /// `None`: no outline.
    pub outline: Option<Outline>,
    /// Paragraphs are separated by line feeds.
    pub text: String,
    /// The font of the whole text; its colour `None` is black.
    pub font: Font,
    pub align: TextAlign,
    pub anchor: TextAnchor,
    /// What the letters have besides their font: an outline, a shadow
    /// (WordArt, see `wordart.rs`).
    pub effects: crate::wordart::TextEffects,
    /// The looks of a text formatted in parts; empty when the whole text
    /// has the look of `font` and `effects`.
    pub runs: Vec<crate::wordart::TextRun>,
    /// Lines are broken at the width of the shape (`wrap` of
    /// `<a:bodyPr>`; WordArt is not wrapped).
    pub wrap: bool,
    /// The adjustment of the geometry (`adj` of `<a:avLst>`), for the
    /// presets that have one here; `None` is the preset's own.
    pub adjust: Option<f32>,
    /// The text is an equation (see `math.rs`); `text` then holds it in
    /// the linear format, which is what is typed and what applications
    /// that draw no equations show.
    pub math: Option<crate::math::Equation>,
}

impl Shape {
    /// The outline of the shape in a frame of the given size (see
    /// `ShapePreset::path`).
    pub fn path(&self, width: f32, height: f32) -> Vec<PathCommand> {
        self.preset.path_adjusted(width, height, self.adjust)
    }

    /// The part of the frame the text is laid out in (see
    /// `ShapePreset::text_rect`).
    pub fn text_rect(&self, width: f32, height: f32) -> [f32; 4] {
        self.preset.text_rect_adjusted(width, height, self.adjust)
    }
}

/// The entries of the Insert ▸ Shapes gallery: a geometry with the look
/// Excel gives it at first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShapeTool {
    Rectangle,
    RoundedRectangle,
    Ellipse,
    Triangle,
    RightTriangle,
    Diamond,
    Pentagon,
    Hexagon,
    RightArrow,
    LeftArrow,
    UpArrow,
    DownArrow,
    Star5,
    Line,
    /// A line with an arrowhead at its end.
    Arrow,
    /// A line with an arrowhead at both ends.
    DoubleArrow,
    TextBox,
    Callout,
    Chevron,
    Trapezoid,
}

/// Fill of new shapes and colour of new lines (the first accent of the
/// theme, here with its value in the Office theme: `add_drawing` gives it
/// the one of the workbook), and the darker shade of their outline.
const ACCENT: Color = Color::themed(ThemeSlot::Accent1, 0.0, 0x156082);
const ACCENT_DARK: Color = Color(0x042433);

impl ShapeTool {
    pub const ALL: [ShapeTool; 20] = [
        ShapeTool::Rectangle,
        ShapeTool::RoundedRectangle,
        ShapeTool::Ellipse,
        ShapeTool::Triangle,
        ShapeTool::RightTriangle,
        ShapeTool::Diamond,
        ShapeTool::Pentagon,
        ShapeTool::Hexagon,
        ShapeTool::RightArrow,
        ShapeTool::LeftArrow,
        ShapeTool::UpArrow,
        ShapeTool::DownArrow,
        ShapeTool::Star5,
        ShapeTool::Line,
        ShapeTool::Arrow,
        ShapeTool::DoubleArrow,
        ShapeTool::TextBox,
        ShapeTool::Callout,
        ShapeTool::Chevron,
        ShapeTool::Trapezoid,
    ];

    pub fn preset(self) -> ShapePreset {
        match self {
            ShapeTool::Rectangle | ShapeTool::TextBox => ShapePreset::Rect,
            ShapeTool::RoundedRectangle => ShapePreset::RoundRect,
            ShapeTool::Ellipse => ShapePreset::Ellipse,
            ShapeTool::Triangle => ShapePreset::Triangle,
            ShapeTool::RightTriangle => ShapePreset::RightTriangle,
            ShapeTool::Diamond => ShapePreset::Diamond,
            ShapeTool::Pentagon => ShapePreset::Pentagon,
            ShapeTool::Hexagon => ShapePreset::Hexagon,
            ShapeTool::RightArrow => ShapePreset::RightArrow,
            ShapeTool::LeftArrow => ShapePreset::LeftArrow,
            ShapeTool::UpArrow => ShapePreset::UpArrow,
            ShapeTool::DownArrow => ShapePreset::DownArrow,
            ShapeTool::Star5 => ShapePreset::Star5,
            ShapeTool::Line | ShapeTool::Arrow | ShapeTool::DoubleArrow => ShapePreset::Line,
            ShapeTool::Callout => ShapePreset::Callout,
            ShapeTool::Chevron => ShapePreset::Chevron,
            ShapeTool::Trapezoid => ShapePreset::Trapezoid,
        }
    }

    /// A new shape as Excel first draws it.
    pub fn shape(self) -> Shape {
        let preset = self.preset();
        let mut shape = Shape {
            preset,
            text_box: false,
            fill: Some(ACCENT),
            outline: Some(Outline::solid(ACCENT_DARK, 1.0)),
            text: String::new(),
            font: Font {
                color: Some(Color::WHITE),
                ..Font::default()
            },
            align: TextAlign::Center,
            anchor: TextAnchor::Middle,
            effects: Default::default(),
            runs: Vec::new(),
            wrap: true,
            adjust: None,
            math: None,
        };
        if preset.is_line() {
            let arrow = |pointed: bool| if pointed { LineEnd::Triangle } else { LineEnd::None };
            shape.fill = None;
            shape.font.color = None;
            shape.outline = Some(Outline {
                start: arrow(self == ShapeTool::DoubleArrow),
                end: arrow(self != ShapeTool::Line),
                ..Outline::solid(ACCENT, 1.0)
            });
        }
        if self == ShapeTool::TextBox {
            shape.text_box = true;
            shape.fill = Some(Color::WHITE);
            shape.outline = Some(Outline::solid(Color(0xBCBCBC), 0.75));
            shape.font.color = None;
            shape.align = TextAlign::Left;
            shape.anchor = TextAnchor::Top;
        }
        shape
    }

    /// The size a shape gets from a single click, in pixels at 100 %.
    pub fn default_size(self) -> (f32, f32) {
        match self {
            ShapeTool::RightArrow | ShapeTool::LeftArrow | ShapeTool::TextBox | ShapeTool::Chevron => (96.0, 48.0),
            ShapeTool::UpArrow | ShapeTool::DownArrow => (48.0, 96.0),
            ShapeTool::Line | ShapeTool::Arrow | ShapeTool::DoubleArrow => (96.0, 0.0),
            ShapeTool::Callout => (144.0, 72.0),
            _ => (96.0, 96.0),
        }
    }

    /// The name Excel gives such a shape, before its number.
    pub fn name(self, locale: &Locale) -> &'static str {
        localized(
            locale,
            match self {
                ShapeTool::Rectangle => ["Rectangle", "Rectangle", "Rectángulo", "Rechteck", "Rettangolo"],
                ShapeTool::RoundedRectangle => [
                    "Rounded Rectangle",
                    "Rectangle arrondi",
                    "Rectángulo redondeado",
                    "Abgerundetes Rechteck",
                    "Rettangolo smussato",
                ],
                ShapeTool::Ellipse => ["Oval", "Ellipse", "Óvalo", "Ellipse", "Ovale"],
                ShapeTool::Triangle => [
                    "Isosceles Triangle",
                    "Triangle isocèle",
                    "Triángulo isósceles",
                    "Gleichschenkliges Dreieck",
                    "Triangolo isoscele",
                ],
                ShapeTool::RightTriangle => [
                    "Right Triangle",
                    "Triangle rectangle",
                    "Triángulo rectángulo",
                    "Rechtwinkliges Dreieck",
                    "Triangolo rettangolo",
                ],
                ShapeTool::Diamond => ["Diamond", "Losange", "Rombo", "Raute", "Diamante"],
                ShapeTool::Pentagon => [
                    "Regular Pentagon",
                    "Pentagone régulier",
                    "Pentágono regular",
                    "Regelmäßiges Fünfeck",
                    "Pentagono regolare",
                ],
                ShapeTool::Hexagon => ["Hexagon", "Hexagone", "Hexágono", "Sechseck", "Esagono"],
                ShapeTool::RightArrow => [
                    "Right Arrow",
                    "Flèche droite",
                    "Flecha derecha",
                    "Pfeil nach rechts",
                    "Freccia a destra",
                ],
                ShapeTool::LeftArrow => [
                    "Left Arrow",
                    "Flèche gauche",
                    "Flecha izquierda",
                    "Pfeil nach links",
                    "Freccia a sinistra",
                ],
                ShapeTool::UpArrow => [
                    "Up Arrow",
                    "Flèche vers le haut",
                    "Flecha arriba",
                    "Pfeil nach oben",
                    "Freccia su",
                ],
                ShapeTool::DownArrow => [
                    "Down Arrow",
                    "Flèche vers le bas",
                    "Flecha abajo",
                    "Pfeil nach unten",
                    "Freccia in giù",
                ],
                ShapeTool::Star5 => [
                    "5-Point Star",
                    "Étoile à 5 branches",
                    "Estrella de 5 puntas",
                    "Stern mit 5 Zacken",
                    "Stella a 5 punte",
                ],
                ShapeTool::Line => [
                    "Straight Connector",
                    "Lien droit",
                    "Conector recto",
                    "Gerader Verbinder",
                    "Connettore dritto",
                ],
                ShapeTool::Arrow | ShapeTool::DoubleArrow => [
                    "Straight Arrow Connector",
                    "Connecteur droit avec flèche",
                    "Conector recto de flecha",
                    "Gerader pfeilförmiger Verbinder",
                    "Connettore dritto a freccia",
                ],
                ShapeTool::TextBox => ["TextBox", "ZoneTexte", "CuadroTexto", "Textfeld", "CasellaDiTesto"],
                ShapeTool::Chevron => ["Chevron", "Chevron", "Cheurón", "Chevron", "Gallone"],
                ShapeTool::Trapezoid => ["Trapezoid", "Trapèze", "Trapecio", "Trapezoid", "Trapezio"],
                ShapeTool::Callout => [
                    "Rectangular Callout",
                    "Rectangle",
                    "Llamada rectangular",
                    "Rechteckige Legende",
                    "Callout rettangolare",
                ],
            },
        )
    }
}

/// The name Excel gives a picture, before its number.
fn picture_name(locale: &Locale) -> &'static str {
    localized(locale, ["Picture", "Image", "Imagen", "Bild", "Immagine"])
}

/// The name Excel gives a vector picture, before its number.
fn graphic_name(locale: &Locale) -> &'static str {
    localized(locale, ["Graphic", "Graphique", "Gráfico", "Grafik", "Elemento grafico"])
}

/// The name Excel gives a group, before its number.
fn group_name(locale: &Locale) -> &'static str {
    localized(locale, ["Group", "Groupe", "Grupo", "Gruppieren", "Gruppo"])
}

// ---- drawings ------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum DrawingKind {
    Picture(Picture),
    Shape(Shape),
    /// A SmartArt graphic (see `smartart.rs`).
    SmartArt(crate::smartart::SmartArt),
    Group(Group),
}

/// Pictures and shapes held together: one object on the sheet, which
/// moves, resizes, turns and flips its members with it.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// The members, bottom-most first; a member may be a group itself.
    /// A member is placed in its group, not on the sheet: its `offset`
    /// is its corner from the corner of the group and its `size` its
    /// size while the group has the size `extent`. Its rotation and
    /// flips are its own, before those of the group. Its `anchor`, `to`,
    /// `placement` and `z` mean nothing.
    pub members: Vec<Drawing>,
    /// The size of the group in which the places of its members are
    /// given; they stretch with the group from there.
    pub extent: (f32, f32),
}

impl Group {
    /// Where a member is when the group is in `frame`, before the group
    /// is turned or flipped (what a file says of it).
    pub fn member_frame(&self, frame: Frame, member: &Drawing) -> Frame {
        let scale = |length: f64, extent: f32| if extent > 0.0 { length / extent as f64 } else { 1.0 };
        let (sx, sy) = (scale(frame.width, self.extent.0), scale(frame.height, self.extent.1));
        Frame {
            x: frame.x + member.offset.0 as f64 * sx,
            y: frame.y + member.offset.1 as f64 * sy,
            width: member.size.0 as f64 * sx,
            height: member.size.1 as f64 * sy,
        }
    }
}

/// A picture or a shape as it shows on the sheet: one that stands alone,
/// or a member of a group, where the groups around it put it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Leaf<'a> {
    pub drawing: &'a Drawing,
    pub frame: Frame,
    pub rotation: f32,
    pub flip_h: bool,
    pub flip_v: bool,
}

/// How a drawing follows the cells under it (`editAs` of its anchor;
/// `twoCell` when a file leaves it out).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Placement {
    /// "Move and size with cells" (`twoCell`): both corners are pinned to
    /// cells, so resizing, hiding, inserting or deleting the rows and
    /// columns it covers stretches or squeezes it.
    #[default]
    MoveAndSize,
    /// "Move but don't size with cells" (`oneCell`): the top-left corner
    /// is pinned to a cell, the size is kept.
    MoveOnly,
    /// "Don't move or size with cells" (`absolute`): it stays where it is
    /// on the sheet. Its anchor is then always cell A1.
    Fixed,
}

impl Placement {
    pub const ALL: [Placement; 3] = [Placement::MoveAndSize, Placement::MoveOnly, Placement::Fixed];

    pub fn ooxml_name(self) -> &'static str {
        match self {
            Placement::MoveAndSize => "twoCell",
            Placement::MoveOnly => "oneCell",
            Placement::Fixed => "absolute",
        }
    }

    pub fn from_ooxml_name(name: &str) -> Option<Placement> {
        Placement::ALL.into_iter().find(|p| p.ooxml_name() == name)
    }
}

/// A picture or a shape.
#[derive(Clone, Debug, PartialEq)]
pub struct Drawing {
    /// Unique among the drawings of the workbook (assigned by
    /// `Workbook::add_drawing`); charts are numbered apart.
    pub id: u32,
    /// "Picture 1", "Rectangle 2": unique on the sheet.
    pub name: String,
    /// Description for screen readers (`descr`).
    pub alt_text: String,
    pub kind: DrawingKind,
    /// Cell holding the top-left corner (row, col).
    pub anchor: (u32, u32),
    /// Offset of the corner inside that cell, pixels at 100 % (x, y).
    pub offset: (f32, f32),
    /// Width and height, pixels at 100 %. For `Placement::MoveAndSize`
    /// this is the size when the drawing was last placed: the real one
    /// follows the cells, ask `Sheet::drawing_frame`.
    pub size: (f32, f32),
    /// Cell holding the bottom-right corner and the offset inside it:
    /// what pins a `MoveAndSize` drawing. Kept up to date for the other
    /// placements but not read.
    pub to: (u32, u32),
    pub to_offset: (f32, f32),
    pub placement: Placement,
    /// Degrees, clockwise, about the centre of the frame.
    pub rotation: f32,
    pub flip_h: bool,
    pub flip_v: bool,
    /// "Lock aspect ratio": resizing keeps the proportions.
    pub lock_aspect: bool,
    /// Cannot be changed while the sheet is protected without "Edit
    /// objects" (`fLocksWithSheet`).
    pub locked: bool,
    /// "Print object" (`fPrintsWithSheet`).
    pub print: bool,
    /// Place in the stacking order the sheet's charts, pictures and
    /// shapes share (`Sheet::object_stack`): a higher one is drawn over a
    /// lower one. 0: not placed yet, which is above all that are.
    pub z: u32,
    /// Hidden from the Selection pane (`hidden` of `cNvPr`): not drawn,
    /// not printed, not clickable.
    pub hidden: bool,
    /// Shadow, glow and soft edges (those of a group are not drawn).
    pub effects: Effects,
}

impl Drawing {
    fn new(kind: DrawingKind, size: (f32, f32), placement: Placement, lock_aspect: bool) -> Drawing {
        Drawing {
            id: 0,
            name: String::new(),
            alt_text: String::new(),
            kind,
            anchor: (0, 0),
            offset: (0.0, 0.0),
            size,
            to: (0, 0),
            to_offset: (0.0, 0.0),
            placement,
            rotation: 0.0,
            flip_h: false,
            flip_v: false,
            lock_aspect,
            locked: true,
            print: true,
            z: 0,
            hidden: false,
            effects: Effects::default(),
        }
    }

    /// A picture at its natural size, as Excel inserts it: it moves with
    /// its cell without being sized with it, and keeps its proportions.
    pub fn picture(picture: Picture) -> Drawing {
        let size = picture.natural_size();
        Drawing::new(DrawingKind::Picture(picture), size, Placement::MoveOnly, true)
    }

    /// A shape of the given size, moved and sized with its cells.
    pub fn shape(shape: Shape, size: (f32, f32)) -> Drawing {
        Drawing::new(DrawingKind::Shape(shape), size, Placement::MoveAndSize, false)
    }

    /// A SmartArt graphic of the given size, moved and sized with its
    /// cells.
    pub fn smartart(art: crate::smartart::SmartArt, size: (f32, f32)) -> Drawing {
        Drawing::new(DrawingKind::SmartArt(art), size, Placement::MoveAndSize, false)
    }

    /// A group of the given size holding `members`, whose `offset` and
    /// `size` say where each is in it (a file reader builds groups this
    /// way; on a sheet, `Workbook::group_drawings`).
    pub fn group(members: Vec<Drawing>, size: (f32, f32)) -> Drawing {
        let group = Group { members, extent: size };
        Drawing::new(DrawingKind::Group(group), size, Placement::MoveAndSize, false)
    }

    pub fn as_picture(&self) -> Option<&Picture> {
        match &self.kind {
            DrawingKind::Picture(picture) => Some(picture),
            _ => None,
        }
    }

    pub fn as_picture_mut(&mut self) -> Option<&mut Picture> {
        match &mut self.kind {
            DrawingKind::Picture(picture) => Some(picture),
            _ => None,
        }
    }

    pub fn as_shape(&self) -> Option<&Shape> {
        match &self.kind {
            DrawingKind::Shape(shape) => Some(shape),
            _ => None,
        }
    }

    pub fn as_shape_mut(&mut self) -> Option<&mut Shape> {
        match &mut self.kind {
            DrawingKind::Shape(shape) => Some(shape),
            _ => None,
        }
    }

    pub fn as_smartart(&self) -> Option<&crate::smartart::SmartArt> {
        match &self.kind {
            DrawingKind::SmartArt(art) => Some(art),
            _ => None,
        }
    }

    pub fn as_group(&self) -> Option<&Group> {
        match &self.kind {
            DrawingKind::Group(group) => Some(group),
            _ => None,
        }
    }

    pub fn as_smartart_mut(&mut self) -> Option<&mut crate::smartart::SmartArt> {
        match &mut self.kind {
            DrawingKind::SmartArt(art) => Some(art),
            _ => None,
        }
    }

    /// Where a member of this group is when the group is in `frame`,
    /// with the rotation and the flips the group adds to its own.
    fn member_leaf<'a>(&self, group: &Group, frame: Frame, turn: (f32, bool, bool), member: &'a Drawing) -> Leaf<'a> {
        let inside = group.member_frame(frame, member);
        let (width, height) = (inside.width, inside.height);
        // The middle of the member, from the middle of the group.
        let mut dx = inside.x - frame.x + width / 2.0 - frame.width / 2.0;
        let mut dy = inside.y - frame.y + height / 2.0 - frame.height / 2.0;
        let (mut rotation, mut flip_h, mut flip_v) = (member.rotation, member.flip_h, member.flip_v);
        // A flipped group mirrors where its members are and what they
        // show; a mirrored turn runs the other way.
        if turn.1 {
            dx = -dx;
            flip_h = !flip_h;
            rotation = -rotation;
        }
        if turn.2 {
            dy = -dy;
            flip_v = !flip_v;
            rotation = -rotation;
        }
        let (sin, cos) = (turn.0 as f64).to_radians().sin_cos();
        let (rx, ry) = (dx * cos - dy * sin, dx * sin + dy * cos);
        Leaf {
            drawing: member,
            frame: Frame {
                x: frame.x + frame.width / 2.0 + rx - width / 2.0,
                y: frame.y + frame.height / 2.0 + ry - height / 2.0,
                width,
                height,
            },
            rotation: (rotation + turn.0).rem_euclid(360.0),
            flip_h,
            flip_v,
        }
    }

    fn collect_leaves<'a>(&'a self, frame: Frame, turn: (f32, bool, bool), out: &mut Vec<Leaf<'a>>) {
        match &self.kind {
            DrawingKind::Group(group) => {
                for member in group.members.iter().filter(|member| !member.hidden) {
                    let leaf = self.member_leaf(group, frame, turn, member);
                    member.collect_leaves(leaf.frame, (leaf.rotation, leaf.flip_h, leaf.flip_v), out);
                }
            }
            _ => out.push(Leaf {
                drawing: self,
                frame,
                rotation: turn.0,
                flip_h: turn.1,
                flip_v: turn.2,
            }),
        }
    }

    /// The pictures and shapes this drawing shows when it is in `frame`,
    /// bottom-most first: itself, or the members of the group it is,
    /// each where the group puts it.
    pub fn leaves(&self, frame: Frame) -> Vec<Leaf<'_>> {
        self.leaves_turned(frame, (self.rotation, self.flip_h, self.flip_v))
    }

    /// `leaves` for a drawing turned and flipped (horizontally, then
    /// vertically) as `turn` says instead of as it is.
    pub fn leaves_turned(&self, frame: Frame, turn: (f32, bool, bool)) -> Vec<Leaf<'_>> {
        let mut out = Vec::new();
        self.collect_leaves(frame, turn, &mut out);
        out
    }

    /// For a group: give the places of its members in a group of the
    /// size `size` (they show where they did). A file may measure them
    /// in any unit; read this way, a group says the same thing however
    /// it was written.
    pub fn measure_members_in(&mut self, size: (f32, f32)) {
        let DrawingKind::Group(group) = &mut self.kind else {
            return;
        };
        let scale = |to: f32, from: f32| if from > 0.0 { to / from } else { 1.0 };
        let (sx, sy) = (scale(size.0, group.extent.0), scale(size.1, group.extent.1));
        group.extent = size;
        for member in &mut group.members {
            member.offset = (member.offset.0 * sx, member.offset.1 * sy);
            member.size = (member.size.0 * sx, member.size.1 * sy);
            member.measure_members_in(member.size);
        }
    }

    /// Work out again the theme colours of the drawing after the theme
    /// of its workbook changed: those of a shape, of the members of a
    /// group, of the effects and of a recolouring.
    pub fn retheme(&mut self, theme: &crate::theme::Theme) {
        if let Some(shadow) = &mut self.effects.shadow {
            shadow.color = theme.resolve(shadow.color);
        }
        if let Some(glow) = &mut self.effects.glow {
            glow.color = theme.resolve(glow.color);
        }
        match &mut self.kind {
            DrawingKind::Shape(shape) => theme.resolve_shape(shape),
            DrawingKind::Picture(picture) => {
                if let Recolor::Duotone(dark, light) = &mut picture.adjust.recolor {
                    (*dark, *light) = (theme.resolve(*dark), theme.resolve(*light));
                }
                // A vector picture painted in a colour of the theme: its
                // document is repainted. (The image kept beside it is
                // made again by whoever can draw it.)
                if let Some(document) = picture.svg.as_deref().and_then(|document| crate::svg::rethemed(document, theme)) {
                    picture.svg = Some(document.into());
                }
            }
            // A graphic takes its colours from the theme when it is laid
            // out.
            DrawingKind::SmartArt(_) => {}
            DrawingKind::Group(group) => {
                for member in &mut group.members {
                    member.retheme(theme);
                }
            }
        }
    }

    /// How many objects the drawing is in a file: itself, and for a
    /// group every member, the groups among them counted the same way.
    pub fn object_count(&self) -> usize {
        match &self.kind {
            DrawingKind::Group(group) => 1 + group.members.iter().map(Drawing::object_count).sum::<usize>(),
            DrawingKind::SmartArt(art) => art.object_count(),
            _ => 1,
        }
    }

    /// The largest id in the drawing: its own, or one of its members'.
    fn max_id(&self) -> u32 {
        let members = self.as_group().map(|group| group.members.iter().map(Drawing::max_id).max().unwrap_or(0));
        self.id.max(members.unwrap_or(0))
    }

    /// Give the drawing and the members of a group ids from `next` on.
    fn renumber(&mut self, next: &mut u32) {
        self.id = *next;
        *next += 1;
        if let DrawingKind::Group(group) = &mut self.kind {
            for member in &mut group.members {
                member.renumber(next);
            }
        }
    }

    /// A straight line (it is hit along its length, not in its frame).
    fn is_line(&self) -> bool {
        self.as_shape().is_some_and(|shape| shape.preset.is_line())
    }
}

/// Where a drawing is on its sheet: pixels at 100 % from the top-left
/// corner of cell A1, before rotation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Frame {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Frame {
        Frame { x, y, width, height }
    }
}

/// A corner of a frame as files give it: the cell (row, col) holding it
/// and its offset inside that cell (x, y), in pixels at 100 %.
pub type Corner = ((u32, u32), (f32, f32));

/// Moves along the stacking order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ZOrder {
    BringToFront,
    SendToBack,
    BringForward,
    SendBackward,
}

/// A floating object of a sheet, whichever list holds it: charts and
/// drawings are numbered apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ObjectRef {
    Chart(u32),
    Drawing(u32),
}

static NO_CELLS: BTreeSet<u32> = BTreeSet::new();

/// The columns (or the rows) of a sheet as a run of sizes in pixels.
pub(crate) struct Track<'a> {
    default: f64,
    sizes: &'a BTreeMap<u32, f32>,
    /// Cells of no size: hidden by hand, or by a filter.
    hidden: [&'a BTreeSet<u32>; 2],
    count: u32,
}

impl Track<'_> {
    fn is_hidden(&self, index: u32) -> bool {
        self.hidden.iter().any(|set| set.contains(&index))
    }

    fn own_size(&self, index: u32) -> f64 {
        self.sizes.get(&index).map_or(self.default, |s| (*s as f64).max(0.0))
    }

    fn size(&self, index: u32) -> f64 {
        if self.is_hidden(index) {
            0.0
        } else {
            self.own_size(index)
        }
    }

    /// Pixels from the leading edge of cell `a` to that of cell `b`.
    pub(crate) fn span(&self, a: u32, b: u32) -> f64 {
        if b < a {
            return -self.span(b, a);
        }
        let mut total = (b - a) as f64 * self.default;
        total += self
            .sizes
            .range(a..b)
            .map(|(_, s)| (*s as f64).max(0.0) - self.default)
            .sum::<f64>();
        total -= self.hidden[0].range(a..b).map(|i| self.own_size(*i)).sum::<f64>();
        total -= self.hidden[1]
            .range(a..b)
            .filter(|i| !self.hidden[0].contains(i))
            .map(|i| self.own_size(*i))
            .sum::<f64>();
        total
    }

    /// Where a corner pinned to a cell is, from the edge of the sheet. An
    /// offset larger than its cell (it was narrowed since) stops at the
    /// far edge of the cell.
    pub(crate) fn position(&self, index: u32, offset: f32) -> f64 {
        let index = index.min(self.count - 1);
        self.span(0, index) + (offset as f64).clamp(0.0, self.size(index))
    }

    /// The cell holding the point `distance` pixels from the edge of the
    /// sheet, and how far into the cell it lies. A point on the line
    /// between two cells belongs to the second.
    pub(crate) fn locate(&self, distance: f64) -> (u32, f32) {
        let last = self.count - 1;
        let mut index = 0;
        let mut rest = if distance.is_finite() { distance.max(0.0) } else { 0.0 };
        while index < last {
            // Up to the next cell with a size of its own, all have the
            // default size: step over them at once.
            let special = [
                self.sizes.range(index..).next().map(|(i, _)| *i),
                self.hidden[0].range(index..).next().copied(),
                self.hidden[1].range(index..).next().copied(),
            ]
            .into_iter()
            .flatten()
            .min()
            .map_or(last, |i| i.min(last));
            if special > index {
                let run = (special - index) as f64 * self.default;
                if self.default > 0.0 && rest < run {
                    let whole = (rest / self.default).floor();
                    return (index + whole as u32, (rest - whole * self.default) as f32);
                }
                rest -= run;
                index = special;
            } else {
                let size = self.size(index);
                if rest < size {
                    break;
                }
                rest -= size;
                index += 1;
            }
        }
        (index, rest.min(self.size(index).max(0.0)) as f32)
    }

    /// A corner given as a cell and an offset, moved to the cell it
    /// really falls in when the offset runs past its cell. Corners in
    /// hidden cells are left alone: they come back with their cell.
    fn normalised(&self, index: u32, offset: f32) -> (u32, f32) {
        let index = index.min(self.count - 1);
        let offset = if offset.is_finite() { offset } else { 0.0 };
        if offset >= 0.0 && (self.is_hidden(index) || (offset as f64) < self.size(index).max(f64::MIN_POSITIVE)) {
            (index, offset)
        } else {
            self.locate(self.span(0, index) + offset as f64)
        }
    }
}

pub(crate) fn columns(sheet: &Sheet) -> Track<'_> {
    Track {
        default: sheet.default_col_width as f64,
        sizes: &sheet.col_widths,
        hidden: [&sheet.hidden_cols, &NO_CELLS],
        count: MAX_COLS,
    }
}

pub(crate) fn rows(sheet: &Sheet) -> Track<'_> {
    Track {
        default: sheet.default_row_height as f64,
        sizes: &sheet.row_heights,
        hidden: [&sheet.hidden_rows, &sheet.filtered_rows],
        count: MAX_ROWS,
    }
}

impl Sheet {
    /// The pictures and shapes a drawing shows, each where it is on the
    /// sheet now (see `Drawing::leaves`).
    pub fn drawing_leaves<'a>(&self, drawing: &'a Drawing) -> Vec<Leaf<'a>> {
        drawing.leaves(self.drawing_frame(drawing))
    }

    /// Where a drawing is on the sheet now, given the widths of its
    /// columns and the heights of its rows.
    pub fn drawing_frame(&self, drawing: &Drawing) -> Frame {
        let (cols, rows) = (columns(self), rows(self));
        let (x, y) = if drawing.placement == Placement::Fixed {
            // Not tied to its cell: the offsets are not held inside it.
            let x = cols.span(0, drawing.anchor.1.min(MAX_COLS - 1)) + drawing.offset.0 as f64;
            let y = rows.span(0, drawing.anchor.0.min(MAX_ROWS - 1)) + drawing.offset.1 as f64;
            (x.max(0.0), y.max(0.0))
        } else {
            (
                cols.position(drawing.anchor.1, drawing.offset.0),
                rows.position(drawing.anchor.0, drawing.offset.1),
            )
        };
        let (width, height) = if drawing.placement == Placement::MoveAndSize {
            let right = cols.position(drawing.to.1, drawing.to_offset.0);
            let bottom = rows.position(drawing.to.0, drawing.to_offset.1);
            ((right - x).max(0.0), (bottom - y).max(0.0))
        } else {
            (drawing.size.0.max(0.0) as f64, drawing.size.1.max(0.0) as f64)
        };
        Frame { x, y, width, height }
    }

    /// Bring the placement of a drawing in order after its `anchor`,
    /// `offset`, `size` or `placement` were set by hand: offsets are
    /// brought back inside their cells, a `Fixed` drawing is anchored to
    /// A1 and the bottom-right corner is computed from the size. (The
    /// functions of `Workbook` do it themselves; a file reader calls it
    /// once the column widths and row heights of the sheet are known,
    /// unless it has both corners: then `place_drawing`.)
    pub fn settle_drawing(&self, drawing: &mut Drawing) {
        let (cols, rows) = (columns(self), rows(self));
        let clean = |v: f32| if v.is_finite() { v.max(0.0) } else { 0.0 };
        drawing.size = (clean(drawing.size.0), clean(drawing.size.1));
        if drawing.placement == Placement::Fixed {
            let frame = self.drawing_frame(drawing);
            drawing.anchor = (0, 0);
            drawing.offset = (frame.x as f32, frame.y as f32);
        } else {
            let (col, dx) = cols.normalised(drawing.anchor.1, drawing.offset.0);
            let (row, dy) = rows.normalised(drawing.anchor.0, drawing.offset.1);
            drawing.anchor = (row, col);
            drawing.offset = (dx, dy);
        }
        let frame = Frame {
            width: drawing.size.0 as f64,
            height: drawing.size.1 as f64,
            ..self.drawing_frame(drawing)
        };
        let (col, dx) = cols.locate(frame.x + frame.width);
        let (row, dy) = rows.locate(frame.y + frame.height);
        drawing.to = (row, col);
        drawing.to_offset = (dx, dy);
    }

    /// Place a drawing between two corners: the `from` and `to` of an
    /// anchor in a file. The size follows from the sheet's columns and
    /// rows.
    pub fn place_drawing(&self, drawing: &mut Drawing, from: Corner, to: Corner) {
        (drawing.anchor, drawing.offset) = from;
        (drawing.to, drawing.to_offset) = to;
        let (cols, rows) = (columns(self), rows(self));
        let width = cols.position(to.0.1, to.1.0) - cols.position(from.0.1, from.1.0);
        let height = rows.position(to.0.0, to.1.1) - rows.position(from.0.0, from.1.1);
        drawing.size = (width.max(0.0) as f32, height.max(0.0) as f32);
        if drawing.placement != Placement::MoveAndSize {
            self.settle_drawing(drawing);
        }
    }

    /// The two corners of a drawing as a file writes them (`from` and
    /// `to`), computed from where the drawing is now.
    pub fn drawing_corners(&self, drawing: &Drawing) -> (Corner, Corner) {
        let (cols, rows) = (columns(self), rows(self));
        let frame = self.drawing_frame(drawing);
        let (c0, dx0) = cols.locate(frame.x);
        let (r0, dy0) = rows.locate(frame.y);
        let (c1, dx1) = cols.locate(frame.x + frame.width);
        let (r1, dy1) = rows.locate(frame.y + frame.height);
        (((r0, c0), (dx0, dy0)), ((r1, c1), (dx1, dy1)))
    }

    /// The cell holding a point of the sheet (pixels at 100 % from the
    /// corner of A1) and the offset of the point inside it. A screen
    /// places a frame through the cells of its corners, so that its edges
    /// fall on the cell edges at any zoom.
    pub fn corner_at(&self, x: f64, y: f64) -> Corner {
        let (col, dx) = columns(self).locate(x);
        let (row, dy) = rows(self).locate(y);
        ((row, col), (dx, dy))
    }

    /// Where a corner is on the sheet: the reverse of `corner_at`. An
    /// offset that runs past its cell is taken as it is (a point being
    /// dragged over the sheet is not held back by the cell it started in).
    pub fn corner_point(&self, corner: Corner) -> (f64, f64) {
        let ((row, col), (dx, dy)) = corner;
        (
            columns(self).span(0, col.min(MAX_COLS - 1)) + dx as f64,
            rows(self).span(0, row.min(MAX_ROWS - 1)) + dy as f64,
        )
    }

    /// A name no chart, picture or shape of the sheet has yet:
    /// "Rectangle 3".
    pub(crate) fn drawing_name(&self, base: &str) -> String {
        let mut number = self.drawings.len() + self.charts.len() + 1;
        loop {
            let name = format!("{base} {number}");
            let taken = self.drawings.iter().any(|d| d.name.eq_ignore_ascii_case(&name))
                || self.charts.iter().any(|c| c.name.eq_ignore_ascii_case(&name));
            if !taken {
                return name;
            }
            number += 1;
        }
    }

    /// Every chart, picture and shape of the sheet, bottom-most first.
    /// Those that were given a place (`z`) come in that order; the others
    /// lie above them, charts first, each list in its own order.
    pub fn object_stack(&self) -> Vec<ObjectRef> {
        let charts = self.charts.iter().map(|chart| (chart.z, ObjectRef::Chart(chart.id)));
        let drawings = self.drawings.iter().map(|drawing| (drawing.z, ObjectRef::Drawing(drawing.id)));
        let mut all: Vec<(u32, ObjectRef)> = charts.chain(drawings).collect();
        // Stable: equal places keep the order above.
        all.sort_by_key(|(z, _)| (*z == 0, *z));
        all.into_iter().map(|(_, object)| object).collect()
    }

    /// Give every object its place in `order` (objects it does not name
    /// keep their order, above the others), and bring the two lists in
    /// that order too.
    pub fn set_object_stack(&mut self, order: &[ObjectRef]) {
        let mut stack: Vec<ObjectRef> = order.to_vec();
        for object in self.object_stack() {
            if !stack.contains(&object) {
                stack.push(object);
            }
        }
        let place = |object: ObjectRef| stack.iter().position(|o| *o == object).map_or(0, |i| i as u32 + 1);
        for chart in &mut self.charts {
            chart.z = place(ObjectRef::Chart(chart.id));
        }
        for drawing in &mut self.drawings {
            drawing.z = place(ObjectRef::Drawing(drawing.id));
        }
        self.charts.sort_by_key(|chart| chart.z);
        self.drawings.sort_by_key(|drawing| drawing.z);
    }

    /// The place of an object that goes on top of all the others.
    pub(crate) fn top_z(&mut self) -> u32 {
        // Placing the ones that are not yet keeps them under the new one.
        if self.charts.iter().any(|c| c.z == 0) || self.drawings.iter().any(|d| d.z == 0) {
            self.set_object_stack(&[]);
        }
        (self.charts.len() + self.drawings.len()) as u32 + 1
    }
}

impl Workbook {
    fn next_drawing_id(&self) -> u32 {
        self.sheets
            .iter()
            .flat_map(|s| s.drawings.iter().map(Drawing::max_id))
            .max()
            .unwrap_or(0)
            + 1
    }

    /// Add a drawing on top of the others; returns its id. It gets a name
    /// when it has none and its placement is settled. Recorded for undo,
    /// like every function below that changes a drawing.
    pub fn add_drawing(&mut self, sheet: usize, mut drawing: Drawing) -> Option<u32> {
        if sheet >= self.sheets.len() {
            return None;
        }
        self.snapshot();
        // The members of a group get ids of their own too.
        let mut next = self.next_drawing_id();
        drawing.renumber(&mut next);
        if let DrawingKind::Shape(shape) = &mut drawing.kind {
            self.theme().resolve_shape(shape);
        }
        let sh = &mut self.sheets[sheet];
        if drawing.name.is_empty() {
            let base = match &drawing.kind {
                DrawingKind::Picture(picture) if picture.svg.is_some() => graphic_name(self.locale),
                DrawingKind::Picture(_) => picture_name(self.locale),
                DrawingKind::SmartArt(_) => crate::smartart::name(self.locale),
                DrawingKind::Group(_) => group_name(self.locale),
                DrawingKind::Shape(shape) => {
                    let tool = ShapeTool::ALL.into_iter().find(|tool| {
                        let model = tool.shape();
                        let ends = |s: &Shape| s.outline.map(|o| (o.start, o.end));
                        model.preset == shape.preset && model.text_box == shape.text_box && ends(&model) == ends(shape)
                    });
                    let fallback = ShapeTool::ALL.into_iter().find(|tool| tool.preset() == shape.preset);
                    tool.or(fallback).unwrap_or(ShapeTool::Rectangle).name(self.locale)
                }
            };
            drawing.name = sh.drawing_name(base);
        }
        sh.settle_drawing(&mut drawing);
        drawing.z = sh.top_z();
        let id = drawing.id;
        sh.drawings.push(drawing);
        Some(id)
    }

    /// Insert an image file with its top-left corner on a cell (row,
    /// col), at its natural size. `None` when the bytes are not a PNG,
    /// JPEG, GIF or BMP image.
    pub fn add_picture(&mut self, sheet: usize, data: Vec<u8>, at: (u32, u32)) -> Option<u32> {
        let mut drawing = Drawing::picture(Picture::new(data)?);
        drawing.anchor = at;
        self.add_drawing(sheet, drawing)
    }

    /// Insert a shape of the gallery in a frame (use
    /// `ShapeTool::default_size` after a plain click).
    pub fn add_shape(&mut self, sheet: usize, tool: ShapeTool, frame: Frame) -> Option<u32> {
        let mut drawing = Drawing::shape(tool.shape(), (frame.width as f32, frame.height as f32));
        drawing.name = self.sheets.get(sheet)?.drawing_name(tool.name(self.locale));
        // A frame is a place on the sheet: anchored to A1 the offsets are
        // its coordinates, and settling finds the cell.
        drawing.offset = (frame.x as f32, frame.y as f32);
        self.add_drawing(sheet, drawing)
    }

    pub fn remove_drawing(&mut self, sheet: usize, id: u32) -> bool {
        if self.drawing(sheet, id).is_none() {
            return false;
        }
        self.snapshot();
        self.sheets[sheet].drawings.retain(|d| d.id != id);
        true
    }

    pub fn drawing(&self, sheet: usize, id: u32) -> Option<&Drawing> {
        self.sheets.get(sheet)?.drawings.iter().find(|d| d.id == id)
    }

    /// Where a drawing is on its sheet.
    pub fn drawing_frame(&self, sheet: usize, id: u32) -> Option<Frame> {
        let sh = self.sheets.get(sheet)?;
        sh.drawings.iter().find(|d| d.id == id).map(|d| sh.drawing_frame(d))
    }

    /// Change a drawing: its look, text, name, rotation, placement, or
    /// its `anchor`, `offset` and `size`. `size` holds the current size
    /// when `f` is called and the placement is settled afterwards.
    pub fn update_drawing(&mut self, sheet: usize, id: u32, f: impl FnOnce(&mut Drawing)) -> bool {
        let Some(index) = self
            .sheets
            .get(sheet)
            .and_then(|sh| sh.drawings.iter().position(|d| d.id == id))
        else {
            return false;
        };
        self.snapshot();
        let sh = &mut self.sheets[sheet];
        let mut drawing = sh.drawings.remove(index);
        let frame = sh.drawing_frame(&drawing);
        drawing.size = (frame.width as f32, frame.height as f32);
        f(&mut drawing);
        drawing.id = id;
        sh.settle_drawing(&mut drawing);
        sh.drawings.insert(index, drawing);
        true
    }

    /// Hold several drawings of a sheet together as one group, which
    /// takes the place of the topmost of them in the stacking order.
    /// Returns its id; `None` unless at least two of `ids` are drawings
    /// of the sheet.
    pub fn group_drawings(&mut self, sheet: usize, ids: &[u32]) -> Option<u32> {
        let sh = self.sheets.get(sheet)?;
        let members: Vec<&Drawing> = sh.drawings.iter().filter(|d| ids.contains(&d.id)).collect();
        if members.len() < 2 {
            return None;
        }
        let frames: Vec<Frame> = members.iter().map(|d| sh.drawing_frame(d)).collect();
        let left = frames.iter().map(|f| f.x).fold(f64::INFINITY, f64::min);
        let top = frames.iter().map(|f| f.y).fold(f64::INFINITY, f64::min);
        let right = frames.iter().map(|f| f.x + f.width).fold(f64::NEG_INFINITY, f64::max);
        let bottom = frames.iter().map(|f| f.y + f.height).fold(f64::NEG_INFINITY, f64::max);
        let extent = ((right - left) as f32, (bottom - top) as f32);
        let held: Vec<Drawing> = members
            .iter()
            .zip(&frames)
            .map(|(member, frame)| Drawing {
                anchor: (0, 0),
                offset: ((frame.x - left) as f32, (frame.y - top) as f32),
                size: (frame.width as f32, frame.height as f32),
                to: (0, 0),
                to_offset: (0.0, 0.0),
                z: 0,
                ..(*member).clone()
            })
            .collect();
        let stack = sh.object_stack();
        let topmost = members.iter().filter_map(|d| stack.iter().position(|o| *o == ObjectRef::Drawing(d.id))).max()?;
        let mut group = Drawing::group(held, extent);
        group.offset = (left as f32, top as f32);
        group.name = sh.drawing_name(group_name(self.locale));
        group.locked = members.iter().any(|d| d.locked);
        group.print = members.iter().any(|d| d.print);
        self.snapshot();
        let id = self.next_drawing_id();
        group.id = id;
        let sh = &mut self.sheets[sheet];
        sh.settle_drawing(&mut group);
        let mut order: Vec<ObjectRef> = Vec::new();
        for (i, object) in stack.iter().enumerate() {
            let member = matches!(object, ObjectRef::Drawing(d) if ids.contains(d));
            if !member {
                order.push(*object);
            }
            if i == topmost {
                order.push(ObjectRef::Drawing(id));
            }
        }
        sh.drawings.retain(|d| !ids.contains(&d.id));
        sh.drawings.push(group);
        sh.set_object_stack(&order);
        Some(id)
    }

    /// Let the members of a group go: each becomes a drawing of the
    /// sheet where the group showed it, in the place the group had in
    /// the stacking order. Returns their ids, bottom-most first (empty
    /// when `id` is not a group).
    pub fn ungroup_drawing(&mut self, sheet: usize, id: u32) -> Vec<u32> {
        let Some(sh) = self.sheets.get(sheet) else {
            return Vec::new();
        };
        let Some(holder) = sh.drawings.iter().find(|d| d.id == id) else {
            return Vec::new();
        };
        let Some(group) = holder.as_group() else {
            return Vec::new();
        };
        let frame = sh.drawing_frame(holder);
        let turn = (holder.rotation, holder.flip_h, holder.flip_v);
        let mut freed: Vec<Drawing> = Vec::new();
        for member in &group.members {
            let leaf = holder.member_leaf(group, frame, turn, member);
            let mut drawing = member.clone();
            drawing.anchor = (0, 0);
            drawing.offset = (leaf.frame.x.max(0.0) as f32, leaf.frame.y.max(0.0) as f32);
            drawing.size = (leaf.frame.width as f32, leaf.frame.height as f32);
            drawing.rotation = leaf.rotation;
            drawing.flip_h = leaf.flip_h;
            drawing.flip_v = leaf.flip_v;
            drawing.placement = if holder.placement == Placement::Fixed { Placement::Fixed } else { drawing.placement };
            // A group of its own keeps the places of its members: they
            // are measured in its own extent.
            freed.push(drawing);
        }
        let ids: Vec<u32> = freed.iter().map(|d| d.id).collect();
        let stack = sh.object_stack();
        self.snapshot();
        let sh = &mut self.sheets[sheet];
        sh.drawings.retain(|d| d.id != id);
        for mut drawing in freed {
            let taken = |name: &str| {
                name.is_empty()
                    || sh.drawings.iter().any(|d| d.name.eq_ignore_ascii_case(name))
                    || sh.charts.iter().any(|c| c.name.eq_ignore_ascii_case(name))
            };
            if taken(&drawing.name) {
                let base = drawing.name.trim_end_matches(|c: char| c.is_ascii_digit()).trim_end().to_string();
                drawing.name = sh.drawing_name(if base.is_empty() { "Shape" } else { &base });
            }
            sh.settle_drawing(&mut drawing);
            sh.drawings.push(drawing);
        }
        let mut order: Vec<ObjectRef> = Vec::new();
        for object in stack {
            if object == ObjectRef::Drawing(id) {
                order.extend(ids.iter().map(|member| ObjectRef::Drawing(*member)));
            } else {
                order.push(object);
            }
        }
        sh.set_object_stack(&order);
        ids
    }

    /// Move and / or resize a drawing to a frame on the sheet.
    pub fn set_drawing_frame(&mut self, sheet: usize, id: u32, frame: Frame) -> bool {
        self.update_drawing(sheet, id, |drawing| {
            drawing.anchor = (0, 0);
            drawing.offset = (frame.x.max(0.0) as f32, frame.y.max(0.0) as f32);
            drawing.size = (frame.width as f32, frame.height as f32);
        })
    }

    /// Copy a drawing next to itself (Duplicate); returns the id of the
    /// copy, which is on top.
    pub fn duplicate_drawing(&mut self, sheet: usize, id: u32) -> Option<u32> {
        let sh = self.sheets.get(sheet)?;
        let original = sh.drawings.iter().find(|d| d.id == id)?;
        let frame = sh.drawing_frame(original);
        let mut copy = original.clone();
        let base = copy
            .name
            .trim_end_matches(|c: char| c.is_ascii_digit())
            .trim_end()
            .to_string();
        copy.name = if base.is_empty() {
            String::new()
        } else {
            sh.drawing_name(&base)
        };
        copy.anchor = (0, 0);
        copy.offset = ((frame.x + 16.0) as f32, (frame.y + 16.0) as f32);
        copy.size = (frame.width as f32, frame.height as f32);
        self.add_drawing(sheet, copy)
    }

    /// Move a drawing in the stacking order of its sheet. Returns false
    /// when it is already there.
    pub fn reorder_drawing(&mut self, sheet: usize, id: u32, order: ZOrder) -> bool {
        self.reorder_objects(sheet, &[ObjectRef::Drawing(id)], order)
    }

    /// The charts, pictures and shapes of a sheet, bottom-most first.
    pub fn object_stack(&self, sheet: usize) -> Vec<ObjectRef> {
        self.sheets.get(sheet).map(Sheet::object_stack).unwrap_or_default()
    }

    /// Move objects in the stacking order their sheet's charts, pictures
    /// and shapes share: to the front or the back (keeping their order
    /// among themselves), or one step past the next object that is not
    /// moving. Returns false when nothing moves.
    pub fn reorder_objects(&mut self, sheet: usize, objects: &[ObjectRef], order: ZOrder) -> bool {
        let Some(sh) = self.sheets.get(sheet) else {
            return false;
        };
        let before = sh.object_stack();
        let moving = |object: &ObjectRef| objects.contains(object);
        let mut after = before.clone();
        match order {
            ZOrder::BringToFront | ZOrder::SendToBack => {
                let (mut moved, rest): (Vec<ObjectRef>, Vec<ObjectRef>) = before.iter().partition(|o| moving(o));
                if order == ZOrder::BringToFront {
                    after = rest;
                    after.append(&mut moved);
                } else {
                    moved.extend(rest);
                    after = moved;
                }
            }
            ZOrder::BringForward => {
                // From the top down, so that two neighbours both move.
                for i in (0..after.len().saturating_sub(1)).rev() {
                    if moving(&after[i]) && !moving(&after[i + 1]) {
                        after.swap(i, i + 1);
                    }
                }
            }
            ZOrder::SendBackward => {
                for i in 1..after.len() {
                    if moving(&after[i]) && !moving(&after[i - 1]) {
                        after.swap(i, i - 1);
                    }
                }
            }
        }
        if after == before {
            return false;
        }
        self.snapshot();
        self.sheets[sheet].set_object_stack(&after);
        true
    }

    /// The name of a chart, picture or shape, as the Selection pane
    /// lists it.
    pub fn object_name(&self, sheet: usize, object: ObjectRef) -> Option<&str> {
        match object {
            ObjectRef::Chart(id) => self.chart(sheet, id).map(|chart| chart.name.as_str()),
            ObjectRef::Drawing(id) => self.drawing(sheet, id).map(|drawing| drawing.name.as_str()),
        }
    }

    pub fn object_hidden(&self, sheet: usize, object: ObjectRef) -> Option<bool> {
        match object {
            ObjectRef::Chart(id) => self.chart(sheet, id).map(|chart| chart.hidden),
            ObjectRef::Drawing(id) => self.drawing(sheet, id).map(|drawing| drawing.hidden),
        }
    }

    /// Rename an object (Excel lets two objects share a name). Returns
    /// false when the name is empty or already its name.
    pub fn rename_object(&mut self, sheet: usize, object: ObjectRef, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() || self.object_name(sheet, object).is_none_or(|current| current == name) {
            return false;
        }
        match object {
            ObjectRef::Chart(id) => self.update_chart(sheet, id, |chart| chart.name = name.to_string()),
            ObjectRef::Drawing(id) => self.update_drawing(sheet, id, |drawing| drawing.name = name.to_string()),
        }
    }

    /// Show or hide objects. Returns false when they all are as asked.
    pub fn set_objects_hidden(&mut self, sheet: usize, objects: &[ObjectRef], hidden: bool) -> bool {
        let mut changed = false;
        for object in objects {
            if self.object_hidden(sheet, *object) != Some(!hidden) {
                continue;
            }
            changed |= match *object {
                ObjectRef::Chart(id) => self.update_chart(sheet, id, |chart| chart.hidden = hidden),
                ObjectRef::Drawing(id) => self.update_drawing(sheet, id, |drawing| drawing.hidden = hidden),
            };
        }
        changed
    }

    /// Put the objects of a sheet in a given order, bottom-most first
    /// (the Selection pane, by dragging). Returns false when that is the
    /// order they have.
    pub fn restack_objects(&mut self, sheet: usize, order: &[ObjectRef]) -> bool {
        let Some(sh) = self.sheets.get(sheet) else {
            return false;
        };
        let mut wanted: Vec<ObjectRef> = order.to_vec();
        let before = sh.object_stack();
        wanted.retain(|object| before.contains(object));
        for object in &before {
            if !wanted.contains(object) {
                wanted.push(*object);
            }
        }
        if wanted == before {
            return false;
        }
        self.snapshot();
        self.sheets[sheet].set_object_stack(&wanted);
        true
    }

    /// The topmost drawing under a point of the sheet (pixels at 100 %
    /// from the corner of A1). Rotation is taken into account; a line is
    /// hit within a few pixels of it, anything else anywhere in its
    /// frame.
    pub fn drawing_at(&self, sheet: usize, x: f64, y: f64) -> Option<u32> {
        let sh = self.sheets.get(sheet)?;
        // How close to a line a click must be.
        const REACH: f64 = 4.0;
        sh.drawings.iter().rev().filter(|drawing| !drawing.hidden).find_map(|drawing| {
            let frame = sh.drawing_frame(drawing);
            // The point as the unrotated drawing sees it.
            let (cx, cy) = (frame.x + frame.width / 2.0, frame.y + frame.height / 2.0);
            let (sin, cos) = (-(drawing.rotation as f64)).to_radians().sin_cos();
            let (dx, dy) = (x - cx, y - cy);
            let (px, py) = (cx + dx * cos - dy * sin, cy + dx * sin + dy * cos);
            let hit = if drawing.is_line() {
                let (mut x0, mut x1) = (frame.x, frame.x + frame.width);
                let (mut y0, mut y1) = (frame.y, frame.y + frame.height);
                if drawing.flip_h {
                    (x0, x1) = (x1, x0);
                }
                if drawing.flip_v {
                    (y0, y1) = (y1, y0);
                }
                let (lx, ly) = (x1 - x0, y1 - y0);
                let length = lx * lx + ly * ly;
                let along = if length > 0.0 {
                    (((px - x0) * lx + (py - y0) * ly) / length).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                (px - x0 - along * lx).hypot(py - y0 - along * ly) <= REACH
            } else {
                px >= frame.x && px <= frame.x + frame.width && py >= frame.y && py <= frame.y + frame.height
            };
            hit.then_some(drawing.id)
        })
    }
}

/// Where a corner pinned to cell `position` goes when rows or columns are
/// inserted or deleted. `far` is the bottom-right corner of a drawing: one
/// resting exactly on the line where cells are inserted stays before them.
pub(crate) fn moved_corner(
    position: u32,
    offset: f32,
    far: bool,
    insert: bool,
    at: u32,
    count: u32,
    max: u32,
) -> (u32, f32) {
    if insert {
        let before = position < at || (far && position == at && offset <= 0.0);
        if before {
            (position, offset)
        } else {
            ((position + count).min(max - 1), offset)
        }
    } else if position >= at + count {
        (position - count, offset)
    } else if position >= at {
        // The cell is gone: the corner lands where the deleted cells were.
        (at, 0.0)
    } else {
        (position, offset)
    }
}

/// See `features::on_structural_edit`: drawings follow their cells like
/// charts do, and those sized with their cells grow or shrink with the
/// rows and columns inserted or deleted under them.
pub(crate) fn on_structural_edit(sheet: &mut Sheet, edit: StructuralEdit, edited_sheet: &str) {
    if sheet.drawings.is_empty() || !sheet.name.eq_ignore_ascii_case(edited_sheet) {
        return;
    }
    let (row_axis, insert, at, count) = match edit {
        StructuralEdit::InsertRows { at, count } => (true, true, at, count),
        StructuralEdit::DeleteRows { at, count } => (true, false, at, count),
        StructuralEdit::InsertCols { at, count } => (false, true, at, count),
        StructuralEdit::DeleteCols { at, count } => (false, false, at, count),
    };
    let mut drawings = std::mem::take(&mut sheet.drawings);
    for drawing in drawings.iter_mut().filter(|d| d.placement != Placement::Fixed) {
        if row_axis {
            (drawing.anchor.0, drawing.offset.1) =
                moved_corner(drawing.anchor.0, drawing.offset.1, false, insert, at, count, MAX_ROWS);
            (drawing.to.0, drawing.to_offset.1) =
                moved_corner(drawing.to.0, drawing.to_offset.1, true, insert, at, count, MAX_ROWS);
        } else {
            (drawing.anchor.1, drawing.offset.0) =
                moved_corner(drawing.anchor.1, drawing.offset.0, false, insert, at, count, MAX_COLS);
            (drawing.to.1, drawing.to_offset.0) =
                moved_corner(drawing.to.1, drawing.to_offset.0, true, insert, at, count, MAX_COLS);
        }
        if drawing.placement == Placement::MoveAndSize {
            let frame = sheet.drawing_frame(drawing);
            drawing.size = (frame.width as f32, frame.height as f32);
        } else {
            sheet.settle_drawing(drawing);
        }
    }
    sheet.drawings = drawings;
}

/// See `features::on_sheet_renamed`: drawings name no sheet.
pub(crate) fn on_sheet_renamed(_sheet: &mut Sheet, _old: &str, _new: &str) {}

/// See `features::on_sheet_duplicated`: the drawings of the copy get ids
/// of their own (they share the bytes of their pictures).
pub(crate) fn on_sheet_duplicated(wb: &mut Workbook, copy: usize, _original: &str) {
    let mut next = wb.next_drawing_id();
    for drawing in wb.sheets[copy].drawings.iter_mut() {
        drawing.renumber(&mut next);
    }
}

/// See `features::on_sheet_deleted`: the drawings of a sheet go with it,
/// and those of the other sheets name no sheet.
pub(crate) fn on_sheet_deleted(_wb: &mut Workbook, _deleted: &Sheet, _order: &[&str]) {}

/// See `features::on_sheet_moved`: drawings are kept on their sheet.
pub(crate) fn on_sheet_moved(_wb: &mut Workbook, _moved: &str, _before: &[&str], _after: &[&str]) {}

/// See `features::on_cells_shifted`. Drawings are pinned to the rows and
/// the columns of the grid, which stay where they are when only some of
/// their cells are shifted.
pub(crate) fn on_cells_shifted(_wb: &mut Workbook, _sheet: usize, _shift: CellShift) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::wb;

    /// The first bytes of a PNG file of the given size and resolution.
    fn png(width: u32, height: u32, per_metre: Option<u32>) -> Vec<u8> {
        let mut data = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut chunk = |kind: &[u8; 4], body: &[u8]| {
            data.extend_from_slice(&(body.len() as u32).to_be_bytes());
            data.extend_from_slice(kind);
            data.extend_from_slice(body);
            data.extend_from_slice(&[0; 4]);
        };
        let mut header = Vec::new();
        header.extend_from_slice(&width.to_be_bytes());
        header.extend_from_slice(&height.to_be_bytes());
        header.extend_from_slice(&[8, 6, 0, 0, 0]);
        chunk(b"IHDR", &header);
        if let Some(density) = per_metre {
            let mut body = Vec::new();
            body.extend_from_slice(&density.to_be_bytes());
            body.extend_from_slice(&density.to_be_bytes());
            body.push(1);
            chunk(b"pHYs", &body);
        }
        chunk(b"IDAT", &[1, 2, 3]);
        chunk(b"IEND", &[]);
        data
    }

    fn jpeg(width: u16, height: u16, dpi: Option<u16>) -> Vec<u8> {
        let mut data = vec![0xff, 0xd8];
        if let Some(dpi) = dpi {
            data.extend_from_slice(&[0xff, 0xe0, 0, 16]);
            data.extend_from_slice(b"JFIF\0\x01\x02\x01");
            data.extend_from_slice(&dpi.to_be_bytes());
            data.extend_from_slice(&dpi.to_be_bytes());
            data.extend_from_slice(&[0, 0]);
        }
        // A table before the frame header, and a progressive frame.
        data.extend_from_slice(&[0xff, 0xdb, 0, 4, 9, 9, 0xff, 0xc4, 0, 3, 7]);
        data.extend_from_slice(&[0xff, 0xc2, 0, 11, 8]);
        data.extend_from_slice(&height.to_be_bytes());
        data.extend_from_slice(&width.to_be_bytes());
        data.extend_from_slice(&[3, 0, 0, 0, 0xff, 0xd9]);
        data
    }

    fn bmp(width: i32, height: i32) -> Vec<u8> {
        let mut data = b"BM".to_vec();
        data.extend_from_slice(&[0; 12]);
        data.extend_from_slice(&40i32.to_le_bytes());
        data.extend_from_slice(&width.to_le_bytes());
        data.extend_from_slice(&height.to_le_bytes());
        data.extend_from_slice(&[1, 0, 24, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        data.extend_from_slice(&7874i32.to_le_bytes());
        data.extend_from_slice(&7874i32.to_le_bytes());
        data.extend_from_slice(&[0; 8]);
        data
    }

    #[test]
    fn image_headers() {
        let info = image_info(&png(640, 480, None)).unwrap();
        assert_eq!(
            (info.format, info.pixels, info.dpi),
            (ImageFormat::Png, (640, 480), (96.0, 96.0))
        );
        assert_eq!(info.natural_size(), (640.0, 480.0));
        // 300 dpi: the picture is inserted at its printed size.
        let info = image_info(&png(3000, 1500, Some(11811))).unwrap();
        assert!((info.dpi.0 - 300.0).abs() < 0.01);
        let (w, h) = info.natural_size();
        assert!((w - 960.0).abs() < 0.1 && (h - 480.0).abs() < 0.1);

        let info = image_info(&jpeg(800, 600, None)).unwrap();
        assert_eq!(
            (info.format, info.pixels, info.dpi),
            (ImageFormat::Jpeg, (800, 600), (96.0, 96.0))
        );
        let info = image_info(&jpeg(1440, 720, Some(144))).unwrap();
        assert_eq!((info.pixels, info.natural_size()), ((1440, 720), (960.0, 480.0)));

        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&[0x40, 0x01, 0xf0, 0x00, 0, 0, 0]);
        let info = image_info(&gif).unwrap();
        assert_eq!((info.format, info.pixels), (ImageFormat::Gif, (320, 240)));

        // Rows stored top to bottom have a negative height; 7874 pixels per metre is 200 dpi.
        let info = image_info(&bmp(200, -100)).unwrap();
        assert_eq!((info.format, info.pixels), (ImageFormat::Bmp, (200, 100)));
        assert!((info.natural_size().0 - 96.0).abs() < 0.1);

        assert!(image_info(b"").is_none());
        assert!(image_info(b"GIF89a").is_none());
        assert!(image_info(b"%PDF-1.7 not an image").is_none());
        assert!(image_info(&png(0, 10, None)).is_none());
        assert!(image_info(&[0xff, 0xd8, 0xff, 0xd9]).is_none());
        assert!(image_info(&png(640, 480, None)[..20]).is_none());
        assert_eq!(ImageFormat::Jpeg.extension(), "jpeg");
        assert_eq!(ImageFormat::Png.mime(), "image/png");
    }

    #[test]
    fn pictures() {
        let mut w = wb(&[]);
        assert_eq!(w.add_picture(0, b"not an image".to_vec(), (0, 0)), None);
        let bytes = png(128, 40, None);
        let id = w.add_picture(0, bytes.clone(), (2, 1)).unwrap();
        w.commit("picture");
        let picture = w.drawing(0, id).unwrap();
        assert_eq!(picture.name, "Picture 1");
        assert_eq!((picture.anchor, picture.size), ((2, 1), (128.0, 40.0)));
        assert_eq!(
            (picture.placement, picture.lock_aspect, picture.locked),
            (Placement::MoveOnly, true, true)
        );
        let stored = picture.as_picture().unwrap();
        assert_eq!((stored.format, stored.pixels), (ImageFormat::Png, (128, 40)));
        assert_eq!(&stored.data[..], &bytes[..]);
        assert_eq!(w.drawing_frame(0, id), Some(Frame::new(64.0, 40.0, 128.0, 40.0)));
        // The copy of a sheet shares the bytes and has ids of its own.
        let copy = w.duplicate_sheet(0).unwrap();
        w.commit("copy");
        let twin = &w.sheets[copy].drawings[0];
        assert_ne!(twin.id, id);
        assert!(Arc::ptr_eq(
            &twin.as_picture().unwrap().data,
            &w.sheets[0].drawings[0].as_picture().unwrap().data
        ));
        w.undo();
        w.undo();
        assert!(w.drawing(0, id).is_none());
        w.redo();
        assert!(w.drawing(0, id).is_some());
        // In French a picture is an "Image".
        w.locale = Locale::fr();
        let second = w.add_picture(0, jpeg(10, 10, None), (0, 0)).unwrap();
        assert_eq!(w.drawing(0, second).unwrap().name, "Image 2");
    }

    #[test]
    fn shapes_of_the_gallery() {
        let mut w = wb(&[]);
        let mut ids = Vec::new();
        for (i, tool) in ShapeTool::ALL.into_iter().enumerate() {
            let (width, height) = tool.default_size();
            let frame = Frame::new(10.0 + 100.0 * i as f64, 30.0, width as f64, height as f64);
            ids.push(w.add_shape(0, tool, frame).unwrap());
        }
        w.commit("shapes");
        let named = |id: u32| w.drawing(0, id).unwrap().name.clone();
        assert_eq!(named(ids[0]), "Rectangle 1");
        assert_eq!(named(ids[2]), "Oval 3");
        assert_eq!(named(ids[14]), "Straight Arrow Connector 15");
        assert_eq!(named(ids[16]), "TextBox 17");
        let shape = |tool: ShapeTool| tool.shape();
        let rectangle = shape(ShapeTool::Rectangle);
        assert_eq!(
            (rectangle.fill, rectangle.align, rectangle.anchor),
            (Some(ACCENT), TextAlign::Center, TextAnchor::Middle)
        );
        assert_eq!(rectangle.font.color, Some(Color::WHITE));
        let ends = |tool: ShapeTool| shape(tool).outline.map(|o| (o.start, o.end)).unwrap();
        assert_eq!(ends(ShapeTool::Line), (LineEnd::None, LineEnd::None));
        assert_eq!(ends(ShapeTool::Arrow), (LineEnd::None, LineEnd::Triangle));
        assert_eq!(ends(ShapeTool::DoubleArrow), (LineEnd::Triangle, LineEnd::Triangle));
        assert!(shape(ShapeTool::Arrow).fill.is_none());
        let text_box = shape(ShapeTool::TextBox);
        assert!(text_box.text_box && text_box.preset == ShapePreset::Rect);
        assert_eq!(
            (text_box.fill, text_box.align, text_box.anchor),
            (Some(Color::WHITE), TextAlign::Left, TextAnchor::Top)
        );
        // Shapes are moved and sized with their cells and sit where they were drawn.
        let second = w.drawing(0, ids[1]).unwrap();
        assert_eq!(
            (second.placement, second.anchor, second.offset),
            (Placement::MoveAndSize, (1, 1), (46.0, 10.0))
        );
        assert_eq!((second.to, second.to_offset), ((6, 3), (14.0, 6.0)));
        assert_eq!(w.drawing_frame(0, ids[1]), Some(Frame::new(110.0, 30.0, 96.0, 96.0)));
        // A shape given without a name is named after what it looks like.
        let unnamed = w
            .add_drawing(0, Drawing::shape(ShapeTool::DoubleArrow.shape(), (50.0, 0.0)))
            .unwrap();
        assert_eq!(named_in(&w, unnamed), "Straight Arrow Connector 21");
        w.locale = Locale::de();
        let star = w
            .add_drawing(0, Drawing::shape(ShapeTool::Star5.shape(), (50.0, 50.0)))
            .unwrap();
        assert_eq!(named_in(&w, star), "Stern mit 5 Zacken 22");
    }

    fn named_in(w: &Workbook, id: u32) -> String {
        w.drawing(0, id).unwrap().name.clone()
    }

    #[test]
    fn frames_follow_column_widths() {
        let mut w = wb(&[]);
        let sized = w
            .add_shape(0, ShapeTool::Rectangle, Frame::new(80.0, 30.0, 100.0, 50.0))
            .unwrap();
        let moved = w
            .add_shape(0, ShapeTool::Ellipse, Frame::new(80.0, 30.0, 100.0, 50.0))
            .unwrap();
        let fixed = w
            .add_shape(0, ShapeTool::Diamond, Frame::new(80.0, 30.0, 100.0, 50.0))
            .unwrap();
        w.update_drawing(0, moved, |d| d.placement = Placement::MoveOnly);
        w.update_drawing(0, fixed, |d| d.placement = Placement::Fixed);
        w.commit("shapes");
        assert_eq!(w.drawing(0, fixed).unwrap().anchor, (0, 0));
        assert_eq!(w.drawing(0, fixed).unwrap().offset, (80.0, 30.0));
        let frame = |w: &Workbook, id: u32| w.drawing_frame(0, id).unwrap();
        for id in [sized, moved, fixed] {
            assert_eq!(frame(&w, id), Frame::new(80.0, 30.0, 100.0, 50.0));
        }
        // Column A (before all three) and column B (under them) get wider.
        w.set_col_width(0, 0, Some(100.0));
        w.set_col_width(0, 1, Some(164.0));
        w.commit("widths");
        assert_eq!(frame(&w, sized), Frame::new(116.0, 30.0, 200.0, 50.0));
        assert_eq!(frame(&w, moved), Frame::new(116.0, 30.0, 100.0, 50.0));
        assert_eq!(frame(&w, fixed), Frame::new(80.0, 30.0, 100.0, 50.0));
        // Hiding the rows under a drawing squeezes the one sized with them.
        w.set_rows_hidden(0, 2, 2, true);
        w.commit("hide");
        assert_eq!(frame(&w, sized).height, 30.0);
        assert_eq!(frame(&w, moved).height, 50.0);
        // The stored size is brought up to date whenever the drawing is changed.
        assert_eq!(w.drawing(0, sized).unwrap().size, (100.0, 50.0));
        w.update_drawing(0, sized, |d| d.alt_text = "A box".into());
        assert_eq!(w.drawing(0, sized).unwrap().size, (200.0, 30.0));
        assert_eq!(frame(&w, sized), Frame::new(116.0, 30.0, 200.0, 30.0));
        // The corners a file would hold.
        let corners = w.sheets[0].drawing_corners(w.drawing(0, moved).unwrap());
        assert_eq!(corners, (((1, 1), (16.0, 10.0)), ((5, 1), (116.0, 0.0))));
        // Narrowing the anchor column below the offset pulls the corner back to its edge.
        w.set_col_width(0, 1, Some(10.0));
        assert_eq!(frame(&w, moved).x, 110.0);
    }

    #[test]
    fn moving_and_resizing() {
        let mut w = wb(&[]);
        let id = w
            .add_shape(0, ShapeTool::Rectangle, Frame::new(0.0, 0.0, 96.0, 96.0))
            .unwrap();
        w.commit("shape");
        assert!(w.set_drawing_frame(0, id, Frame::new(200.0, 110.0, 60.0, 25.0)));
        w.commit("move");
        let d = w.drawing(0, id).unwrap();
        assert_eq!((d.anchor, d.offset, d.size), ((5, 3), (8.0, 10.0), (60.0, 25.0)));
        assert_eq!((d.to, d.to_offset), ((6, 4), (4.0, 15.0)));
        // Setting the cell by hand works too, and offsets past the cell are carried over.
        w.update_drawing(0, id, |d| {
            d.anchor = (0, 0);
            d.offset = (70.0, 45.0);
            d.rotation = 90.0;
            d.flip_h = true;
        });
        w.commit("turn");
        let d = w.drawing(0, id).unwrap();
        assert_eq!(
            (d.anchor, d.offset, d.rotation, d.flip_h),
            ((2, 1), (6.0, 5.0), 90.0, true)
        );
        w.undo();
        w.undo();
        assert_eq!(w.drawing_frame(0, id), Some(Frame::new(0.0, 0.0, 96.0, 96.0)));
        assert!(!w.set_drawing_frame(0, 999, Frame::new(0.0, 0.0, 1.0, 1.0)));
        assert!(!w.update_drawing(3, id, |_| {}));
        // The id cannot be changed by an update.
        w.update_drawing(0, id, |d| d.id = 77);
        assert!(w.drawing(0, id).is_some());
    }

    #[test]
    fn points_and_corners() {
        let mut w = wb(&[]);
        w.set_col_width(0, 1, Some(100.0));
        w.set_rows_hidden(0, 1, 1, true);
        let sheet = &w.sheets[0];
        // Column B is 100 wide and row 2 takes no room.
        assert_eq!(sheet.corner_at(0.0, 0.0), ((0, 0), (0.0, 0.0)));
        assert_eq!(sheet.corner_at(70.0, 25.0), ((2, 1), (6.0, 5.0)));
        assert_eq!(sheet.corner_at(164.0, 20.0), ((2, 2), (0.0, 0.0)));
        assert_eq!(sheet.corner_at(-5.0, -5.0), ((0, 0), (0.0, 0.0)));
        for point in [(0.0, 0.0), (70.0, 25.0), (164.0, 20.0), (1000.5, 333.25)] {
            assert_eq!(sheet.corner_point(sheet.corner_at(point.0, point.1)), point);
        }
        // An offset larger than its cell is not held back.
        assert_eq!(sheet.corner_point(((0, 0), (300.0, 50.0))), (300.0, 50.0));
        assert_eq!(sheet.corner_point(((3, 2), (1.0, 2.0))), (165.0, 42.0));
    }

    #[test]
    fn stacking_order_and_duplicates() {
        let mut w = wb(&[]);
        let a = w
            .add_shape(0, ShapeTool::Rectangle, Frame::new(0.0, 0.0, 50.0, 50.0))
            .unwrap();
        let b = w
            .add_shape(0, ShapeTool::Ellipse, Frame::new(20.0, 20.0, 50.0, 50.0))
            .unwrap();
        let c = w.add_picture(0, png(50, 50, None), (2, 0)).unwrap();
        w.commit("three");
        let order = |w: &Workbook| w.sheets[0].drawings.iter().map(|d| d.id).collect::<Vec<_>>();
        assert_eq!(order(&w), [a, b, c]);
        assert!(w.reorder_drawing(0, a, ZOrder::BringForward));
        assert_eq!(order(&w), [b, a, c]);
        assert!(w.reorder_drawing(0, c, ZOrder::SendToBack));
        assert_eq!(order(&w), [c, b, a]);
        assert!(w.reorder_drawing(0, c, ZOrder::BringToFront));
        assert!(w.reorder_drawing(0, c, ZOrder::SendBackward));
        assert_eq!(order(&w), [b, c, a]);
        assert!(!w.reorder_drawing(0, a, ZOrder::BringToFront));
        assert!(!w.reorder_drawing(0, b, ZOrder::SendBackward));
        assert!(!w.reorder_drawing(0, 999, ZOrder::SendToBack));
        w.commit("order");
        // The topmost drawing under the pointer is hit.
        assert_eq!(w.drawing_at(0, 30.0, 30.0), Some(a));
        assert_eq!(w.drawing_at(0, 60.0, 60.0), Some(b));
        assert_eq!(w.drawing_at(0, 5.0, 60.0), Some(c));
        assert_eq!(w.drawing_at(0, 300.0, 300.0), None);

        let copy = w.duplicate_drawing(0, b).unwrap();
        w.commit("duplicate");
        assert_eq!(order(&w), [b, c, a, copy]);
        let (original, twin) = (w.drawing(0, b).unwrap(), w.drawing(0, copy).unwrap());
        assert_eq!((original.name.as_str(), twin.name.as_str()), ("Oval 2", "Oval 4"));
        assert_eq!(twin.kind, original.kind);
        assert_eq!(w.drawing_frame(0, copy), Some(Frame::new(36.0, 36.0, 50.0, 50.0)));
        assert!(w.remove_drawing(0, b));
        assert!(!w.remove_drawing(0, b));
        w.commit("remove");
        assert_eq!(order(&w), [c, a, copy]);
        w.undo();
        w.undo();
        assert_eq!(order(&w), [b, c, a]);
    }

    #[test]
    fn hits_respect_rotation_and_lines() {
        let mut w = wb(&[]);
        // A wide bar turned by a quarter stands upright around its centre (100, 100).
        let bar = w
            .add_shape(0, ShapeTool::Rectangle, Frame::new(50.0, 90.0, 100.0, 20.0))
            .unwrap();
        assert_eq!(w.drawing_at(0, 60.0, 100.0), Some(bar));
        w.update_drawing(0, bar, |d| d.rotation = 90.0);
        assert_eq!(w.drawing_at(0, 60.0, 100.0), None);
        assert_eq!(w.drawing_at(0, 100.0, 60.0), Some(bar));
        w.remove_drawing(0, bar);
        // A line is hit along its length only.
        let line = w
            .add_shape(0, ShapeTool::Arrow, Frame::new(200.0, 200.0, 100.0, 100.0))
            .unwrap();
        assert_eq!(w.drawing_at(0, 250.0, 252.0), Some(line));
        assert_eq!(w.drawing_at(0, 290.0, 210.0), None);
        w.update_drawing(0, line, |d| d.flip_v = true);
        assert_eq!(w.drawing_at(0, 290.0, 210.0), Some(line));
        assert_eq!(w.drawing_at(0, 250.0, 252.0), Some(line));
        assert_eq!(w.drawing_at(0, 210.0, 210.0), None);
        let flat = w
            .add_shape(0, ShapeTool::Line, Frame::new(10.0, 500.0, 96.0, 0.0))
            .unwrap();
        assert_eq!(w.drawing_at(0, 50.0, 502.0), Some(flat));
        assert_eq!(w.drawing_at(0, 50.0, 510.0), None);
    }

    #[test]
    fn structural_edits() {
        let mut w = wb(&[]);
        // All three cover rows 3 to 6 (pixels 50 to 110) and columns B to C.
        let frame = Frame::new(80.0, 50.0, 100.0, 60.0);
        let sized = w.add_shape(0, ShapeTool::Rectangle, frame).unwrap();
        let moved = w.add_picture(0, png(100, 60, None), (2, 1)).unwrap();
        let fixed = w.add_shape(0, ShapeTool::Diamond, frame).unwrap();
        w.update_drawing(0, moved, |d| d.offset = (16.0, 10.0));
        w.update_drawing(0, fixed, |d| d.placement = Placement::Fixed);
        w.commit("drawings");
        let frames = |w: &Workbook| [sized, moved, fixed].map(|id| w.drawing_frame(0, id).unwrap());
        assert_eq!(frames(&w), [frame; 3]);
        // Rows inserted above push down what moves with the cells.
        w.insert_rows(0, 0, 2);
        w.commit("insert");
        let lower = Frame::new(80.0, 90.0, 100.0, 60.0);
        assert_eq!(frames(&w), [lower, lower, frame]);
        // Rows inserted in the middle stretch what is sized with them.
        w.insert_rows(0, 6, 1);
        w.commit("insert");
        assert_eq!(frames(&w), [Frame::new(80.0, 90.0, 100.0, 80.0), lower, frame]);
        assert_eq!(w.drawing(0, sized).unwrap().size, (100.0, 80.0));
        // A column deleted under it takes away the part that lay there.
        w.delete_cols(0, 2, 1);
        w.commit("delete");
        assert_eq!(frames(&w)[0], Frame::new(80.0, 90.0, 48.0, 80.0));
        assert_eq!(frames(&w)[1], lower);
        // Deleting the anchor row drops the drawing on the line of the deletion.
        w.delete_rows(0, 3, 2);
        w.commit("delete");
        assert_eq!(frames(&w)[0], Frame::new(80.0, 60.0, 48.0, 70.0));
        assert_eq!(frames(&w)[1], Frame::new(80.0, 60.0, 100.0, 60.0));
        assert_eq!(frames(&w)[2], frame);
        // Edits on another sheet change nothing, and everything undoes.
        let other = w.add_sheet(1);
        w.commit("sheet");
        let before = frames(&w);
        w.insert_rows(other, 0, 5);
        w.commit("insert");
        assert_eq!(frames(&w), before);
        for _ in 0..6 {
            w.undo();
        }
        assert_eq!(frames(&w), [frame; 3]);
    }

    #[test]
    fn a_far_corner_on_a_gridline_does_not_grow() {
        let mut w = wb(&[]);
        // Exactly rows 2 and 3: the bottom edge is the top of row 4.
        let id = w
            .add_shape(0, ShapeTool::Rectangle, Frame::new(64.0, 20.0, 64.0, 40.0))
            .unwrap();
        assert_eq!(w.drawing(0, id).unwrap().to, (3, 2));
        w.insert_rows(0, 3, 4);
        w.insert_cols(0, 2, 1);
        assert_eq!(w.drawing_frame(0, id), Some(Frame::new(64.0, 20.0, 64.0, 40.0)));
        w.insert_rows(0, 2, 1);
        assert_eq!(w.drawing_frame(0, id), Some(Frame::new(64.0, 20.0, 64.0, 60.0)));
        // A drawing wholly inside deleted rows collapses onto the line, as in Excel.
        w.delete_rows(0, 0, 10);
        assert_eq!(w.drawing_frame(0, id), Some(Frame::new(64.0, 0.0, 64.0, 0.0)));
    }

    #[test]
    fn corners_from_a_file() {
        let mut w = wb(&[]);
        w.set_col_width(0, 1, Some(100.0));
        w.commit("width");
        let sheet = &w.sheets[0];
        let mut shape = Drawing::shape(ShapeTool::Hexagon.shape(), (0.0, 0.0));
        sheet.place_drawing(&mut shape, ((1, 1), (10.0, 5.0)), ((4, 2), (20.0, 15.0)));
        assert_eq!(shape.size, (110.0, 70.0));
        assert_eq!(sheet.drawing_frame(&shape), Frame::new(74.0, 25.0, 110.0, 70.0));
        assert_eq!(
            sheet.drawing_corners(&shape),
            (((1, 1), (10.0, 5.0)), ((4, 2), (20.0, 15.0)))
        );
        // An absolute anchor becomes a place on the sheet.
        let mut fixed = Drawing::shape(ShapeTool::Hexagon.shape(), (0.0, 0.0));
        fixed.placement = Placement::Fixed;
        sheet.place_drawing(&mut fixed, ((1, 1), (10.0, 5.0)), ((4, 2), (20.0, 15.0)));
        assert_eq!(
            (fixed.anchor, fixed.offset, fixed.size),
            ((0, 0), (74.0, 25.0), (110.0, 70.0))
        );
        assert_eq!(
            sheet.drawing_corners(&fixed),
            (((1, 1), (10.0, 5.0)), ((4, 2), (20.0, 15.0)))
        );
        // A corner in a hidden column keeps its offset for when the column comes back.
        let mut hidden = Drawing::shape(ShapeTool::Hexagon.shape(), (30.0, 30.0));
        hidden.placement = Placement::MoveOnly;
        hidden.anchor = (0, 3);
        hidden.offset = (12.0, 0.0);
        w.set_cols_hidden(0, 3, 3, true);
        w.sheets[0].settle_drawing(&mut hidden);
        assert_eq!((hidden.anchor, hidden.offset), ((0, 3), (12.0, 0.0)));
        assert_eq!(w.sheets[0].drawing_frame(&hidden).x, 228.0);
        w.set_cols_hidden(0, 3, 3, false);
        assert_eq!(w.sheets[0].drawing_frame(&hidden).x, 240.0);
    }

    /// The corners of a path, scaled to a frame.
    fn points(preset: ShapePreset, width: f32, height: f32) -> Vec<(f32, f32)> {
        preset
            .path(width, height)
            .into_iter()
            .filter_map(|command| match command {
                PathCommand::MoveTo(x, y) | PathCommand::LineTo(x, y) => Some((x * width, y * height)),
                PathCommand::CurveTo(.., x, y) => Some((x * width, y * height)),
                PathCommand::Close => None,
            })
            .collect()
    }

    #[test]
    fn outlines() {
        for preset in ShapePreset::ALL {
            let path = preset.path(120.0, 80.0);
            assert!(matches!(path[0], PathCommand::MoveTo(..)), "{preset:?}");
            assert_eq!(
                matches!(path.last(), Some(PathCommand::Close)),
                !preset.is_line(),
                "{preset:?}"
            );
            // Every outline spans its frame, the tail of the callout apart.
            let xs: Vec<f32> = points(preset, 1.0, 1.0).iter().map(|p| p.0).collect();
            let ys: Vec<f32> = points(preset, 1.0, 1.0).iter().map(|p| p.1).collect();
            let min = |v: &[f32]| v.iter().copied().fold(f32::MAX, f32::min);
            let max = |v: &[f32]| v.iter().copied().fold(f32::MIN, f32::max);
            assert!(
                min(&xs).abs() < 1e-3 && (max(&xs) - 1.0).abs() < 1e-3,
                "{preset:?} {xs:?}"
            );
            assert!(min(&ys).abs() < 1e-3, "{preset:?} {ys:?}");
            let bottom = if preset == ShapePreset::Callout { 1.125 } else { 1.0 };
            assert!((max(&ys) - bottom).abs() < 1e-3, "{preset:?} {ys:?}");
            let [l, t, r, b] = preset.text_rect(120.0, 80.0);
            assert!(
                0.0 <= l && l < r && r <= 1.0 && 0.0 <= t && t < b && b <= 1.0,
                "{preset:?}"
            );
        }
        // Round corners stay round whatever the proportions: a sixth of the shorter side.
        let corners = points(ShapePreset::RoundRect, 300.0, 60.0);
        assert!((corners[0].0 - 10.0).abs() < 1e-3 && (corners[2].1 - 10.0).abs() < 1e-3);
        // The head of an arrow is half the shorter side long.
        let arrow = points(ShapePreset::RightArrow, 300.0, 60.0);
        assert!((arrow[1].0 - 270.0).abs() < 1e-3 && (arrow[3].0 - 300.0).abs() < 1e-3);
        let up = points(ShapePreset::UpArrow, 60.0, 300.0);
        assert!((up[3].1 - 0.0).abs() < 1e-3 && (up[1].1 - 30.0).abs() < 1e-3);
        assert_eq!(points(ShapePreset::LeftArrow, 100.0, 100.0)[3], (0.0, 50.0));
        assert_eq!(points(ShapePreset::DownArrow, 100.0, 100.0)[3], (50.0, 100.0));
        // A five-pointed star: ten corners, the top point in the middle.
        let star = points(ShapePreset::Star5, 100.0, 100.0);
        assert_eq!(star.len(), 10);
        assert!((star[2].0 - 50.0).abs() < 1e-3 && star[2].1.abs() < 1e-3);
        assert!((star[1].0 - 38.2).abs() < 0.1 && (star[1].1 - 38.2).abs() < 0.1);
        let pentagon = points(ShapePreset::Pentagon, 100.0, 100.0);
        assert_eq!(pentagon.len(), 5);
        assert!((pentagon[0].1 - 38.2).abs() < 0.1 && (pentagon[3].0 - 80.9).abs() < 0.1);
        assert_eq!(points(ShapePreset::Line, 40.0, 30.0), [(0.0, 0.0), (40.0, 30.0)]);
        assert_eq!(ShapePreset::Ellipse.path(10.0, 10.0).len(), 6);
    }

    #[test]
    fn file_names() {
        for preset in ShapePreset::ALL {
            assert_eq!(ShapePreset::from_ooxml_name(preset.ooxml_name()), Some(preset));
        }
        assert_eq!(
            ShapePreset::from_ooxml_name("straightConnector1"),
            Some(ShapePreset::Line)
        );
        assert_eq!(ShapePreset::from_ooxml_name("cloud"), None);
        assert_eq!(ShapePreset::Callout.ooxml_name(), "wedgeRectCallout");
        for dash in Dash::ALL {
            assert_eq!(Dash::from_ooxml_name(dash.ooxml_name()), Some(dash));
            assert_eq!(dash.pattern().len() % 2, 0);
        }
        for end in LineEnd::ALL {
            assert_eq!(LineEnd::from_ooxml_name(end.ooxml_name()), Some(end));
        }
        for align in TextAlign::ALL {
            assert_eq!(TextAlign::from_ooxml_name(align.ooxml_name()), Some(align));
        }
        for anchor in TextAnchor::ALL {
            assert_eq!(TextAnchor::from_ooxml_name(anchor.ooxml_name()), Some(anchor));
        }
        for placement in Placement::ALL {
            assert_eq!(Placement::from_ooxml_name(placement.ooxml_name()), Some(placement));
        }
        for tool in ShapeTool::ALL {
            for locale in Locale::all() {
                assert!(!tool.name(locale).is_empty());
            }
        }
        assert_eq!(ShapeTool::TextBox.name(Locale::fr()), "ZoneTexte");
    }

    #[test]
    fn charts_and_drawings_share_one_stacking_order() {
        use crate::chart::{Chart, ChartKind};
        let mut w = wb(&[]);
        let rect = w.add_shape(0, ShapeTool::Rectangle, Frame::new(0.0, 0.0, 50.0, 50.0)).unwrap();
        let chart = w.add_chart(0, Chart::new(ChartKind::Column)).unwrap();
        let oval = w.add_shape(0, ShapeTool::Ellipse, Frame::new(0.0, 0.0, 50.0, 50.0)).unwrap();
        let (r, c, o) = (ObjectRef::Drawing(rect), ObjectRef::Chart(chart), ObjectRef::Drawing(oval));
        // Each new object goes on top, whatever its kind.
        assert_eq!(w.object_stack(0), vec![r, c, o]);
        assert_eq!(w.chart(0, chart).unwrap().name, "Chart 2");
        // One step takes a shape under a chart.
        assert!(w.reorder_objects(0, &[o], ZOrder::SendBackward));
        assert_eq!(w.object_stack(0), vec![r, o, c]);
        assert!(w.reorder_objects(0, &[c], ZOrder::SendToBack));
        assert_eq!(w.object_stack(0), vec![c, r, o]);
        // Several keep their order among themselves.
        assert!(w.reorder_objects(0, &[c, r], ZOrder::BringToFront));
        assert_eq!(w.object_stack(0), vec![o, c, r]);
        assert!(!w.reorder_objects(0, &[c, r], ZOrder::BringForward));
        assert!(w.reorder_objects(0, &[c, r], ZOrder::SendBackward));
        assert_eq!(w.object_stack(0), vec![c, r, o]);
        // The lists follow.
        assert_eq!(w.sheets[0].drawings.iter().map(|d| d.id).collect::<Vec<_>>(), vec![rect, oval]);
        // Names and visibility, whatever the kind.
        assert!(w.rename_object(0, c, "Sales"));
        assert!(!w.rename_object(0, c, " Sales ") && !w.rename_object(0, o, ""));
        assert_eq!((w.object_name(0, c), w.object_name(0, o)), (Some("Sales"), Some("Oval 3")));
        assert!(w.set_objects_hidden(0, &[c, o], true));
        assert!(!w.set_objects_hidden(0, &[c, o], true));
        assert_eq!((w.object_hidden(0, c), w.object_hidden(0, r)), (Some(true), Some(false)));
        assert_eq!(w.drawing_at(0, 5.0, 5.0), Some(rect));
        assert!(w.set_objects_hidden(0, &[c, o], false));
        w.commit("three objects");
        assert!(w.restack_objects(0, &[o, c]));
        assert_eq!(w.object_stack(0), vec![o, c, r]);
        assert_eq!(w.sheets[0].drawings.iter().map(|d| d.id).collect::<Vec<_>>(), vec![oval, rect]);
        assert!(!w.restack_objects(0, &[o, c, r]));
        w.commit("order");
        w.undo();
        assert_eq!(w.object_stack(0), vec![c, r, o]);
        // Objects that were never placed (an old file) lie above those
        // that were, charts first.
        let mut loose = wb(&[]);
        let a = loose.add_shape(0, ShapeTool::Rectangle, Frame::new(0.0, 0.0, 9.0, 9.0)).unwrap();
        let k = loose.add_chart(0, Chart::new(ChartKind::Pie)).unwrap();
        for d in &mut loose.sheets[0].drawings {
            d.z = 0;
        }
        loose.sheets[0].charts[0].z = 0;
        assert_eq!(loose.object_stack(0), vec![ObjectRef::Chart(k), ObjectRef::Drawing(a)]);
        let b = loose.add_shape(0, ShapeTool::Rectangle, Frame::new(0.0, 0.0, 9.0, 9.0)).unwrap();
        assert_eq!(loose.object_stack(0), vec![ObjectRef::Chart(k), ObjectRef::Drawing(a), ObjectRef::Drawing(b)]);
    }

    #[test]
    fn groups_hold_their_members_and_let_them_go() {
        let mut w = wb(&[]);
        let under = w.add_shape(0, ShapeTool::Rectangle, Frame::new(0.0, 0.0, 10.0, 10.0)).unwrap();
        let a = w.add_shape(0, ShapeTool::Rectangle, Frame::new(100.0, 100.0, 100.0, 50.0)).unwrap();
        let between = w.add_shape(0, ShapeTool::Ellipse, Frame::new(0.0, 0.0, 10.0, 10.0)).unwrap();
        let b = w.add_shape(0, ShapeTool::Ellipse, Frame::new(250.0, 120.0, 50.0, 80.0)).unwrap();
        w.update_drawing(0, b, |d| d.rotation = 30.0);
        w.commit("shapes");
        assert_eq!(w.group_drawings(0, &[a]), None);
        let group = w.group_drawings(0, &[a, b]).expect("a group");
        w.commit("group");
        // The box around both, in the place of the topmost.
        assert_eq!(w.drawing_frame(0, group), Some(Frame::new(100.0, 100.0, 200.0, 100.0)));
        let d = |id| ObjectRef::Drawing(id);
        assert_eq!(w.object_stack(0), vec![d(under), d(between), d(group)]);
        assert_eq!(w.drawing(0, group).unwrap().name, "Group 5");
        assert!(w.drawing(0, a).is_none());
        let frames = |w: &Workbook| -> Vec<(u32, Frame, f32)> {
            let sh = &w.sheets[0];
            let holder = w.drawing(0, group).unwrap();
            sh.drawing_leaves(holder).iter().map(|leaf| (leaf.drawing.id, leaf.frame, leaf.rotation)).collect()
        };
        assert_eq!(
            frames(&w),
            vec![(a, Frame::new(100.0, 100.0, 100.0, 50.0), 0.0), (b, Frame::new(250.0, 120.0, 50.0, 80.0), 30.0)]
        );
        // Twice as wide: the members stretch with it.
        w.set_drawing_frame(0, group, Frame::new(100.0, 100.0, 400.0, 100.0));
        assert_eq!(frames(&w)[1].1, Frame::new(400.0, 120.0, 100.0, 80.0));
        w.set_drawing_frame(0, group, Frame::new(100.0, 100.0, 200.0, 100.0));
        // Turned a quarter: the members go round its middle (200, 150).
        w.update_drawing(0, group, |g| g.rotation = 90.0);
        let turned = frames(&w);
        let near = |f: Frame, x: f64, y: f64| (f.x - x).abs() < 1e-6 && (f.y - y).abs() < 1e-6;
        // The middle of `a`, (150, 125), goes to (225, 100).
        assert!(near(turned[0].1, 175.0, 75.0), "{:?}", turned[0]);
        assert_eq!((turned[0].2, turned[1].2), (90.0, 120.0));
        // Flipped as well: mirrored first, then turned.
        w.update_drawing(0, group, |g| {
            g.rotation = 0.0;
            g.flip_h = true;
        });
        let sh = &w.sheets[0];
        let leaves = sh.drawing_leaves(w.drawing(0, group).unwrap());
        assert_eq!(leaves[0].frame, Frame::new(200.0, 100.0, 100.0, 50.0));
        assert!(leaves[0].flip_h && leaves[1].flip_h);
        assert_eq!(leaves[1].rotation, 330.0);
        w.update_drawing(0, group, |g| g.flip_h = false);
        w.commit("back");

        // A copy has members of its own.
        let copy = w.duplicate_drawing(0, group).unwrap();
        let ids = |w: &Workbook, id| w.drawing(0, id).unwrap().as_group().unwrap().members.iter().map(|m| m.id).collect::<Vec<_>>();
        assert!(ids(&w, copy).iter().all(|id| *id > copy));
        w.remove_drawing(0, copy);
        w.commit("copy and remove");

        // Ungrouping gives them back where they were, in its place.
        let freed = w.ungroup_drawing(0, group);
        assert_eq!(freed, vec![a, b]);
        assert_eq!(w.object_stack(0), vec![d(under), d(between), d(a), d(b)]);
        assert_eq!(w.drawing_frame(0, a), Some(Frame::new(100.0, 100.0, 100.0, 50.0)));
        assert_eq!(w.drawing_frame(0, b), Some(Frame::new(250.0, 120.0, 50.0, 80.0)));
        assert_eq!(w.drawing(0, b).unwrap().rotation, 30.0);
        assert!(w.ungroup_drawing(0, a).is_empty());
        w.commit("ungroup");
        w.undo();
        assert!(w.drawing(0, group).is_some() && w.drawing(0, a).is_none());

        // A group in a group.
        let outer = w.group_drawings(0, &[group, between]).unwrap();
        let sh = &w.sheets[0];
        assert_eq!(sh.drawing_leaves(w.drawing(0, outer).unwrap()).len(), 3);
        assert_eq!(w.ungroup_drawing(0, outer), vec![between, group]);
        assert_eq!(w.drawing_frame(0, group), Some(Frame::new(100.0, 100.0, 200.0, 100.0)));
    }

    #[test]
    fn picture_adjustments() {
        let plain = PictureAdjust::default();
        assert!(!plain.is_cropped() && !plain.changes_pixels());
        let pixel = [0.8, 0.4, 0.2, 1.0];
        assert_eq!(plain.apply(pixel), pixel);
        let near = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4);
        // Transparency only touches the alpha.
        let faded = PictureAdjust { transparency: 0.25, ..plain };
        assert!(near(faded.apply(pixel), [0.8, 0.4, 0.2, 0.75]));
        assert!(faded.changes_pixels() && !faded.is_cropped());
        // Brightness goes towards white or black; contrast spreads about
        // mid grey.
        assert!(near(PictureAdjust { brightness: 0.5, ..plain }.apply(pixel), [0.9, 0.7, 0.6, 1.0]));
        assert!(near(PictureAdjust { brightness: -0.5, ..plain }.apply(pixel), [0.4, 0.2, 0.1, 1.0]));
        assert!(near(PictureAdjust { contrast: -1.0, ..plain }.apply(pixel), [0.5, 0.5, 0.5, 1.0]));
        assert!(near(PictureAdjust { contrast: 0.5, ..plain }.apply(pixel), [0.95, 0.35, 0.05, 1.0]));
        // No saturation and grayscale agree.
        let grey = 0.299 * 0.8 + 0.587 * 0.4 + 0.114 * 0.2;
        assert!(near(PictureAdjust { saturation: 0.0, ..plain }.apply(pixel), [grey, grey, grey, 1.0]));
        assert!(near(PictureAdjust { recolor: Recolor::Grayscale, ..plain }.apply(pixel), [grey, grey, grey, 1.0]));
        assert!(near(PictureAdjust { recolor: Recolor::BlackAndWhite(0.5), ..plain }.apply(pixel), [0.0, 0.0, 0.0, 1.0]));
        assert!(near(PictureAdjust { recolor: Recolor::BlackAndWhite(0.25), ..plain }.apply(pixel), [1.0, 1.0, 1.0, 1.0]));
        // Two colours: black gives the dark one, white the light one.
        let duo = PictureAdjust { recolor: Recolor::Duotone(Color(0x102030), Color(0xF0E0D0)), ..plain };
        assert!(near(duo.apply([0.0, 0.0, 0.0, 1.0]), [16.0 / 255.0, 32.0 / 255.0, 48.0 / 255.0, 1.0]));
        assert!(near(duo.apply([1.0, 1.0, 1.0, 1.0]), [240.0 / 255.0, 224.0 / 255.0, 208.0 / 255.0, 1.0]));
        // Warm adds red and takes blue.
        let warm = PictureAdjust { temperature: 11_500.0, ..plain }.apply([0.5, 0.5, 0.5, 1.0]);
        assert!(warm[0] > 0.69 && warm[2] < 0.31 && warm[1] == 0.5);

        // A quarter cut off at the left and half at the bottom: the whole
        // image is larger than the frame, and starts before it.
        let cropped = PictureAdjust { crop: [0.25, 0.0, 0.0, 0.5], ..plain };
        assert!(cropped.is_cropped() && !cropped.changes_pixels());
        let frame = Frame::new(100.0, 100.0, 150.0, 50.0);
        let image = cropped.uncropped(frame);
        assert_eq!(image, Frame::new(50.0, 100.0, 200.0, 100.0));
        assert_eq!(PictureAdjust::crop_between(image, frame), [0.25, 0.0, 0.0, 0.5]);
        // Dragging the frame out past the image leaves room around it.
        let wider = PictureAdjust::crop_between(image, Frame::new(40.0, 100.0, 210.0, 100.0));
        assert!(near([wider[0], wider[1], wider[2], wider[3]], [-0.05, 0.0, 0.0, 0.0]));
    }

    #[test]
    fn shadow_presets() {
        assert_eq!(ShadowPreset::BottomRight.shadow(), Shadow::default());
        assert_eq!(ShadowPreset::Top.shadow().angle, 270.0);
        let centre = ShadowPreset::Center.shadow();
        assert_eq!((centre.distance, centre.size), (0.0, 1.02));
        assert!(Effects::default().is_none());
        assert!(!Effects { soft_edge: 2.5, ..Effects::default() }.is_none());
        assert_eq!(ShadowPreset::Left.name(Locale::fr()), "Décalage : gauche");
    }
}
