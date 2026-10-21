//! Cell formatting model.

use crate::theme::Theme;
use std::sync::Arc;

/// A colour: its 0xRRGGBB value and, for a colour picked among the theme
/// colours, which colour of the theme it is.
///
/// The value is always the colour to draw with. A theme colour keeps its
/// reference (slot and tint) beside it, and `Theme::resolve` works the
/// value out again when the theme of the workbook changes; a colour
/// without a reference never moves. Two colours of the same value are
/// different colours when only one of them follows the theme.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    rgb: u32,
    /// One more than the theme slot; 0 for a colour that is not a theme
    /// colour.
    slot: u8,
    /// The tint as files store it, in 1/32767.
    tint: i16,
}

/// A colour that is not a theme colour, from its 0xRRGGBB value.
#[allow(non_snake_case)]
pub const fn Color(rgb: u32) -> Color {
    Color {
        rgb,
        slot: 0,
        tint: 0,
    }
}

/// The twelve colours of a theme, numbered as cell colours number them
/// (`theme="4"` is accent 1): the light colours come before the dark ones,
/// the other way round from a theme part.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum ThemeSlot {
    /// Background 1 (white in most themes).
    Light1 = 0,
    /// Text 1 (black in most themes).
    Dark1 = 1,
    /// Background 2.
    Light2 = 2,
    /// Text 2.
    Dark2 = 3,
    Accent1 = 4,
    Accent2 = 5,
    Accent3 = 6,
    Accent4 = 7,
    Accent5 = 8,
    Accent6 = 9,
    Hyperlink = 10,
    FollowedHyperlink = 11,
}

impl ThemeSlot {
    pub const ALL: [ThemeSlot; 12] = [
        ThemeSlot::Light1,
        ThemeSlot::Dark1,
        ThemeSlot::Light2,
        ThemeSlot::Dark2,
        ThemeSlot::Accent1,
        ThemeSlot::Accent2,
        ThemeSlot::Accent3,
        ThemeSlot::Accent4,
        ThemeSlot::Accent5,
        ThemeSlot::Accent6,
        ThemeSlot::Hyperlink,
        ThemeSlot::FollowedHyperlink,
    ];

    /// The six accents, in order.
    pub const ACCENTS: [ThemeSlot; 6] = [
        ThemeSlot::Accent1,
        ThemeSlot::Accent2,
        ThemeSlot::Accent3,
        ThemeSlot::Accent4,
        ThemeSlot::Accent5,
        ThemeSlot::Accent6,
    ];

    /// The slot a cell colour names with `theme="index"`.
    pub fn from_index(index: usize) -> Option<ThemeSlot> {
        ThemeSlot::ALL.get(index).copied()
    }

    pub fn index(self) -> usize {
        self as usize
    }

    /// Accent `n` (1 to 6).
    pub fn accent(n: usize) -> ThemeSlot {
        ThemeSlot::ACCENTS[(n.clamp(1, 6)) - 1]
    }
}

/// A reference to a colour of the theme: the slot, made lighter
/// (`tint > 0`) or darker (`tint < 0`) by a fraction of 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeColor {
    pub slot: ThemeSlot,
    pub tint: f64,
}

/// Tints are kept in the unit Excel keeps them in: "40 % lighter" is
/// written 0.39997558519241921 in files, which is 13106 / 32767.
const TINT_UNIT: f64 = 32767.0;

impl Color {
    pub const BLACK: Color = Color(0x000000);
    pub const WHITE: Color = Color(0xFFFFFF);

