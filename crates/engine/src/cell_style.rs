//! Cell styles: the named formats of the Cell Styles gallery.
//!
//! A cell style is a name, a format and the parts of that format it
//! brings ("Style includes"): applying it to a cell overwrites those
//! parts and leaves the others. The cell remembers the name
//! (`Style::cell_style`), so changing the style later changes the cells
//! that still have what the style gave them.
//!
//! Excel's built-in styles are a catalogue computed from the theme; the
//! workbook keeps only the styles the user made and the built-in ones that
//! were changed (`Workbook::cell_styles`).

use crate::ast::same_name;
use crate::refs::Rect;
use crate::style::{
    Border, BorderStyle, Borders, Color, Style, StyleId, ThemeFont, ThemeSlot, Underline,
};
use crate::theme::Theme;
use crate::workbook::Workbook;
use std::collections::HashMap;
use std::sync::Arc;

/// The name of the style every cell has until it is given another.
pub const NORMAL: &str = "Normal";

/// The parts of its format a style brings: the six check boxes of "Style
/// includes".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StyleParts {
    pub number: bool,
    pub alignment: bool,
    pub font: bool,
    pub border: bool,
    pub fill: bool,
    pub protection: bool,
}

impl StyleParts {
    pub const ALL: StyleParts = StyleParts {
        number: true,
        alignment: true,
        font: true,
        border: true,
        fill: true,
        protection: true,
    };

    pub const NONE: StyleParts = StyleParts {
        number: false,
        alignment: false,
        font: false,
        border: false,
        fill: false,
        protection: false,
    };
}

impl Default for StyleParts {
    fn default() -> Self {
        StyleParts::ALL
    }
}

/// One part of a format.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Part {
    Number,
    Alignment,
    Font,
    Border,
    Fill,
    Protection,
}

const PARTS: [Part; 6] = [
    Part::Number,
    Part::Alignment,
    Part::Font,
    Part::Border,
    Part::Fill,
    Part::Protection,
];

impl StyleParts {
    fn has(&self, part: Part) -> bool {
        match part {
            Part::Number => self.number,
            Part::Alignment => self.alignment,
            Part::Font => self.font,
            Part::Border => self.border,
            Part::Fill => self.fill,
            Part::Protection => self.protection,
        }
    }
}

fn same_part(a: &Style, b: &Style, part: Part) -> bool {
    match part {
        Part::Number => a.num_fmt == b.num_fmt,
        Part::Alignment => {
            (
                a.h_align,
                a.v_align,
                a.wrap,
                a.shrink_to_fit,
                a.indent,
                a.rotation,
            ) == (
                b.h_align,
                b.v_align,
                b.wrap,
                b.shrink_to_fit,
                b.indent,
                b.rotation,
            )
        }
        Part::Font => a.font == b.font,
        Part::Border => a.borders == b.borders,
        Part::Fill => a.fill == b.fill,
        Part::Protection => (a.locked, a.hidden) == (b.locked, b.hidden),
    }
}

fn copy_part(to: &mut Style, from: &Style, part: Part) {
    match part {
        Part::Number => to.num_fmt = from.num_fmt.clone(),
        Part::Alignment => {
            to.h_align = from.h_align;
            to.v_align = from.v_align;
            to.wrap = from.wrap;
            to.shrink_to_fit = from.shrink_to_fit;
            to.indent = from.indent;
            to.rotation = from.rotation;
        }
        Part::Font => to.font = from.font.clone(),
        Part::Border => to.borders = from.borders,
        Part::Fill => to.fill = from.fill,
        Part::Protection => {
            to.locked = from.locked;
            to.hidden = from.hidden;
        }
    }
}

/// Where the gallery shows a style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleGroup {
    /// Made by the user.
    Custom,
    GoodBadNeutral,
    DataModel,
    TitlesHeadings,
    Themed,
    NumberFormat,
}

