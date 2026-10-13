//! The theme of a workbook: twelve colours, a font for headings and one
//! for body text, and the name of its effects.
//!
//! A colour picked among the "Theme Colors" of a picker is a reference
//! (`Color::theme`): a slot of the theme and a tint. A font can be the
//! heading or the body font of the theme (`Font::theme`). Everything that
//! is stored keeps the colour and the font name it is drawn with, worked
//! out from the reference, so nothing that draws or writes has to know
//! about themes; changing the theme (`Workbook::set_theme`) works them out
//! again, everywhere, in one undo step. A colour without a reference never
//! moves.
//!
//! What is computed from the theme rather than stored (table styles,
//! PivotTable styles, the cell styles of the gallery, the automatic
//! colours of chart series) asks the workbook for its theme when it is
//! needed.

pub mod builtin;

use crate::chart::{Chart, Fill};
use crate::condfmt::Rule;
#[cfg(test)]
use crate::drawing::DrawingKind;
use crate::drawing::Shape;
use crate::filter::Criteria;
use crate::metrics::FontMetrics;
use crate::style::{Border, Borders, Color, Font, Style, StylePatch, ThemeColor, ThemeFont, ThemeSlot};
use crate::workbook::{Sheet, Workbook};
use builtin::{BuiltinColors, BuiltinFonts, BuiltinTheme};
use std::sync::Arc;

/// The twelve colours of a theme.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ThemeColors {
    /// The name of the set in the Colors menu ("Office", "Blue Warm", or
    /// the name the user gave a set of their own).
    pub name: String,
    /// By slot (`ThemeSlot::index`).
    rgb: [u32; 12],
}

impl ThemeColors {
    /// From colours in the order of a theme part: dark 1, light 1, dark 2,
    /// light 2, the accents, the two links.
    pub fn from_part_order(name: impl Into<String>, colors: [u32; 12]) -> ThemeColors {
        let mut rgb = colors;
        rgb.swap(0, 1);
        rgb.swap(2, 3);
        ThemeColors { name: name.into(), rgb }
    }

    /// The colours in the order of a theme part.
    pub fn part_order(&self) -> [u32; 12] {
        let mut colors = self.rgb;
        colors.swap(0, 1);
        colors.swap(2, 3);
        colors
    }

    pub fn get(&self, slot: ThemeSlot) -> u32 {
        self.rgb[slot.index()]
    }

    pub fn set(&mut self, slot: ThemeSlot, rgb: u32) {
        self.rgb[slot.index()] = rgb & 0xFF_FFFF;
    }

    /// The colour `slot` of this set, lighter (`tint > 0`) or darker.
    pub fn color(&self, slot: ThemeSlot, tint: f64) -> Color {
        // The tint a colour keeps is cut to Excel's unit; the value is
        // worked out from what is kept, so that it comes out the same
        // when it is worked out again.
        let kept = Color::themed(slot, tint, 0).theme().map_or(0.0, |t| t.tint);
        Color::themed(slot, kept, tint_rgb(self.get(slot), kept))
    }

    /// The same twelve colours, whatever the sets are called.
    pub fn same_colors(&self, other: &ThemeColors) -> bool {
        self.rgb == other.rgb
    }

    /// The eight colours a set is shown with in the Colors menu: text 2,
    /// background 2 and the six accents.
    pub fn swatches(&self) -> [u32; 8] {
        [3, 2, 4, 5, 6, 7, 8, 9].map(|slot| self.rgb[slot])
    }
}

impl From<&BuiltinColors> for ThemeColors {
    fn from(set: &BuiltinColors) -> ThemeColors {
        ThemeColors::from_part_order(set.name, set.colors)
    }
}

/// The two fonts of a theme.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ThemeFonts {
    /// The name of the pair in the Fonts menu.
    pub name: String,
    /// The font of headings.
    pub major: Arc<str>,
    /// The font of body text: the font of the cells.
    pub minor: Arc<str>,
}

impl ThemeFonts {
    pub fn new(name: impl Into<String>, major: &str, minor: &str) -> ThemeFonts {
        ThemeFonts {
            name: name.into(),
            major: Arc::from(major),
            minor: Arc::from(minor),
        }
    }

    pub fn get(&self, font: ThemeFont) -> &Arc<str> {
        match font {
            ThemeFont::Major => &self.major,
            ThemeFont::Minor => &self.minor,
        }
    }

    pub fn same_fonts(&self, other: &ThemeFonts) -> bool {
        self.major == other.major && self.minor == other.minor
    }
}

