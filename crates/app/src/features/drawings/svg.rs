//! Vector images on screen: an SVG document is turned into pixels by the
//! renderer GPUI itself draws its icons with (`resvg`), at the size it is
//! shown, so that a vector picture stays sharp at any zoom. The letters of
//! WordArt go the same way: an outline or a shadow around glyphs is
//! something an SVG renderer draws and GPUI's text does not.
//!
//! Bitmaps are kept by the text of the document they were made from and
//! forgotten when they have not been drawn for a while.

use super::picture::Bitmap;
use gpui::{App, RenderImage, Window};
use image::{Frame, RgbaImage};
use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;

/// The longest side of a bitmap made from an SVG document, in pixels.
pub const MAX_SIDE: f32 = 4096.0;

/// How many repaints a bitmap that is no longer drawn is kept for.
const KEPT_FOR: u64 = 300;

struct Shown {
    image: Arc<RenderImage>,
    used: u64,
}

#[derive(Default)]
struct Cache {
    images: HashMap<u64, Option<Shown>>,
    repaint: u64,
    retired: Vec<Arc<RenderImage>>,
}

thread_local! {
    static CACHE: RefCell<Cache> = RefCell::default();
}

/// Turns the bytes of a document into pixels.
type Rasterizer = Rc<dyn Fn(&[u8]) -> Option<Bitmap>>;

thread_local! {
    static RASTERIZER: RefCell<Option<Rasterizer>> = const { RefCell::new(None) };
}

/// Take hold of the application's SVG renderer (it shares the fonts of the
/// system, loaded once): painting has no other way to reach it.
pub fn install(cx: &App) {
    if RASTERIZER.with_borrow(|rasterizer| rasterizer.is_some()) {
        return;
    }
    let renderer = cx.svg_renderer();
    let rasterizer: Rasterizer = Rc::new(move |svg: &[u8]| {
        let image = gpui::Image::from_bytes(gpui::ImageFormat::Svg, svg.to_vec());
        let rendered = image.to_image_data(renderer.clone()).ok()?;
        let size = rendered.size(0);
        let (width, height) = (u32::try_from(size.width.0).ok()?, u32::try_from(size.height.0).ok()?);
        // The renderer gives red, green, blue and alpha with the colours
        // multiplied by alpha; GPUI paints blue, green, red, not
        // multiplied.
        let mut data = rendered.as_bytes(0)?.to_vec();
        for pixel in data.chunks_exact_mut(4) {
            let alpha = pixel[3] as u32;
            if alpha > 0 && alpha < 255 {
                for channel in pixel.iter_mut().take(3) {
                    *channel = ((*channel as u32 * 255 + alpha / 2) / alpha).min(255) as u8;
                }
            }
            pixel.swap(0, 2);
        }
        (width > 0 && height > 0).then_some(Bitmap { width, height, data })
    });
    RASTERIZER.with_borrow_mut(|slot| *slot = Some(rasterizer));
}

/// The pixels of an SVG document at its own size (its `width` and
/// `height`). `None` when it cannot be read, or before the first repaint.
pub fn rasterize(svg: &[u8]) -> Option<Bitmap> {
    let rasterizer = RASTERIZER.with_borrow(|rasterizer| rasterizer.clone())?;
    rasterizer(svg)
}

fn into_image(bitmap: Bitmap) -> Option<Arc<RenderImage>> {
    let pixels = RgbaImage::from_raw(bitmap.width, bitmap.height, bitmap.data)?;
    Some(Arc::new(RenderImage::new(vec![Frame::new(pixels)])))
}

fn key(svg: &str, turn: (bool, bool, u8)) -> u64 {
    let mut hasher = DefaultHasher::new();
    svg.hash(&mut hasher);
    turn.hash(&mut hasher);
    hasher.finish()
}

/// What to paint for an SVG document: its pixels at its own size, flipped
/// and turned by quarters as asked. Made once and kept while it is drawn.
pub fn image(svg: &str, flips: (bool, bool), quarters: u8) -> Option<Arc<RenderImage>> {
    let key = key(svg, (flips.0, flips.1, quarters % 4));
    let known = CACHE.with_borrow_mut(|cache| {
        let repaint = cache.repaint;
        cache.images.get_mut(&key).map(|entry| {
            entry.as_mut().map(|shown| {
                shown.used = repaint;
                shown.image.clone()
            })
        })
    });
    if let Some(known) = known {
        return known;
    }
    let image = rasterize(svg.as_bytes())
        .map(|bitmap| bitmap.oriented(flips.0, flips.1, quarters))
        .and_then(into_image);
    CACHE.with_borrow_mut(|cache| {
        let used = cache.repaint;
        cache.images.insert(key, image.clone().map(|image| Shown { image, used }));
    });
    image
}

/// Called once per repaint: forget the bitmaps that have not been drawn
/// for a while and release their textures.
pub fn sweep(window: &mut Window, cx: &mut App) {
    let retired = CACHE.with_borrow_mut(|cache| {
        cache.repaint += 1;
        let Cache { images, repaint, retired } = cache;
        images.retain(|_, entry| match entry {
            Some(shown) if shown.used + KEPT_FOR < *repaint => {
                retired.push(shown.image.clone());
                false
            }
            // A document that could not be read is tried again later.
            None => *repaint % KEPT_FOR != 0,
            _ => true,
        });
        std::mem::take(retired)
    });
    for image in retired {
        cx.drop_image(image, Some(window));
    }
}

// ---- documents -----------------------------------------------------------

/// Text as it goes between the tags of a document or in an attribute.
pub fn escape(text: &str) -> String {
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

fn base64(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let bits = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(ALPHABET[(bits >> shift) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A document that shows another one stretched over `width` × `height`
/// pixels: how a picture of any proportions is drawn in its frame.
pub fn stretched(svg: &[u8], width: f32, height: f32) -> String {
    let fit = (MAX_SIDE / width.max(height).max(1.0)).min(1.0);
    let (width, height) = ((width * fit).round().max(1.0), (height * fit).round().max(1.0));
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{width}" height="{height}" viewBox="0 0 {width} {height}"><image width="{width}" height="{height}" preserveAspectRatio="none" xlink:href="data:image/svg+xml;base64,{}"/></svg>"#,
        base64(svg)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_are_wrapped_and_escaped() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(escape("a<b & \"c\"\n"), "a&lt;b &amp; &quot;c&quot; ");
        let wrapped = stretched(b"<svg/>", 200.4, 50.0);
        assert!(wrapped.contains(r#"width="200" height="50""#) && wrapped.contains("base64,PHN2Zy8+"));
        // A frame larger than any screen is drawn from fewer pixels.
        assert!(stretched(b"<svg/>", 8192.0, 1024.0).contains(r#"width="4096" height="512""#));
        assert_ne!(key("a", (false, false, 0)), key("a", (true, false, 0)));
    }
}
