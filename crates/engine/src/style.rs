//! Cell formatting model.

use std::sync::Arc;

/// 0xRRGGBB colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color(pub u32);

impl Color {
    pub const BLACK: Color = Color(0x000000);
    pub const WHITE: Color = Color(0xFFFFFF);

    pub fn rgb(r: u8, g: u8, b: u8) -> Color {
        Color(((r as u32) << 16) | ((g as u32) << 8) | b as u32)
    }

    pub fn r(self) -> u8 {
        (self.0 >> 16) as u8
    }

    pub fn g(self) -> u8 {
        (self.0 >> 8) as u8
    }

    pub fn b(self) -> u8 {
        self.0 as u8
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
        format!("{:06X}", self.0)
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

pub const DEFAULT_FONT: &str = "Aptos Narrow";
/// Font size in half points (22 = 11 pt).
pub const DEFAULT_FONT_SIZE: u16 = 22;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Font {
    pub name: Arc<str>,
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
}

impl Default for Font {
    fn default() -> Font {
        Font {
            name: Arc::from(DEFAULT_FONT),
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
            style.font.name = v.clone();
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
            font_name, font_size, bold, italic, underline, strike, font_color, fill, border_left, border_right,
            border_top, border_bottom, num_fmt
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

/// Interning table for styles.
#[derive(Clone, Debug)]
pub struct StyleTable {
    styles: Vec<Arc<Style>>,
    index: std::collections::HashMap<Arc<Style>, u32>,
}

impl Default for StyleTable {
    fn default() -> Self {
        let default = Arc::new(Style::default());
        let mut index = std::collections::HashMap::new();
        index.insert(default.clone(), 0);
        StyleTable {
            styles: vec![default],
            index,
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

    pub fn intern(&mut self, style: Style) -> StyleId {
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
}