impl From<&BuiltinFonts> for ThemeFonts {
    fn from(pair: &BuiltinFonts) -> ThemeFonts {
        ThemeFonts::new(pair.name, pair.major, pair.minor)
    }
}

/// The theme of a workbook.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Theme {
    /// The name of the theme ("Office Theme", "Facet").
    pub name: String,
    pub colors: ThemeColors,
    pub fonts: ThemeFonts,
    /// The name of the effects (the format scheme of the theme part).
    /// Nothing is drawn from it: it is kept and written.
    pub effects: String,
    /// The theme part of the file the workbook was read from, for what
    /// the engine has no model of (the fills, lines and shadows of the
    /// effects, the fonts of other scripts): the writer takes them from
    /// here.
    pub source: Option<Arc<str>>,
}

impl Default for Theme {
    fn default() -> Theme {
        Theme::office()
    }
}

impl From<&BuiltinTheme> for Theme {
    fn from(theme: &BuiltinTheme) -> Theme {
        Theme {
            name: theme.name.to_string(),
            colors: ThemeColors::from_part_order(theme.scheme, theme.colors),
            fonts: ThemeFonts::from(&theme.fonts),
            effects: theme.scheme.to_string(),
            source: None,
        }
    }
}

impl Theme {
    /// The theme of a new workbook: the Office theme of 2023.
    pub fn office() -> Theme {
        Theme::from(&builtin::THEMES[0])
    }

    /// The Office theme of 2013 to 2022, which is the theme of a file
    /// that has no theme part.
    pub fn office_2013() -> Theme {
        Theme::from(&builtin::THEMES[1])
    }

    /// A theme of the gallery by its name.
    pub fn builtin(name: &str) -> Option<Theme> {
        builtin::THEMES
            .iter()
            .find(|theme| theme.name.eq_ignore_ascii_case(name))
            .map(Theme::from)
    }

    /// The colour sets of the Colors menu, in its order.
    pub fn builtin_colors() -> Vec<ThemeColors> {
        let first = ThemeColors::from_part_order(builtin::THEMES[0].scheme, builtin::THEMES[0].colors);
        std::iter::once(first)
            .chain(builtin::COLOR_SETS.iter().map(ThemeColors::from))
            .collect()
    }

    /// The font pairs of the Fonts menu, in its order.
    pub fn builtin_fonts() -> Vec<ThemeFonts> {
        std::iter::once(ThemeFonts::from(&builtin::THEMES[0].fonts))
            .chain(builtin::FONT_PAIRS.iter().map(ThemeFonts::from))
            .collect()
    }

    /// The value of a slot.
    pub fn rgb(&self, slot: ThemeSlot) -> u32 {
        self.colors.get(slot)
    }

    /// The colour `slot` of this theme, lighter (`tint > 0`) or darker.
    pub fn color(&self, slot: ThemeSlot, tint: f64) -> Color {
        self.colors.color(slot, tint)
    }

    /// Accent `n` (1 to 6) with a tint.
    pub fn accent(&self, n: usize, tint: f64) -> Color {
        self.color(ThemeSlot::accent(n), tint)
    }

    /// The colour a reference stands for in this theme.
    pub fn color_of(&self, reference: ThemeColor) -> Color {
        self.color(reference.slot, reference.tint)
    }

    /// `color` as it is in this theme: worked out again for a theme
    /// colour, itself otherwise.
    pub fn resolve(&self, color: Color) -> Color {
        match color.theme() {
            Some(reference) => self.color_of(reference),
            None => color,
        }
    }

    fn resolve_opt(&self, color: &mut Option<Color>) {
        if let Some(c) = color {
            *c = self.resolve(*c);
        }
    }

    pub fn resolve_border(&self, border: &mut Border) {
        self.resolve_opt(&mut border.color);
    }

    fn resolve_borders(&self, borders: &mut Borders) {
        for side in [&mut borders.left, &mut borders.right, &mut borders.top, &mut borders.bottom] {
            self.resolve_border(side);
        }
    }

    /// The name of a theme font.
    pub fn font(&self, font: ThemeFont) -> &Arc<str> {
        self.fonts.get(font)
    }

    /// Give a font that is a font of the theme the name it has in this
    /// theme, and a theme colour its value.
    pub fn resolve_font(&self, font: &mut Font) {
        if let Some(role) = font.theme {
            let name = self.font(role);
            if font.name != *name {
                font.name = name.clone();
            }
        }
        self.resolve_opt(&mut font.color);
    }

