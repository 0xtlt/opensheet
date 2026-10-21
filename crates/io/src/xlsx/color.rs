//! Colour resolution: SpreadsheetML colours are RGB, theme (+ tint),
//! indexed (legacy palette) or automatic. A theme colour stays one in the
//! engine (`Color::theme`), with the value it has in the theme of the
//! file; the others are plain colours.

use super::xml::Tag;
use opensheet_engine::Color;
use opensheet_engine::style::ThemeSlot;
use opensheet_engine::theme::ThemeColors;

/// The default Office theme, in the order the colours appear in a theme
/// part: dk1, lt1, dk2, lt2, accent1..6, hlink, folHlink.
pub(crate) const OFFICE_THEME: [u32; 12] = [
    0x000000, 0xFFFFFF, 0x44546A, 0xE7E6E6, 0x4472C4, 0xED7D31, 0xA5A5A5, 0xFFC000, 0x5B9BD5,
    0x70AD47, 0x0563C1, 0x954F72,
];

/// The legacy 56-colour palette (indices 0..=63; 64 and 65 are the system
/// foreground / background, i.e. "automatic").
const INDEXED: [u32; 64] = [
    0x000000, 0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x000000,
    0xFFFFFF, 0xFF0000, 0x00FF00, 0x0000FF, 0xFFFF00, 0xFF00FF, 0x00FFFF, 0x800000, 0x008000,
    0x000080, 0x808000, 0x800080, 0x008080, 0xC0C0C0, 0x808080, 0x9999FF, 0x993366, 0xFFFFCC,
    0xCCFFFF, 0x660066, 0xFF8080, 0x0066CC, 0xCCCCFF, 0x000080, 0xFF00FF, 0xFFFF00, 0x00FFFF,
    0x800080, 0x800000, 0x008080, 0x0000FF, 0x00CCFF, 0xCCFFFF, 0xCCFFCC, 0xFFFF99, 0x99CCFF,
    0xFF99CC, 0xCC99FF, 0xFFCC99, 0x3366FF, 0x33CCCC, 0x99CC00, 0xFFCC00, 0xFF9900, 0xFF6600,
    0x666699, 0x969696, 0x003366, 0x339966, 0x003300, 0x333300, 0x993300, 0x993366, 0x333399,
    0x333333,
];

/// Everything needed to turn a colour element into RGB.
pub(crate) struct Palette {
    /// Theme colours in theme-part order (see [`OFFICE_THEME`]).
    pub theme: [u32; 12],
    pub indexed: Vec<u32>,
}

impl Default for Palette {
    fn default() -> Palette {
        Palette {
            theme: OFFICE_THEME,
            indexed: INDEXED.to_vec(),
        }
    }
}

/// A colour element as written (`<color theme="4" tint="0.4"/>`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct RawColor {
    rgb: Option<u32>,
    theme: Option<u32>,
    indexed: Option<u32>,
    tint: f64,
    auto: bool,
}

/// Parse "RRGGBB" or "AARRGGBB" (alpha ignored).
pub(crate) fn parse_rgb(s: &str) -> Option<u32> {
    let s = s.trim().trim_start_matches('#');
    if !(s.len() == 6 || s.len() == 8) || !s.is_ascii() {
        return None;
    }
    u32::from_str_radix(&s[s.len() - 6..], 16).ok()
}

impl RawColor {
    pub fn from_tag(tag: &Tag) -> RawColor {
        let mut c = RawColor::default();
        for (k, v) in tag.attrs() {
            match k {
                "rgb" => c.rgb = parse_rgb(v),
                "theme" => c.theme = v.trim().parse().ok(),
                "indexed" => c.indexed = v.trim().parse().ok(),
                "tint" => {
                    c.tint = v
                        .trim()
                        .parse()
                        .ok()
                        .filter(|t: &f64| t.is_finite())
                        .unwrap_or(0.0)
                }
                "auto" => c.auto = super::xml::parse_bool(v),
                _ => {}
            }
        }
        c
    }

