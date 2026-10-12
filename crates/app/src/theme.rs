//! Colours and metrics of the interface.

use gpui::{Hsla, Rgba, rgb};

pub fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

pub fn ca(hex: u32, alpha: f32) -> Hsla {
    let mut c: Rgba = rgb(hex);
    c.a = alpha;
    c.into()
}

pub fn engine_color(color: opensheet_engine::Color) -> Hsla {
    c(color.value())
}

// Brand
pub const ACCENT: u32 = 0x107C41;
pub const ACCENT_DARK: u32 = 0x0E5C2F;
pub const ACCENT_LIGHT: u32 = 0xCAEAD8;
pub const TITLEBAR: u32 = 0x107C41;
pub const TITLEBAR_HOVER: u32 = 0x0E6B38;

// Chrome
pub const CHROME_BG: u32 = 0xF5F5F5;
pub const PANEL_BG: u32 = 0xFFFFFF;
pub const BORDER: u32 = 0xE0E0E0;
pub const BORDER_STRONG: u32 = 0xC8C8C8;
pub const TEXT: u32 = 0x242424;
pub const TEXT_MUTED: u32 = 0x616161;
pub const TEXT_DISABLED: u32 = 0xA0A0A0;
pub const HOVER: u32 = 0xEBEBEB;
pub const PRESSED: u32 = 0xDCDCDC;
pub const TOGGLED: u32 = 0xD6D6D6;

// Grid
pub const GRID_BG: u32 = 0xFFFFFF;
pub const GRIDLINE: u32 = 0xE1E1E1;
pub const HEADER_BG: u32 = 0xF5F5F5;
pub const HEADER_BORDER: u32 = 0xD4D4D4;
pub const HEADER_TEXT: u32 = 0x444444;
pub const HEADER_SEL_BG: u32 = 0xD2E8DC;
pub const HEADER_SEL_TEXT: u32 = 0x0E5C2F;
pub const HEADER_FULL_BG: u32 = 0x107C41;
pub const SELECTION: u32 = 0x107C41;

/// Colours cycled through for references while a formula is edited.
pub const REF_COLORS: [u32; 7] = [
    0x4472C4, 0xC0504D, 0x8064A2, 0x2E9E4F, 0xD9609C, 0xE0802B, 0x2AA7B8,
];

// Metrics (logical pixels)
pub const TITLEBAR_H: f32 = 38.0;
pub const TABS_H: f32 = 34.0;
pub const RIBBON_H: f32 = 96.0;
pub const FORMULA_BAR_H: f32 = 28.0;
pub const SHEET_TABS_H: f32 = 30.0;
pub const STATUS_H: f32 = 24.0;
pub const COL_HEADER_H: f32 = 20.0;
pub const SCROLLBAR: f32 = 12.0;

/// Standard palette shown in colour pickers (theme colours row + tints).
pub const PALETTE_THEME: [u32; 10] = [
    0xFFFFFF, 0x000000, 0xE8E8E8, 0x0E2841, 0x156082, 0xE97132, 0x196B24, 0x0F9ED5, 0xA02B93,
    0x4EA72E,
];

pub const PALETTE_STANDARD: [u32; 10] = [
    0xC00000, 0xFF0000, 0xFFC000, 0xFFFF00, 0x92D050, 0x00B050, 0x00B0F0, 0x0070C0, 0x002060,
    0x7030A0,
];

/// The theme colours as a colour picker lays them out: the ten colours,
/// then five rows of lighter and darker variants.
pub fn palette_rows() -> Vec<[u32; 10]> {
    let mut rows = vec![PALETTE_THEME];
    for amount in [0.8f32, 0.6, 0.4, -0.25, -0.5] {
        rows.push(PALETTE_THEME.map(|col| match col {
            // White and black get greys instead of tints.
            0xFFFFFF => tint(0xFFFFFF, -(1.0 - amount.abs()) * 0.5 - 0.05),
            0x000000 => tint(0x000000, (amount.abs() * 0.6).min(0.9)),
            other => tint(other, amount),
        }));
    }
    rows
}

/// Lighter / darker variants of a theme colour, as in the Office picker.
pub fn tint(hex: u32, amount: f32) -> u32 {
    let ch = |shift: u32| -> u32 {
        let v = ((hex >> shift) & 0xFF) as f32;
        let out = if amount >= 0.0 {
            v + (255.0 - v) * amount
        } else {
            v * (1.0 + amount)
        };
        (out.round().clamp(0.0, 255.0) as u32) << shift
    };
    ch(16) | ch(8) | ch(0)
}