/// A named format.
#[derive(Clone, Debug, PartialEq)]
pub struct CellStyle {
    /// The name; for a built-in style, its English name (files and the
    /// interface give it in their language).
    pub name: String,
    /// Excel's number for a built-in style (`builtinId`).
    pub builtin: Option<u32>,
    /// The whole format; only the parts of `includes` count.
    pub style: Style,
    pub includes: StyleParts,
}

impl CellStyle {
    /// Where the gallery shows the style.
    pub fn group(&self) -> StyleGroup {
        match self.builtin {
            None => StyleGroup::Custom,
            Some(0 | 26..=28) => StyleGroup::GoodBadNeutral,
            Some(3..=7) => StyleGroup::NumberFormat,
            Some(15..=19 | 25) => StyleGroup::TitlesHeadings,
            Some(29..=52) => StyleGroup::Themed,
            Some(_) => StyleGroup::DataModel,
        }
    }

    /// The format a cell formatted `onto` has once the style is applied:
    /// the parts the style includes are the style's.
    pub fn applied(&self, onto: &Style) -> Style {
        let mut out = onto.clone();
        for part in PARTS.into_iter().filter(|part| self.includes.has(*part)) {
            copy_part(&mut out, &self.style, part);
        }
        out.cell_style = (!same_name(&self.name, NORMAL)).then(|| Arc::from(self.name.as_str()));
        out
    }
}

/// Why a style cannot be made or changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StyleError {
    EmptyName,
    /// Another style has the name.
    NameTaken,
    /// No style of that name.
    Unknown,
    /// The Normal style stays as it is and where it is.
    Normal,
}

// ---- the built-in styles --------------------------------------------------------

fn rgb(hex: u32) -> Option<Color> {
    Some(Color(hex))
}

fn all_sides(style: BorderStyle, color: u32) -> Borders {
    let side = Border {
        style,
        color: rgb(color),
    };
    Borders {
        left: side,
        right: side,
        top: side,
        bottom: side,
    }
}

fn bottom(style: BorderStyle, color: Color) -> Borders {
    Borders {
        bottom: Border {
            style,
            color: Some(color),
        },
        ..Borders::default()
    }
}

const COMMA: &str = "_(* #,##0.00_);_(* \\(#,##0.00\\);_(* \"-\"??_);_(@_)";
const COMMA_0: &str = "_(* #,##0_);_(* \\(#,##0\\);_(* \"-\"_);_(@_)";
const CURRENCY: &str = "_($* #,##0.00_);_($* \\(#,##0.00\\);_($* \"-\"??_);_(@_)";
const CURRENCY_0: &str = "_($* #,##0_);_($* \\(#,##0\\);_($* \"-\"_);_(@_)";

/// The colours of links: the two link colours of the theme, here with
/// the values they have in the Office theme.
pub const LINK_COLOR: Color = Color::themed(ThemeSlot::Hyperlink, 0.0, 0x467886);
pub const FOLLOWED_COLOR: Color = Color::themed(ThemeSlot::FollowedHyperlink, 0.0, 0x96607D);

/// Excel's built-in styles in the order of the gallery, as they are in a
/// workbook of the Office theme whose Normal style is `normal`.
pub fn builtin_styles(normal: &Style) -> Vec<CellStyle> {
    builtin_styles_in(&Theme::office(), normal)
}

