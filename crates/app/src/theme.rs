//! Colours and metrics of the interface.

use gpui::{Hsla, Rgba, rgb};
use crate::i18n::t;
use opensheet_engine::style::{Font, ThemeFont};
use opensheet_engine::{Color, Theme};
use std::cell::RefCell;
use std::sync::Arc;

pub fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

pub fn ca(hex: u32, alpha: f32) -> Hsla {
    let mut c: Rgba = rgb(hex);
    c.a = alpha;
    c.into()
}

pub fn engine_color(color: Color) -> Hsla {
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

/// The standard colours of colour pickers, the same in every theme.
pub const PALETTE_STANDARD: [u32; 10] = [
    0xC00000, 0xFF0000, 0xFFC000, 0xFFFF00, 0x92D050, 0x00B050, 0x00B0F0, 0x0070C0, 0x002060,
    0x7030A0,
];

thread_local! {
    /// The theme of the workbook of the window being drawn.
    static DOCUMENT: RefCell<Arc<Theme>> = RefCell::new(Arc::new(Theme::default()));
}

/// Say which theme the pickers drawn from now on show: the application
/// does when it draws its window, so that a picker anywhere in it offers
/// the theme colours of its workbook.
pub fn set_document_theme(theme: Arc<Theme>) {
    DOCUMENT.with(|current| {
        if !Arc::ptr_eq(&current.borrow(), &theme) {
            *current.borrow_mut() = theme;
        }
    });
}

/// The theme of the workbook of the window being drawn.
pub fn document_theme() -> Arc<Theme> {
    DOCUMENT.with(|current| current.borrow().clone())
}

/// The theme colours as a colour picker lays them out: the ten colours,
/// then five rows of lighter and darker variants. Each is a reference to
/// the theme: cells given one follow the theme when it changes.
pub fn palette_rows() -> Vec<[Color; 10]> {
    document_theme().palette()
}

/// A colour moved towards white (`amount` > 0) or black, channel by
/// channel: the shades of chart palettes. (The variants of theme colours
/// in a picker are the engine's: `Theme::palette`.)
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

// ---- theme fonts --------------------------------------------------------------

/// What follows the name of a theme font in a list of fonts: "Aptos
/// Narrow (Body)", "Aptos Display (Headings)".
fn role_suffix(role: ThemeFont) -> &'static str {
    match role {
        ThemeFont::Major => t("(Headings)", "(En-têtes)"),
        ThemeFont::Minor => t("(Body)", "(Corps)"),
    }
}

/// The two entries a list of fonts begins with: the heading font and the
/// body font of the theme, as Excel names them.
pub fn theme_font_labels(theme: &Theme) -> [String; 2] {
    [ThemeFont::Major, ThemeFont::Minor].map(|role| format!("{} {}", theme.font(role), role_suffix(role)))
}

/// What a font box shows for a font: its name, with "(Body)" or
/// "(Headings)" when it is a font of the theme.
pub fn font_label(font: &Font) -> String {
    match font.theme {
        Some(role) => format!("{} {}", font.name, role_suffix(role)),
        None => font.name.to_string(),
    }
}

/// Give a font what was picked in a list of fonts or typed in a font box:
/// one of the two theme entries makes it a font of the theme, which
/// follows the theme; any other name is a font of its own.
pub fn set_font_choice(font: &mut Font, choice: &str, theme: &Theme) {
    let choice = choice.trim();
    let labels = theme_font_labels(theme);
    for (role, label) in [ThemeFont::Major, ThemeFont::Minor].into_iter().zip(&labels) {
        if choice.eq_ignore_ascii_case(label) {
            font.name = theme.font(role).clone();
            font.theme = Some(role);
            return;
        }
    }
    font.set_name(choice);
}