    /// Work out again everything in `style` that refers to the theme.
    pub fn resolve_style(&self, style: &mut Style) {
        self.resolve_font(&mut style.font);
        self.resolve_opt(&mut style.fill);
        self.resolve_borders(&mut style.borders);
    }

    /// What a style keeps when it enters the style table: theme colours
    /// take their value in this theme, and a font that was given another
    /// name than the theme font it says it is, is no longer a theme font.
    pub(crate) fn settle_style(&self, style: &mut Style) {
        if let Some(role) = style.font.theme
            && style.font.name != *self.font(role)
        {
            style.font.theme = None;
        }
        self.resolve_opt(&mut style.font.color);
        self.resolve_opt(&mut style.fill);
        self.resolve_borders(&mut style.borders);
    }

    /// Work out again the theme colours and the theme font of a shape.
    pub fn resolve_shape(&self, shape: &mut Shape) {
        self.resolve_opt(&mut shape.fill);
        if let Some(outline) = &mut shape.outline {
            outline.color = self.resolve(outline.color);
        }
        self.resolve_font(&mut shape.font);
        // The letters: their outline, their shadow, and each run.
        let effects = |effects: &mut crate::wordart::TextEffects| {
            if let Some(outline) = &mut effects.outline {
                outline.color = self.resolve(outline.color);
            }
            if let Some(shadow) = &mut effects.shadow {
                shadow.color = self.resolve(shadow.color);
            }
        };
        effects(&mut shape.effects);
        for run in &mut shape.runs {
            self.resolve_font(&mut run.font);
            effects(&mut run.effects);
        }
    }

    /// Work out again the theme colours of a partial style.
    pub fn resolve_patch(&self, patch: &mut StylePatch) {
        if let Some(color) = &mut patch.font_color {
            self.resolve_opt(color);
        }
        if let Some(color) = &mut patch.fill {
            self.resolve_opt(color);
        }
        for border in [
            &mut patch.border_left,
            &mut patch.border_right,
            &mut patch.border_top,
            &mut patch.border_bottom,
        ]
        .into_iter()
        .flatten()
        {
            self.resolve_border(border);
        }
    }

    /// The theme colours as a colour picker lays them out: a row of the
    /// ten colours (background 1, text 1, background 2, text 2, the six
    /// accents), then five rows of lighter and darker variants.
    pub fn palette(&self) -> Vec<[Color; 10]> {
        let slots: [ThemeSlot; 10] = std::array::from_fn(|i| ThemeSlot::ALL[i]);
        let mut rows = vec![slots.map(|slot| self.color(slot, 0.0))];
        for row in 0..5 {
            rows.push(slots.map(|slot| self.color(slot, picker_tints(self.rgb(slot))[row])));
        }
        rows
    }

    /// The automatic colour of the `index`-th series of a chart, or slice
    /// of a pie: the six accents, then lighter and darker rounds of them.
    /// It is a colour of its own: an automatic colour is not stored, and
    /// is asked for again when the theme has changed.
    pub fn chart_color(&self, index: usize) -> Color {
        let accents = ThemeSlot::ACCENTS.map(|slot| self.rgb(slot));
        crate::chart::palette_color_of(&accents, index)
    }

    /// True for the colours and fonts of the Office theme, which built-in
    /// styles are worked out for once and for all.
    pub fn is_office(&self) -> bool {
        let office = &builtin::THEMES[0];
        self.colors.part_order() == office.colors
            && *self.fonts.major == *office.fonts.major
            && *self.fonts.minor == *office.fonts.minor
    }
}

/// The five variants a picker shows under a theme colour, as tints. They
/// depend on how light the colour is: white only has darker variants,
/// black only lighter ones.
pub fn picker_tints(rgb: u32) -> [f64; 5] {
    let channels = [16, 8, 0].map(|shift| (rgb >> shift) & 0xFF);
    let max = channels.into_iter().max().unwrap_or(0);
    let min = channels.into_iter().min().unwrap_or(0);
    // Lightness, in 1/510.
    match max + min {
        0 => [0.5, 0.35, 0.25, 0.15, 0.05],
        l if l < 102 => [0.9, 0.75, 0.5, 0.25, 0.1],
        510 => [-0.05, -0.15, -0.25, -0.35, -0.5],
        l if l > 408 => [-0.1, -0.25, -0.5, -0.75, -0.9],
        _ => [0.8, 0.6, 0.4, -0.25, -0.5],
    }
}