/// Excel's built-in styles in the order of the gallery, as they are in a
/// workbook of theme `theme` whose Normal style is `normal`. What they
/// take from the theme, they refer to: cells given such a style follow
/// the theme.
pub fn builtin_styles_in(theme: &Theme, normal: &Style) -> Vec<CellStyle> {
    let accent = |n: usize, lighter: f64| theme.accent(n, lighter);
    let dark2 = theme.color(ThemeSlot::Dark2, 0.0);
    let mut out: Vec<CellStyle> = Vec::new();
    let mut add = |name: &str, id: u32, includes: StyleParts, set: &dyn Fn(&mut Style)| {
        let mut style = normal.clone();
        style.cell_style = None;
        set(&mut style);
        out.push(CellStyle {
            name: name.to_string(),
            builtin: Some(id),
            style,
            includes,
        });
    };
    let font = StyleParts {
        font: true,
        ..StyleParts::NONE
    };
    let font_fill = StyleParts { fill: true, ..font };
    let font_border = StyleParts {
        border: true,
        ..font
    };
    let boxed = StyleParts {
        border: true,
        ..font_fill
    };
    let number = StyleParts {
        number: true,
        ..StyleParts::NONE
    };

    // Good, Bad and Neutral
    add(NORMAL, 0, StyleParts::ALL, &|_| {});
    add("Bad", 27, font_fill, &|s| {
        (s.font.color, s.fill) = (rgb(0x9C0006), rgb(0xFFC7CE))
    });
    add("Good", 26, font_fill, &|s| {
        (s.font.color, s.fill) = (rgb(0x006100), rgb(0xC6EFCE))
    });
    add("Neutral", 28, font_fill, &|s| {
        (s.font.color, s.fill) = (rgb(0x9C5700), rgb(0xFFEB9C))
    });
    // Data and Model
    add("Calculation", 22, boxed, &|s| {
        s.font.bold = true;
        (s.font.color, s.fill) = (rgb(0xFA7D00), rgb(0xF2F2F2));
        s.borders = all_sides(BorderStyle::Thin, 0x7F7F7F);
    });
    add("Check Cell", 23, boxed, &|s| {
        s.font.bold = true;
        (s.font.color, s.fill) = (rgb(0xFFFFFF), rgb(0xA5A5A5));
        s.borders = all_sides(BorderStyle::Double, 0x3F3F3F);
    });
    add("Explanatory Text", 53, font, &|s| {
        s.font.italic = true;
        s.font.color = rgb(0x7F7F7F);
    });
    add("Followed Hyperlink", 9, font, &|s| {
        s.font.color = Some(theme.color(ThemeSlot::FollowedHyperlink, 0.0));
        s.font.underline = Underline::Single;
    });
    add("Hyperlink", 8, font, &|s| {
        s.font.color = Some(theme.color(ThemeSlot::Hyperlink, 0.0));
        s.font.underline = Underline::Single;
    });
    add("Input", 20, boxed, &|s| {
        (s.font.color, s.fill) = (rgb(0x3F3F76), rgb(0xFFCC99));
        s.borders = all_sides(BorderStyle::Thin, 0x7F7F7F);
    });
    add("Linked Cell", 24, font_border, &|s| {
        s.font.color = rgb(0xFA7D00);
        s.borders = bottom(BorderStyle::Double, Color(0xFF8001));
    });
    add(
        "Note",
        10,
        StyleParts {
            border: true,
            fill: true,
            ..StyleParts::NONE
        },
        &|s| {
            s.fill = rgb(0xFFFFCC);
            s.borders = all_sides(BorderStyle::Thin, 0xB2B2B2);
        },
    );
    add("Output", 21, boxed, &|s| {
        s.font.bold = true;
        (s.font.color, s.fill) = (rgb(0x3F3F3F), rgb(0xF2F2F2));
        s.borders = all_sides(BorderStyle::Thin, 0x3F3F3F);
    });
    add("Warning Text", 11, font, &|s| s.font.color = rgb(0xFF0000));
    // Titles and Headings
    let heading = |size: f32, border: Option<(BorderStyle, f64)>| {
        move |s: &mut Style| {
            s.font.bold = true;
            s.font.color = Some(dark2);
            s.font.set_size_pt(size);
            if let Some((line, lighter)) = border {
                s.borders = bottom(line, accent(1, lighter));
            }
        }
    };
    add(
        "Heading 1",
        16,
        font_border,
        &heading(15.0, Some((BorderStyle::Thick, 0.0))),
    );
    add(
        "Heading 2",
        17,
        font_border,
        &heading(13.0, Some((BorderStyle::Thick, 0.5))),
    );
    add(
        "Heading 3",
        18,
        font_border,
        &heading(11.0, Some((BorderStyle::Medium, 0.4))),
    );
    add("Heading 4", 19, font, &heading(11.0, None));
    add("Title", 15, font, &|s| {
        s.font.name = theme.font(ThemeFont::Major).clone();
        s.font.theme = Some(ThemeFont::Major);
        s.font.color = Some(dark2);
        s.font.set_size_pt(18.0);
    });
    add("Total", 25, font_border, &|s| {
        s.font.bold = true;
        s.borders = Borders {
            top: Border {
                style: BorderStyle::Thin,
                color: Some(accent(1, 0.0)),
            },
            bottom: Border {
                style: BorderStyle::Double,
                color: Some(accent(1, 0.0)),
            },
            ..Borders::default()
        };
    });
    // Themed Cell Styles: a row of six accents for each strength.
    for (label, lighter, first_id) in [
        ("20% - ", 0.8, 30),
        ("40% - ", 0.6, 31),
        ("60% - ", 0.4, 32),
        ("", 0.0, 29),
    ] {
        for n in 1..=6u32 {
            let fill = accent(n as usize, lighter);
            // Only the full accent is dark enough for white text.
            let text = if lighter == 0.0 { rgb(0xFFFFFF) } else { None };
            add(
                &format!("{label}Accent{n}"),
                first_id + 4 * (n - 1),
                font_fill,
                &|s| {
                    s.fill = Some(fill);
                    s.font.color = text;
                },
            );
        }
    }
    // Number Format
    add("Comma", 3, number, &|s| s.num_fmt = Arc::from(COMMA));
    add("Comma [0]", 6, number, &|s| s.num_fmt = Arc::from(COMMA_0));
    add("Currency", 4, number, &|s| s.num_fmt = Arc::from(CURRENCY));
    add("Currency [0]", 7, number, &|s| {
        s.num_fmt = Arc::from(CURRENCY_0)
    });
    add("Percent", 5, number, &|s| s.num_fmt = Arc::from("0%"));
    out
}