    /// The colour; `None` means automatic.
    pub fn resolve(&self, palette: &Palette) -> Option<Color> {
        if self.auto {
            return None;
        }
        let base = if let Some(rgb) = self.rgb {
            rgb
        } else if let Some(t) = self.theme {
            // SpreadsheetML counts the colours as the engine does: 0 =
            // lt1, 1 = dk1, 2 = lt2, 3 = dk2, where a theme part has the
            // dark ones first.
            let slot = ThemeSlot::from_index(t as usize)?;
            return Some(ThemeColors::from_part_order("", palette.theme).color(slot, self.tint));
        } else {
            *palette.indexed.get(self.indexed? as usize)?
        };
        Some(Color(apply_tint(base & 0xFF_FFFF, self.tint)))
    }

    /// True for the plain "text 1" theme colour, which is what applications
    /// write for ordinary black text.
    pub fn is_default_text(&self) -> bool {
        self.rgb.is_none() && self.theme == Some(1) && self.tint == 0.0
    }
}

/// Lighten (`tint > 0`) or darken (`tint < 0`) a colour the way Excel does.
///
/// Excel scales the luminance in the integer HLS space of the Windows colour
/// dialog (every component 0..=240), which is why its theme shades differ
/// by a unit or two from a floating point HSL computation (accent 1 of the
/// Office theme, 80 % lighter, is D9E1F2 in Excel and D9E2F3 in Word). This
/// reproduces the palette Excel shows for the Office themes exactly.
pub(crate) fn apply_tint(rgb: u32, tint: f64) -> u32 {
    const HLS_MAX: i32 = 240;
    const RGB_MAX: i32 = 255;
    if tint == 0.0 || !tint.is_finite() {
        return rgb;
    }
    let r = ((rgb >> 16) & 0xFF) as i32;
    let g = ((rgb >> 8) & 0xFF) as i32;
    let b = (rgb & 0xFF) as i32;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let sum = max + min;
    let lum = (sum * HLS_MAX + RGB_MAX) / (2 * RGB_MAX);
    let (hue, sat) = if max == min {
        (0, 0)
    } else {
        let d = max - min;
        let sat = if lum <= HLS_MAX / 2 {
            (d * HLS_MAX + sum / 2) / sum
        } else {
            (d * HLS_MAX + (2 * RGB_MAX - sum) / 2) / (2 * RGB_MAX - sum)
        };
        let delta = |c: i32| ((max - c) * (HLS_MAX / 6) + d / 2) / d;
        let mut hue = if r == max {
            delta(b) - delta(g)
        } else if g == max {
            HLS_MAX / 3 + delta(r) - delta(b)
        } else {
            2 * HLS_MAX / 3 + delta(g) - delta(r)
        };
        if hue < 0 {
            hue += HLS_MAX;
        }
        if hue > HLS_MAX {
            hue -= HLS_MAX;
        }
        (hue, sat)
    };

    let tint = tint.clamp(-1.0, 1.0);
    let scaled = if tint < 0.0 {
        lum as f64 * (1.0 + tint)
    } else {
        lum as f64 * (1.0 - tint) + HLS_MAX as f64 * tint
    };
    // Files store the nominal percentage cut to 1/32767 (80 % is written as
    // 0.79998...); the bias keeps that from dropping a unit of luminance.
    let lum = ((scaled + 0.01) as i32).clamp(0, HLS_MAX);

    let byte = |v: i32| ((v * RGB_MAX + HLS_MAX / 2) / HLS_MAX).clamp(0, 255) as u32;
    if sat == 0 {
        let v = byte(lum);
        return (v << 16) | (v << 8) | v;
    }
    let m2 = if lum <= HLS_MAX / 2 {
        (lum * (HLS_MAX + sat) + HLS_MAX / 2) / HLS_MAX
    } else {
        lum + sat - (lum * sat + HLS_MAX / 2) / HLS_MAX
    };
    let m1 = 2 * lum - m2;
    let channel = |mut h: i32| {
        if h < 0 {
            h += HLS_MAX;
        }
        if h > HLS_MAX {
            h -= HLS_MAX;
        }
        if h < HLS_MAX / 6 {
            m1 + ((m2 - m1) * h + HLS_MAX / 12) / (HLS_MAX / 6)
        } else if h < HLS_MAX / 2 {
            m2
        } else if h < HLS_MAX * 2 / 3 {
            m1 + ((m2 - m1) * (HLS_MAX * 2 / 3 - h) + HLS_MAX / 12) / (HLS_MAX / 6)
        } else {
            m1
        }
    };
    (byte(channel(hue + HLS_MAX / 3)) << 16)
        | (byte(channel(hue)) << 8)
        | byte(channel(hue - HLS_MAX / 3))
}