/// What a picker calls a variant: how many percent lighter (positive) or
/// darker (negative) than the theme colour.
pub fn tint_percent(tint: f64) -> i32 {
    (tint * 100.0).round() as i32
}

/// Lighten (`amount > 0`) or darken (`amount < 0`) a colour the way Excel
/// does for the tints of theme colours: on the luminance of the Windows
/// colour dialog's integer HLS space (0..=240), which is why its shades
/// differ by a unit or two from a floating-point HSL computation. The
/// result is a colour of its own, not a theme colour.
pub fn tint(color: Color, amount: f64) -> Color {
    Color(tint_rgb(color.value() & 0xFF_FFFF, amount))
}

fn tint_rgb(rgb: u32, amount: f64) -> u32 {
    const HLS_MAX: i32 = 240;
    const RGB_MAX: i32 = 255;
    if amount == 0.0 || !amount.is_finite() {
        return rgb;
    }
    let (r, g, b) = (((rgb >> 16) & 0xFF) as i32, ((rgb >> 8) & 0xFF) as i32, (rgb & 0xFF) as i32);
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
        let hue = if r == max {
            delta(b) - delta(g)
        } else if g == max {
            HLS_MAX / 3 + delta(r) - delta(b)
        } else {
            2 * HLS_MAX / 3 + delta(g) - delta(r)
        };
        (hue.rem_euclid(HLS_MAX), sat)
    };
    let amount = amount.clamp(-1.0, 1.0);
    let scaled = if amount < 0.0 {
        lum as f64 * (1.0 + amount)
    } else {
        lum as f64 * (1.0 - amount) + HLS_MAX as f64 * amount
    };
    // Files keep the nominal percentage cut to 1/32767 (80 % is
    // 0.79998...); the bias keeps that from landing a hair under a whole
    // unit of luminance.
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
    let channel = |h: i32| {
        let h = h.rem_euclid(HLS_MAX);
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
    (byte(channel(hue + HLS_MAX / 3)) << 16) | (byte(channel(hue)) << 8) | byte(channel(hue - HLS_MAX / 3))
}

// ---- the theme of a workbook ------------------------------------------------

impl Workbook {
    /// The theme of the workbook.
    pub fn theme(&self) -> &Theme {
        self.styles.theme()
    }

    /// The theme, to keep or to compare (it is shared, not copied).
    pub fn theme_arc(&self) -> Arc<Theme> {
        self.styles.theme_arc()
    }

    /// Change the theme: every theme colour and theme font of the
    /// workbook follows, and so do the columns and rows that take their
    /// size from the default font. Part of the current transaction: the
    /// caller commits, and the whole change is one undo step.
    pub fn set_theme(&mut self, theme: Theme) {
        if *self.theme() == theme {
            return;
        }
        self.snapshot();
        let before = self.font_metrics();
        self.styles.set_theme(Arc::new(theme));
        let after = self.font_metrics();
        let theme = self.theme_arc();
        for style in &mut self.cell_styles {
            theme.resolve_style(&mut style.style);
        }
        for sheet in &mut self.sheets {
            retheme_sheet(&theme, sheet);
            resize_for_font(sheet, &before, &after);
        }
    }

    /// Change the colours of the theme and leave the rest.
    pub fn set_theme_colors(&mut self, colors: ThemeColors) {
        let mut theme = self.theme().clone();
        theme.colors = colors;
        self.set_theme(theme);
    }

    /// Change the fonts of the theme and leave the rest.
    pub fn set_theme_fonts(&mut self, fonts: ThemeFonts) {
        let mut theme = self.theme().clone();
        theme.fonts = fonts;
        self.set_theme(theme);
    }

    /// Change the effects of the theme and leave the rest.
    pub fn set_theme_effects(&mut self, effects: &str) {
        let mut theme = self.theme().clone();
        theme.effects = effects.to_string();
        self.set_theme(theme);
    }

    /// Give a workbook that is being built (by a file reader) its theme:
    /// nothing follows, there is no undo step. The styles interned from
    /// then on are resolved against it.
    pub fn init_theme(&mut self, theme: Theme) {
        self.styles.set_theme(Arc::new(theme));
    }

    /// Put back the theme of an undo state. The sheets of that state come
    /// back with their own colours; the style table, which is shared by
    /// all states, is worked out again.
    pub(crate) fn restore_theme(&mut self, theme: &Arc<Theme>) {
        if **theme != *self.theme() {
            self.styles.set_theme(theme.clone());
        }
    }
}