// ---- the styles of a workbook ---------------------------------------------------

impl Workbook {
    /// Every style of the workbook in the order of the gallery: those the
    /// user made (by name), then Excel's, as changed here.
    pub fn all_cell_styles(&self) -> Vec<CellStyle> {
        let mut custom: Vec<CellStyle> = self
            .cell_styles
            .iter()
            .filter(|style| style.builtin.is_none())
            .cloned()
            .collect();
        custom.sort_by_key(|style| style.name.to_lowercase());
        let builtin = builtin_styles_in(self.theme(), self.styles.default_style())
            .into_iter()
            .map(|style| {
                let changed = self
                    .cell_styles
                    .iter()
                    .find(|own| own.builtin == style.builtin);
                changed.cloned().unwrap_or(style)
            });
        custom.into_iter().chain(builtin).collect()
    }

    /// The style of a name (names are compared without case).
    pub fn cell_style(&self, name: &str) -> Option<CellStyle> {
        if let Some(own) = self
            .cell_styles
            .iter()
            .find(|style| same_name(&style.name, name))
        {
            return Some(own.clone());
        }
        builtin_styles_in(self.theme(), self.styles.default_style())
            .into_iter()
            .find(|style| same_name(&style.name, name))
    }

    /// The name of the style of a cell.
    pub fn cell_style_at(&self, sheet: usize, row: u32, col: u32) -> &str {
        self.style_at(sheet, row, col)
            .cell_style
            .as_deref()
            .unwrap_or(NORMAL)
    }

    /// Give cells a style: the parts it includes replace theirs.
    pub fn apply_cell_style(
        &mut self,
        sheet: usize,
        ranges: &[Rect],
        name: &str,
    ) -> Result<(), StyleError> {
        let style = self.cell_style(name).ok_or(StyleError::Unknown)?;
        for range in ranges {
            self.update_style(sheet, *range, &|cell| *cell = style.applied(cell));
        }
        Ok(())
    }