/// Scale, then shift, the lightness of a colour (both as fractions of 1):
/// the `lumMod` and `lumOff` adjustments of colours in charts and shapes.
///
/// Unlike the tints of cell colours ([`apply_tint`]), these are computed in
/// plain HSL by every Office application: accent 1 of the Office theme,
/// 80 % lighter, is DAE3F3 in a chart.
pub(crate) fn adjust_luminance(rgb: u32, scale: f64, shift: f64) -> u32 {
    let [r, g, b] = [16, 8, 0].map(|shift| ((rgb >> shift) & 0xFF) as f64 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let range = max - min;
    let lightness = (max + min) / 2.0;
    // Hue in sixths of a turn.
    let (hue, saturation) = if range == 0.0 {
        (0.0, 0.0)
    } else {
        let hue = if max == r {
            ((g - b) / range).rem_euclid(6.0)
        } else if max == g {
            (b - r) / range + 2.0
        } else {
            (r - g) / range + 4.0
        };
        (hue, range / (1.0 - (2.0 * lightness - 1.0).abs()))
    };

    let lightness = lightness * scale + shift;
    if !lightness.is_finite() {
        return rgb;
    }
    let lightness = lightness.clamp(0.0, 1.0);
    let chroma = (1.0 - (2.0 * lightness - 1.0).abs()) * saturation;
    let second = chroma * (1.0 - (hue % 2.0 - 1.0).abs());
    let (r, g, b) = match hue as u32 {
        0 => (chroma, second, 0.0),
        1 => (second, chroma, 0.0),
        2 => (0.0, chroma, second),
        3 => (0.0, second, chroma),
        4 => (second, 0.0, chroma),
        _ => (chroma, 0.0, second),
    };
    let floor = lightness - chroma / 2.0;
    let byte = |v: f64| ((v + floor) * 255.0).round().clamp(0.0, 255.0) as u32;
    (byte(r) << 16) | (byte(g) << 8) | byte(b)
}

/// The names charts and shapes call the colours of a theme by, in the
/// order cell colours number them.
const SCHEME_NAMES: [&str; 12] = [
    "bg1", "tx1", "bg2", "tx2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6",
    "hlink", "folHlink",
];

/// The colour of the theme a chart or a shape names (`<a:schemeClr>`, by
/// its place in the palette), its lightness scaled then shifted by
/// `lumMod` and `lumOff`: a reference to the theme when that is what a
/// variant of a colour picker is written as ("60 % lighter" is `lumMod`
/// 40 %, `lumOff` 60 %; "25 % darker" is `lumMod` 75 %). `None` for any
/// other adjustment: the colour is then a colour of its own.
///
/// A reference has the value cells give it (see [`apply_tint`]), which
/// can be a unit or two away from what the plain HSL arithmetic of
/// charts gives ([`adjust_luminance`]).
pub(crate) fn scheme_color(
    palette: &Palette,
    part_slot: usize,
    scale: f64,
    shift: f64,
) -> Option<Color> {
    // A theme part has the dark colours before the light ones.
    let index = match part_slot {
        0 => 1,
        1 => 0,
        2 => 3,
        3 => 2,
        other => other,
    };
    let slot = ThemeSlot::from_index(index)?;
    let tint = if shift == 0.0 && scale > 0.0 && scale <= 1.0 {
        scale - 1.0
    } else if shift > 0.0 && (scale + shift - 1.0).abs() < 5e-4 {
        shift
    } else {
        return None;
    };
    Some(ThemeColors::from_part_order("", palette.theme).color(slot, tint))
}

/// Append the colour of a chart or a shape, the content of an
/// `<a:solidFill>`: a colour of the theme by its name and its lightness,
/// as Excel writes the variants of its pickers; any other colour by its
/// value. `alpha` is the opacity in thousandths of a percent, if not full.
pub(crate) fn push_drawing_color(x: &mut String, color: Color, alpha: Option<u32>) {
    let (open, close) = match color.theme() {
        Some(reference) => {
            let mut open = format!(
                "<a:schemeClr val=\"{}\"",
                SCHEME_NAMES[reference.slot.index()]
            );
            // Whole tenths of a percent, as the pickers have them.
            let amount = (reference.tint.abs() * 1000.0).round() as u32 * 100;
            let adjustments = match amount {
                0 => String::new(),
                _ if reference.tint < 0.0 => format!("<a:lumMod val=\"{}\"/>", 100_000 - amount),
                _ => format!(
                    "<a:lumMod val=\"{}\"/><a:lumOff val=\"{amount}\"/>",
                    100_000 - amount
                ),
            };
            if adjustments.is_empty() && alpha.is_none() {
                open.push_str("/>");
                x.push_str(&open);
                return;
            }
            open.push('>');
            open.push_str(&adjustments);
            (open, "</a:schemeClr>")
        }
        None => {
            let open = format!("<a:srgbClr val=\"{:06X}\"", color.value() & 0xFF_FFFF);
            if alpha.is_none() {
                x.push_str(&open);
                x.push_str("/>");
                return;
            }
            (open + ">", "</a:srgbClr>")
        }
    };
    x.push_str(&open);
    if let Some(alpha) = alpha {
        x.push_str(&format!("<a:alpha val=\"{alpha}\"/>"));
    }
    x.push_str(close);
}

/// Mix two colours: `amount` of `fg` over `bg`.
pub(crate) fn blend(fg: u32, bg: u32, amount: f64) -> u32 {
    let mix = |shift: u32| {
        let f = ((fg >> shift) & 0xFF) as f64;
        let b = ((bg >> shift) & 0xFF) as f64;
        ((f * amount + b * (1.0 - amount)).round() as u32).min(255) << shift
    };
    mix(16) | mix(8) | mix(0)
}

#[cfg(test)]
#[allow(clippy::excessive_precision)]
mod tests {
    use super::*;
    use crate::xlsx::xml::{Event, Reader};

    // The tints Excel writes for the five shades of a palette column,
    // digit for digit as they appear in files.
    const LIGHTER_80: f64 = 0.79998168889431442;
    const LIGHTER_60: f64 = 0.59999389629810485;
    const LIGHTER_40: f64 = 0.39997558519241921;
    const DARKER_25: f64 = -0.249977111117893;
    const DARKER_50: f64 = -0.499984740745262;
    const ACCENT: [f64; 5] = [LIGHTER_80, LIGHTER_60, LIGHTER_40, DARKER_25, DARKER_50];
    const WHITE: [f64; 5] = [
        -4.9989318521683403E-2,
        -0.14999847407452621,
        DARKER_25,
        -0.34998626667073579,
        DARKER_50,
    ];
    const BLACK: [f64; 5] = [
        0.499984740745262,
        0.34998626667073579,
        0.249977111117893,
        0.14999847407452621,
        4.9989318521683403E-2,
    ];
    const BACKGROUND_2: [f64; 5] = [
        -9.9978637043366805E-2,
        DARKER_25,
        DARKER_50,
        -0.749992370372631,
        -0.89999084444715716,
    ];

    fn assert_palette(columns: &[(u32, [f64; 5], [u32; 5])]) {
        for (base, tints, shades) in columns {
            for (tint, expected) in tints.iter().zip(shades) {
                let got = apply_tint(*base, *tint);
                assert_eq!(
                    got, *expected,
                    "{base:06X} tint {tint}: got {got:06X}, Excel shows {expected:06X}"
                );
            }
        }
    }

    #[test]
    fn tints_match_the_excel_palette() {
        // The "Theme Colors" grid of the colour picker, Office 2013-2022 theme.
        assert_palette(&[
            (
                0xFFFFFF,
                WHITE,
                [0xF2F2F2, 0xD9D9D9, 0xBFBFBF, 0xA6A6A6, 0x808080],
            ),
            (
                0x000000,
                BLACK,
                [0x808080, 0x595959, 0x404040, 0x262626, 0x0D0D0D],
            ),
            (
                0xE7E6E6,
                BACKGROUND_2,
                [0xD0CECE, 0xAEAAAA, 0x757171, 0x3A3838, 0x161616],
            ),
            (
                0x44546A,
                ACCENT,
                [0xD6DCE4, 0xACB9CA, 0x8497B0, 0x333F4F, 0x222B35],
            ),
            (
                0x4472C4,
                ACCENT,
                [0xD9E1F2, 0xB4C6E7, 0x8EA9DB, 0x305496, 0x203764],
            ),
            (
                0xED7D31,
                ACCENT,
                [0xFCE4D6, 0xF8CBAD, 0xF4B084, 0xC65911, 0x833C0C],
            ),
            (
                0xA5A5A5,
                ACCENT,
                [0xEDEDED, 0xDBDBDB, 0xC9C9C9, 0x7B7B7B, 0x525252],
            ),
            (
                0xFFC000,
                ACCENT,
                [0xFFF2CC, 0xFFE699, 0xFFD966, 0xBF8F00, 0x806000],
            ),
            (
                0x5B9BD5,
                ACCENT,
                [0xDDEBF7, 0xBDD7EE, 0x9BC2E6, 0x2F75B5, 0x1F4E78],
            ),
            (
                0x70AD47,
                ACCENT,
                [0xE2EFDA, 0xC6E0B4, 0xA9D08E, 0x548235, 0x375623],
            ),
        ]);
        // The same grid for the Office 2007-2010 theme.
        assert_palette(&[
            (
                0xEEECE1,
                BACKGROUND_2,
                [0xDDD9C4, 0xC4BD97, 0x948A54, 0x494529, 0x1D1B10],
            ),
            (
                0x1F497D,
                ACCENT,
                [0xC5D9F1, 0x8DB4E2, 0x538DD5, 0x16365C, 0x0F243E],
            ),
            (
                0x4F81BD,
                ACCENT,
                [0xDCE6F1, 0xB8CCE4, 0x95B3D7, 0x366092, 0x244062],
            ),
            (
                0xC0504D,
                ACCENT,
                [0xF2DCDB, 0xE6B8B7, 0xDA9694, 0x963634, 0x632523],
            ),
            (
                0x9BBB59,
                ACCENT,
                [0xEBF1DE, 0xD8E4BC, 0xC4D79B, 0x76933C, 0x4F6228],
            ),
            (
                0x8064A2,
                ACCENT,
                [0xE4DFEC, 0xCCC0DA, 0xB1A0C7, 0x60497A, 0x403151],
            ),
            (
                0x4BACC6,
                ACCENT,
                [0xDAEEF3, 0xB7DEE8, 0x92CDDC, 0x31869B, 0x215967],
            ),
            (
                0xF79646,
                ACCENT,
                [0xFDE9D9, 0xFCD5B4, 0xFABF8F, 0xE26B0A, 0x974706],
            ),
        ]);
        assert_eq!(apply_tint(0x123456, 0.0), 0x123456);
        assert_eq!(apply_tint(0x123456, f64::NAN), 0x123456);
        assert_eq!(apply_tint(0x4472C4, 1.0), 0xFFFFFF);
        assert_eq!(apply_tint(0x4472C4, -1.0), 0x000000);
        assert_eq!(apply_tint(0x4472C4, 7.0), 0xFFFFFF);
    }

    #[test]
    fn luminance_of_drawing_colours() {
        // The "Theme Colors" grid of the shape fill picker, Office
        // 2013-2022 theme, accent 1: lighter 80 / 60 / 40 %, darker 25 / 50 %.
        assert_eq!(adjust_luminance(0x4472C4, 0.2, 0.8), 0xDAE3F3);
        assert_eq!(adjust_luminance(0x4472C4, 0.4, 0.6), 0xB4C7E7);
        assert_eq!(adjust_luminance(0x4472C4, 0.6, 0.4), 0x8FAADC);
        assert_eq!(adjust_luminance(0x4472C4, 0.75, 0.0), 0x2F5597);
        assert_eq!(adjust_luminance(0x4472C4, 0.5, 0.0), 0x203864);
        // Greys have no hue to keep.
        assert_eq!(adjust_luminance(0x000000, 0.65, 0.35), 0x595959);
        assert_eq!(adjust_luminance(0x000000, 0.15, 0.85), 0xD9D9D9);
        assert_eq!(adjust_luminance(0xFFFFFF, 0.85, 0.0), 0xD9D9D9);
        assert_eq!(adjust_luminance(0x123456, 1.0, 0.0), 0x123456);
        assert_eq!(adjust_luminance(0xED7D31, 1.0, 0.0), 0xED7D31);
        assert_eq!(adjust_luminance(0x4472C4, 0.0, 1.0), 0xFFFFFF);
        assert_eq!(adjust_luminance(0x4472C4, 0.0, 0.0), 0x000000);
        assert_eq!(adjust_luminance(0x4472C4, 9.0, 9.0), 0xFFFFFF);
        assert_eq!(adjust_luminance(0x4472C4, f64::NAN, 0.0), 0x4472C4);
    }

    #[test]
    fn theme_part() {
        let xml = r#"<a:theme xmlns:a="x"><a:themeElements><a:clrScheme name="T">
            <a:dk1><a:sysClr val="windowText" lastClr="111111"/></a:dk1>
            <a:lt1><a:sysClr val="window"/></a:lt1>
            <a:dk2><a:srgbClr val="0E2841"/></a:dk2><a:lt2><a:srgbClr val="E8E8E8"/></a:lt2>
            <a:accent1><a:srgbClr val="156082"/></a:accent1>
            </a:clrScheme><a:fontScheme><a:srgbClr val="FF0000"/></a:fontScheme></a:themeElements></a:theme>"#;
        let theme = crate::xlsx::theme::parse(xml).colors.part_order();
        assert_eq!(
            &theme[..5],
            &[0x111111, 0xFFFFFF, 0x0E2841, 0xE8E8E8, 0x156082]
        );
        assert_eq!(theme[5], OFFICE_THEME[5]);
        let palette = Palette {
            theme,
            ..Palette::default()
        };
        let color = |s: &str| {
            let xml = format!("<color {s}/>");
            let mut r = Reader::new(&xml);
            let Event::Open(t) = r.next() else { panic!() };
            RawColor::from_tag(&t).resolve(&palette)
        };
        // Colours of the theme stay references, with their value.
        let themed = |s: &str| color(s).map(|c| (c.value(), c.theme().map(|t| t.slot)));
        assert_eq!(
            themed("theme=\"0\""),
            Some((0xFFFFFF, Some(ThemeSlot::Light1)))
        );
        assert_eq!(
            themed("theme=\"1\""),
            Some((0x111111, Some(ThemeSlot::Dark1)))
        );
        assert_eq!(
            themed("theme=\"3\""),
            Some((0x0E2841, Some(ThemeSlot::Dark2)))
        );
        assert_eq!(
            themed("theme=\"4\""),
            Some((0x156082, Some(ThemeSlot::Accent1)))
        );
        let lighter = color("theme=\"4\" tint=\"0.39997558519241921\"").unwrap();
        assert_eq!(lighter.theme().unwrap().tint, 0.39997558519241921);
        assert_eq!(lighter.value(), apply_tint(0x156082, 0.39997558519241921));
        assert_eq!(color("rgb=\"FF00B050\""), Some(Color(0x00B050)));
        assert_eq!(color("indexed=\"10\""), Some(Color(0xFF0000)));
        assert_eq!(color("indexed=\"64\""), None);
        assert_eq!(color("auto=\"1\""), None);
        assert_eq!(color("theme=\"99\""), None);
    }
}