/// Work out again the theme colours kept on a sheet: its tab, its
/// conditional formats, its filters by colour, its sparklines, its charts
/// and its shapes. (Cells are in the
/// style table; tables and PivotTables name a style, which is computed
/// from the theme when it is drawn.)
fn retheme_sheet(theme: &Theme, sheet: &mut Sheet) {
    theme.resolve_opt(&mut sheet.tab_color);
    for format in &mut sheet.cond_formats {
        theme.resolve_patch(&mut format.format);
        match &mut format.rule {
            Rule::ColorScale(scale) => {
                for stop in &mut scale.stops {
                    stop.color = theme.resolve(stop.color);
                }
            }
            Rule::DataBar(bar) => {
                bar.fill = theme.resolve(bar.fill);
                bar.axis_color = theme.resolve(bar.axis_color);
                for color in [&mut bar.border, &mut bar.negative_fill, &mut bar.negative_border] {
                    theme.resolve_opt(color);
                }
            }
            _ => {}
        }
    }
    // A filter by colour written with a colour of the theme (files do).
    let table_filters = sheet.tables.iter_mut().filter_map(|table| table.auto_filter.as_mut());
    for filter in sheet.auto_filter.iter_mut().chain(table_filters) {
        for criteria in filter.columns.values_mut() {
            if let Criteria::Color(by) = criteria {
                theme.resolve_opt(&mut by.color);
            }
        }
    }
    for group in &mut sheet.sparklines {
        let colors = &mut group.colors;
        for color in [
            &mut colors.series,
            &mut colors.negative,
            &mut colors.axis,
            &mut colors.markers,
            &mut colors.first,
            &mut colors.last,
            &mut colors.high,
            &mut colors.low,
        ] {
            *color = theme.resolve(*color);
        }
    }
    for chart in &mut sheet.charts {
        retheme_chart(theme, chart);
    }
    for drawing in &mut sheet.drawings {
        drawing.retheme(theme);
    }
}

fn retheme_chart(theme: &Theme, chart: &mut Chart) {
    for area in [&mut chart.chart_area, &mut chart.plot_area] {
        if let Fill::Solid(color) = &mut area.fill {
            *color = theme.resolve(*color);
        }
    }
    for series in &mut chart.series {
        theme.resolve_opt(&mut series.color);
        theme.resolve_opt(&mut series.line.color);
        if let Some(marker) = &mut series.marker {
            theme.resolve_opt(&mut marker.fill);
            theme.resolve_opt(&mut marker.line);
        }
        for point in &mut series.points {
            theme.resolve_opt(&mut point.color);
        }
        for trendline in &mut series.trendlines {
            theme.resolve_opt(&mut trendline.line.color);
        }
    }
    // The colours of the texts, of the axis lines and of the gridlines.
    for text in chart.text_formats_mut() {
        theme.resolve_opt(&mut text.color);
    }
    let axes = &mut chart.axes;
    for axis in [&mut axes.category, &mut axes.value, &mut axes.secondary_category, &mut axes.secondary_value] {
        for line in [&mut axis.line, &mut axis.major_gridline, &mut axis.minor_gridline] {
            theme.resolve_opt(&mut line.color);
        }
    }
}

/// Columns are as wide as a number of digits of the default font and
/// rows nobody sized are as high as a line of it: when the theme brings
/// another default font, the columns keep their number of characters and
/// the default sizes become those of the new font, as in Excel. Rows that
/// were given a height keep it.
fn resize_for_font(sheet: &mut Sheet, before: &FontMetrics, after: &FontMetrics) {
    if before == after {
        return;
    }
    let rescale = |px: f32| {
        let chars = (px as f64 - 5.0) / before.digit_width.max(1.0);
        ((chars * after.digit_width + 5.0).round() as f32).clamp(0.0, after.max_col_width())
    };
    if (sheet.default_col_width - before.col_width).abs() < 0.01 {
        sheet.default_col_width = after.col_width;
    } else {
        sheet.default_col_width = rescale(sheet.default_col_width).max(1.0);
    }
    if before.digit_width != after.digit_width {
        for width in sheet.col_widths.values_mut() {
            // A hidden column (no width at all) stays hidden.
            if *width > 0.0 {
                *width = rescale(*width);
            }
        }
    }
    if (sheet.default_row_height - before.row_height).abs() < 0.01 {
        sheet.default_row_height = after.row_height;
    }
}

#[cfg(test)]
mod tests;