    pub fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color(((r as u32) << 16) | ((g as u32) << 8) | b as u32)
    }

    /// The colour `slot` of a theme with a tint, whose value in that theme
    /// is `rgb`. `Theme::color` works the value out; this is for whoever
    /// already has it.
    pub const fn themed(slot: ThemeSlot, tint: f64, rgb: u32) -> Color {
        let tint = if tint.is_finite() {
            tint.clamp(-1.0, 1.0)
        } else {
            0.0
        };
        Color {
            rgb,
            slot: slot as u8 + 1,
            // Cut, not rounded: Excel writes 0.4 as 13106 / 32767.
            tint: (tint * TINT_UNIT + if tint < 0.0 { -1e-6 } else { 1e-6 }) as i16,
        }
    }

    /// The 0xRRGGBB value.
    pub const fn value(self) -> u32 {
        self.rgb
    }

    /// Which colour of the theme this is, for a theme colour.
    pub fn theme(self) -> Option<ThemeColor> {
        let slot = ThemeSlot::from_index((self.slot as usize).checked_sub(1)?)?;
        Some(ThemeColor {
            slot,
            tint: self.tint as f64 / TINT_UNIT,
        })
    }

    pub fn is_themed(self) -> bool {
        self.slot != 0
    }

    /// The same colour, no longer following the theme.
    pub const fn plain(self) -> Color {
        Color(self.rgb)
    }

    pub fn r(self) -> u8 {
        (self.rgb >> 16) as u8
    }

    pub fn g(self) -> u8 {
        (self.rgb >> 8) as u8
    }

    pub fn b(self) -> u8 {
        self.rgb as u8
    }

    /// Parse "RRGGBB", "#RRGGBB" or "AARRGGBB".
    pub fn from_hex(s: &str) -> Option<Color> {
        let s = s.trim_start_matches('#');
        let s = if s.len() == 8 { &s[2..] } else { s };
        if s.len() != 6 {
            return None;
        }
        u32::from_str_radix(s, 16).ok().map(Color)
    }

    pub fn to_hex(self) -> String {
        format!("{:06X}", self.rgb)
    }
}