    /// Replace every cell format `f` gives another for, on all sheets.
    /// The caller has taken the snapshot.
    fn remap_styles(&mut self, f: &dyn Fn(&Style) -> Option<Style>) {
        let mut map: HashMap<StyleId, StyleId> = HashMap::new();
        for index in 0..self.styles.len() as u32 {
            if let Some(new) = f(self.styles.get(StyleId(index))) {
                let id = self.styles.intern(new);
                map.insert(StyleId(index), id);
            }
        }
        if map.is_empty() {
            return;
        }
        for sheet in &mut self.sheets {
            for cell in sheet.cells.values_mut() {
                if let Some(new) = map.get(&cell.style) {
                    cell.style = *new;
                }
            }
            for id in sheet
                .col_styles
                .values_mut()
                .chain(sheet.row_styles.values_mut())
            {
                if let Some(new) = map.get(id) {
                    *id = *new;
                }
            }
        }
    }

    /// Add a style of the user's ("New Cell Style").
    pub fn add_cell_style(
        &mut self,
        name: &str,
        style: Style,
        includes: StyleParts,
    ) -> Result<(), StyleError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(StyleError::EmptyName);
        }
        if self.cell_style(name).is_some() {
            return Err(StyleError::NameTaken);
        }
        self.snapshot();
        self.cell_styles.push(CellStyle {
            name: name.to_string(),
            builtin: None,
            style: Style {
                cell_style: None,
                ..style
            },
            includes,
        });
        Ok(())
    }

    /// Change a style ("Modify"): the cells that have it follow, part by
    /// part, wherever they still have what the style gave them.
    pub fn modify_cell_style(
        &mut self,
        name: &str,
        style: Style,
        includes: StyleParts,
    ) -> Result<(), StyleError> {
        let old = self.cell_style(name).ok_or(StyleError::Unknown)?;
        if old.builtin == Some(0) {
            return Err(StyleError::Normal);
        }
        let new = CellStyle {
            style: Style {
                cell_style: None,
                ..style
            },
            includes,
            ..old.clone()
        };
        if new == old {
            return Ok(());
        }
        self.snapshot();
        self.remap_styles(&|cell| {
            if !cell
                .cell_style
                .as_deref()
                .is_some_and(|own| same_name(own, &old.name))
            {
                return None;
            }
            let mut out = cell.clone();
            for part in PARTS {
                let followed = !old.includes.has(part) || same_part(cell, &old.style, part);
                if new.includes.has(part) && followed {
                    copy_part(&mut out, &new.style, part);
                }
            }
            Some(out)
        });
        match self
            .cell_styles
            .iter_mut()
            .find(|own| same_name(&own.name, &old.name))
        {
            Some(own) => *own = new,
            None => self.cell_styles.push(new),
        }
        Ok(())
    }

    /// Delete a style of the user's, or put a built-in one back as Excel
    /// has it. The cells that had it are Normal again where they still
    /// had what the style gave them.
    pub fn delete_cell_style(&mut self, name: &str) -> Result<(), StyleError> {
        let old = self.cell_style(name).ok_or(StyleError::Unknown)?;
        if old.builtin == Some(0) {
            return Err(StyleError::Normal);
        }
        let normal = self.styles.default_style().clone();
        self.snapshot();
        self.remap_styles(&|cell| {
            if !cell
                .cell_style
                .as_deref()
                .is_some_and(|own| same_name(own, &old.name))
            {
                return None;
            }
            let mut out = cell.clone();
            for part in PARTS {
                if old.includes.has(part) && same_part(cell, &old.style, part) {
                    copy_part(&mut out, &normal, part);
                }
            }
            out.cell_style = None;
            Some(out)
        });
        self.cell_styles
            .retain(|own| !same_name(&own.name, &old.name));
        Ok(())
    }

    /// A free name for a copy of a style ("Duplicate"): `Good 2`.
    pub fn duplicate_style_name(&self, name: &str) -> String {
        (2..)
            .map(|n| format!("{name} {n}"))
            .find(|candidate| self.cell_style(candidate).is_none())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::wb;

    fn rect(name: &str) -> Rect {
        Rect::parse(name).unwrap()
    }

    #[test]
    fn the_catalogue_has_excels_styles_in_gallery_order() {
        let styles = builtin_styles(&Style::default());
        assert_eq!(styles.len(), 49);
        let names: Vec<&str> = styles.iter().map(|style| style.name.as_str()).collect();
        assert_eq!(&names[..4], ["Normal", "Bad", "Good", "Neutral"]);
        assert_eq!(
            &names[20..26],
            [
                "20% - Accent1",
                "20% - Accent2",
                "20% - Accent3",
                "20% - Accent4",
                "20% - Accent5",
                "20% - Accent6"
            ]
        );
        assert_eq!(
            &names[38..],
            [
                "Accent1",
                "Accent2",
                "Accent3",
                "Accent4",
                "Accent5",
                "Accent6",
                "Comma",
                "Comma [0]",
                "Currency",
                "Currency [0]",
                "Percent"
            ]
        );
        // Excel's numbers, each once.
        let mut ids: Vec<u32> = styles.iter().filter_map(|style| style.builtin).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 49);
        let id = |name: &str| {
            styles
                .iter()
                .find(|style| style.name == name)
                .and_then(|style| style.builtin)
        };
        assert_eq!(
            (
                id("Accent1"),
                id("60% - Accent1"),
                id("Accent6"),
                id("60% - Accent6")
            ),
            (Some(29), Some(32), Some(49), Some(52))
        );
        assert_eq!(
            (
                id("Good"),
                id("Heading 4"),
                id("Percent"),
                id("Explanatory Text")
            ),
            (Some(26), Some(19), Some(5), Some(53))
        );
        let group = |name: &str| {
            styles
                .iter()
                .find(|style| style.name == name)
                .map(CellStyle::group)
        };
        assert_eq!(group("Normal"), Some(StyleGroup::GoodBadNeutral));
        assert_eq!(group("Hyperlink"), Some(StyleGroup::DataModel));
        assert_eq!(group("Total"), Some(StyleGroup::TitlesHeadings));
        assert_eq!(group("40% - Accent3"), Some(StyleGroup::Themed));
        assert_eq!(group("Comma [0]"), Some(StyleGroup::NumberFormat));
    }

    #[test]
    fn a_style_replaces_the_parts_it_includes() {
        let mut w = wb(&[("A1", "0.5")]);
        w.update_style(0, rect("A1"), &|s| {
            s.num_fmt = "0%".into();
            s.font.italic = true;
        });
        w.apply_cell_style(0, &[rect("A1:A2")], "good").unwrap();
        w.commit("style");
        let cell = w.style_at(0, 0, 0);
        assert_eq!(
            (cell.fill, cell.font.color),
            (Some(Color(0xC6EFCE)), Some(Color(0x006100)))
        );
        // The font is the style's, the number format is the cell's.
        assert!(!cell.font.italic);
        assert_eq!(&*cell.num_fmt, "0%");
        assert_eq!(w.cell_style_at(0, 0, 0), "Good");
        assert_eq!(w.cell_style_at(0, 1, 0), "Good");
        assert_eq!(w.cell_style_at(0, 5, 5), "Normal");
        // A number style changes the number format alone.
        w.apply_cell_style(0, &[rect("A1")], "Percent").unwrap();
        assert_eq!(w.style_at(0, 0, 0).fill, Some(Color(0xC6EFCE)));
        // Normal takes everything back.
        w.apply_cell_style(0, &[rect("A1")], "Normal").unwrap();
        w.commit("normal");
        assert_eq!(w.style_at(0, 0, 0), w.styles.default_style());
        assert_eq!(
            w.apply_cell_style(0, &[rect("A1")], "No such style"),
            Err(StyleError::Unknown)
        );
    }

    #[test]
    fn modifying_a_style_changes_the_cells_that_follow_it() {
        let mut w = wb(&[("A1", "x"), ("A2", "y"), ("A3", "z")]);
        w.apply_cell_style(0, &[rect("A1:A2")], "Good").unwrap();
        // A2 gets a fill of its own.
        w.update_style(0, rect("A2"), &|s| s.fill = Some(Color(0xFFFF00)));
        w.commit("setup");
        let mut changed = w.cell_style("Good").unwrap();
        changed.style.fill = Some(Color(0x00FF00));
        changed.style.font.bold = true;
        w.modify_cell_style("Good", changed.style.clone(), changed.includes)
            .unwrap();
        w.commit("modify");
        assert_eq!(w.style_at(0, 0, 0).fill, Some(Color(0x00FF00)));
        assert!(w.style_at(0, 0, 0).font.bold);
        // The fill A2 was given by hand stays; its font follows.
        assert_eq!(w.style_at(0, 1, 0).fill, Some(Color(0xFFFF00)));
        assert!(w.style_at(0, 1, 0).font.bold);
        assert!(!w.style_at(0, 2, 0).font.bold);
        // The gallery shows the style as it is now, where it was.
        let listed = w.all_cell_styles();
        assert_eq!(listed[2].name, "Good");
        assert_eq!(listed[2].style.fill, Some(Color(0x00FF00)));
        // Undo brings back both the cells and the style.
        assert!(w.undo());
        assert_eq!(w.style_at(0, 0, 0).fill, Some(Color(0xC6EFCE)));
        assert_eq!(
            w.cell_style("Good").unwrap().style.fill,
            Some(Color(0xC6EFCE))
        );
        assert!(w.cell_styles.is_empty());
        assert_eq!(
            w.modify_cell_style("Normal", Style::default(), StyleParts::ALL),
            Err(StyleError::Normal)
        );
    }

    #[test]
    fn styles_of_the_user_are_made_duplicated_and_deleted() {
        let mut w = wb(&[("A1", "x")]);
        let mut look = w.styles.default_style().clone();
        look.fill = Some(Color(0x123456));
        look.font.bold = true;
        let fill_only = StyleParts {
            fill: true,
            ..StyleParts::NONE
        };
        w.add_cell_style(" Mine ", look.clone(), fill_only).unwrap();
        w.commit("new style");
        assert_eq!(
            w.add_cell_style("mine", look.clone(), fill_only),
            Err(StyleError::NameTaken)
        );
        assert_eq!(
            w.add_cell_style("GOOD", look.clone(), fill_only),
            Err(StyleError::NameTaken)
        );
        assert_eq!(
            w.add_cell_style("  ", look.clone(), fill_only),
            Err(StyleError::EmptyName)
        );
        assert_eq!(w.all_cell_styles()[0].name, "Mine");
        assert_eq!(w.all_cell_styles()[0].group(), StyleGroup::Custom);
        assert_eq!(w.duplicate_style_name("Mine"), "Mine 2");
        w.apply_cell_style(0, &[rect("A1")], "Mine").unwrap();
        w.commit("apply");
        // Only the fill came: the style does not include its font.
        assert_eq!(w.style_at(0, 0, 0).fill, Some(Color(0x123456)));
        assert!(!w.style_at(0, 0, 0).font.bold);
        w.delete_cell_style("Mine").unwrap();
        w.commit("delete");
        assert_eq!(w.style_at(0, 0, 0), w.styles.default_style());
        assert_eq!(w.cell_style("Mine"), None);
        assert!(w.undo());
        assert_eq!(w.cell_style_at(0, 0, 0), "Mine");
        assert!(w.cell_style("Mine").is_some());
        assert_eq!(w.delete_cell_style("Normal"), Err(StyleError::Normal));
        // Deleting a built-in style that was changed puts Excel's back.
        let mut good = w.cell_style("Good").unwrap();
        good.style.fill = None;
        w.modify_cell_style("Good", good.style, good.includes)
            .unwrap();
        w.delete_cell_style("Good").unwrap();
        w.commit("reset");
        assert_eq!(
            w.cell_style("Good").unwrap().style.fill,
            Some(Color(0xC6EFCE))
        );
    }
}