impl std::fmt::Debug for Color {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.theme() {
            Some(theme) => write!(
                f,
                "Color({:#08X}, {:?}{:+})",
                self.rgb, theme.slot, theme.tint
            ),
            None => write!(f, "Color({})", self.rgb),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum HAlign {
    /// Text left, numbers right, booleans and errors centred.
    #[default]
    General,
    Left,
    Center,
    Right,
    Fill,
    Justify,
    CenterAcross,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum VAlign {
    Top,
    Center,
    #[default]
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum BorderStyle {
    #[default]
    None,
    Hair,
    Thin,
    Medium,
    Thick,
    Dashed,
    Dotted,
    Double,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Border {
    pub style: BorderStyle,
    /// `None` = automatic (black).
    pub color: Option<Color>,
}

impl Border {
    pub const NONE: Border = Border {
        style: BorderStyle::None,
        color: None,
    };

    pub fn thin() -> Border {
        Border {
            style: BorderStyle::Thin,
            color: None,
        }
    }

    pub fn is_none(&self) -> bool {
        self.style == BorderStyle::None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Borders {
    pub left: Border,
    pub right: Border,
    pub top: Border,
    pub bottom: Border,
}

impl Borders {
    pub fn is_none(&self) -> bool {
        self.left.is_none() && self.right.is_none() && self.top.is_none() && self.bottom.is_none()
    }
}

/// The body font of the Office theme: the font of the cells of a new
/// workbook.
pub const DEFAULT_FONT: &str = "Aptos Narrow";
/// Font size in half points (22 = 11 pt).
pub const DEFAULT_FONT_SIZE: u16 = 22;

/// The two fonts of a theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThemeFont {
    /// The font of headings ("Aptos Display (Headings)").
    Major,
    /// The font of body text ("Aptos Narrow (Body)").
    Minor,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Font {
    /// The name of the font the text is drawn in.
    pub name: Arc<str>,
    /// The font of the theme this font is, if it is one: `name` is then
    /// the name that font has in the theme of the workbook, and changes
    /// with the theme. A font given another name stops being a theme font
    /// (see `Font::set_name`).
    pub theme: Option<ThemeFont>,
    /// Size in half points (22 = 11 pt).
    pub size: u16,
    pub bold: bool,
    pub italic: bool,
    pub underline: Underline,
    pub strike: bool,
    /// `None` = automatic (black).
    pub color: Option<Color>,
}

impl Font {
    pub fn size_pt(&self) -> f32 {
        self.size as f32 / 2.0
    }

    pub fn set_size_pt(&mut self, pt: f32) {
        self.size = (pt * 2.0).round().clamp(2.0, 818.0) as u16;
    }

    /// Give the font a name of its own: it no longer follows the theme.
    pub fn set_name(&mut self, name: impl Into<Arc<str>>) {
        self.name = name.into();
        self.theme = None;
    }
}

impl Default for Font {
    fn default() -> Font {
        Font {
            name: Arc::from(DEFAULT_FONT),
            // The cells of a workbook are set in the body font.
            theme: Some(ThemeFont::Minor),
            size: DEFAULT_FONT_SIZE,
            bold: false,
            italic: false,
            underline: Underline::None,
            strike: false,
            color: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Style {
    pub font: Font,
    /// Solid fill colour (`None` = no fill).
    pub fill: Option<Color>,
    pub h_align: HAlign,
    pub v_align: VAlign,
    pub wrap: bool,
    pub shrink_to_fit: bool,
    pub indent: u8,
    /// Text rotation in degrees (-90..=90).
    pub rotation: i16,
    /// Number format code ("General", "0.00", "dd/mm/yyyy", ...), stored in
    /// canonical (English/US) syntax.
    pub num_fmt: Arc<str>,
    pub borders: Borders,
    /// Protection: the cell cannot be edited while its sheet is protected.
    pub locked: bool,
    /// Protection: the formula is not shown while the sheet is protected.
    pub hidden: bool,
    /// The cell style the cell was given (`None` = Normal), see
    /// `cell_style.rs`.
    pub cell_style: Option<Arc<str>>,
}

impl Default for Style {
    fn default() -> Style {
        Style {
            font: Font::default(),
            fill: None,
            h_align: HAlign::General,
            v_align: VAlign::Bottom,
            wrap: false,
            shrink_to_fit: false,
            indent: 0,
            rotation: 0,
            num_fmt: Arc::from("General"),
            borders: Borders::default(),
            locked: true,
            hidden: false,
            cell_style: None,
        }
    }
}

/// A partial style laid over a cell's own formatting: what a table style or
/// a conditional format changes. `None` leaves the property alone.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct StylePatch {
    pub font_name: Option<Arc<str>>,
    /// Half points.
    pub font_size: Option<u16>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<Underline>,
    pub strike: Option<bool>,
    /// `Some(None)` = automatic colour.
    pub font_color: Option<Option<Color>>,
    /// `Some(None)` = no fill.
    pub fill: Option<Option<Color>>,
    pub border_left: Option<Border>,
    pub border_right: Option<Border>,
    pub border_top: Option<Border>,
    pub border_bottom: Option<Border>,
    pub num_fmt: Option<Arc<str>>,
}

impl StylePatch {
    pub fn is_empty(&self) -> bool {
        *self == StylePatch::default()
    }

    /// Change `style` by what this patch sets.
    pub fn apply(&self, style: &mut Style) {
        if let Some(v) = &self.font_name {
            style.font.set_name(v.clone());
        }
        if let Some(v) = self.font_size {
            style.font.size = v;
        }
        if let Some(v) = self.bold {
            style.font.bold = v;
        }
        if let Some(v) = self.italic {
            style.font.italic = v;
        }
        if let Some(v) = self.underline {
            style.font.underline = v;
        }
        if let Some(v) = self.strike {
            style.font.strike = v;
        }
        if let Some(v) = self.font_color {
            style.font.color = v;
        }
        if let Some(v) = self.fill {
            style.fill = v;
        }
        if let Some(v) = self.border_left {
            style.borders.left = v;
        }
        if let Some(v) = self.border_right {
            style.borders.right = v;
        }
        if let Some(v) = self.border_top {
            style.borders.top = v;
        }
        if let Some(v) = self.border_bottom {
            style.borders.bottom = v;
        }
        if let Some(v) = &self.num_fmt {
            style.num_fmt = v.clone();
        }
    }

    /// Lay `over` on top of this patch: what `over` sets wins.
    pub fn merge(&mut self, over: &StylePatch) {
        macro_rules! take {
            ($($f:ident),*) => {$(
                if over.$f.is_some() {
                    self.$f = over.$f.clone();
                }
            )*};
        }
        take!(
            font_name,
            font_size,
            bold,
            italic,
            underline,
            strike,
            font_color,
            fill,
            border_left,
            border_right,
            border_top,
            border_bottom,
            num_fmt
        );
    }
}

impl Style {
    pub fn is_general(&self) -> bool {
        self.num_fmt.eq_ignore_ascii_case("General")
    }
}

/// Index into the workbook's style table. `0` is always the default style.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, PartialOrd, Ord)]
pub struct StyleId(pub u32);

/// Interning table for styles. It knows the theme of the workbook: the
/// styles it holds have the colours and the font names their theme
/// references give in that theme.
#[derive(Clone, Debug)]
pub struct StyleTable {
    styles: Vec<Arc<Style>>,
    index: std::collections::HashMap<Arc<Style>, u32>,
    theme: Arc<Theme>,
}

impl Default for StyleTable {
    fn default() -> Self {
        let default = Arc::new(Style::default());
        let mut index = std::collections::HashMap::new();
        index.insert(default.clone(), 0);
        StyleTable {
            styles: vec![default],
            index,
            theme: Arc::new(Theme::default()),
        }
    }
}

impl StyleTable {
    pub fn get(&self, id: StyleId) -> &Style {
        self.styles
            .get(id.0 as usize)
            .map(|s| s.as_ref())
            .unwrap_or_else(|| self.styles[0].as_ref())
    }

    /// The default style (id 0): the formatting of every cell that was
    /// given none. A new workbook has `Style::default()`; a workbook read
    /// from a file has the default font of that file (Calibri 11 in most).
    pub fn default_style(&self) -> &Style {
        self.styles[0].as_ref()
    }

    /// Replace the default style. The cells without formatting of their
    /// own change with it, so this is for a workbook being built (by the
    /// file reader), not for one being edited.
    pub fn set_default(&mut self, mut style: Style) {
        self.theme.settle_style(&mut style);
        let style = Arc::new(style);
        let old = std::mem::replace(&mut self.styles[0], style.clone());
        if self.index.get(&old) == Some(&0) {
            self.index.remove(&old);
        }
        self.index.insert(style, 0);
    }

    pub fn intern(&mut self, mut style: Style) -> StyleId {
        self.theme.settle_style(&mut style);
        if let Some(id) = self.index.get(&style) {
            return StyleId(*id);
        }
        let id = self.styles.len() as u32;
        let arc = Arc::new(style);
        self.styles.push(arc.clone());
        self.index.insert(arc, id);
        StyleId(id)
    }

    pub fn len(&self) -> usize {
        self.styles.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }

    /// The theme the styles are resolved against.
    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub(crate) fn theme_arc(&self) -> Arc<Theme> {
        self.theme.clone()
    }

    /// Change the theme: every style keeps its id and takes the colours
    /// and font names its theme references give in the new theme.
    /// `Workbook::set_theme` is the way to do this to a workbook.
    pub(crate) fn set_theme(&mut self, theme: Arc<Theme>) {
        self.theme = theme;
        self.index.clear();
        for (id, slot) in self.styles.iter_mut().enumerate() {
            let mut style = (**slot).clone();
            self.theme.resolve_style(&mut style);
            if style != **slot {
                *slot = Arc::new(style);
            }
            // Two styles that differ only by a reference are equal in no
            // theme, so no id is lost; should it ever happen, the first
            // keeps the entry.
            self.index.entry(slot.clone()).or_insert(id as u32);
        }
    }
}
