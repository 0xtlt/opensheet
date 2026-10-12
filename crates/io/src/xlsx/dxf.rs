//! Differential formats (`<dxf>`): formatting laid over that of a cell by
//! a conditional format, stating only what it changes.
//!
//! They live in the `<dxfs>` list of `styles.xml`, where rules name them
//! by position (`dxfId`), each stored once however many rules use it; the
//! rules of the worksheet's extension list carry theirs inline
//! (`<x14:dxf>`). A dxf looks like a cell format written out in full,
//! with one trap: a solid fill keeps its colour in `bgColor`, where the
//! fill of a cell has it in `fgColor`.

use super::color::{Palette, RawColor, blend};
use super::numfmt::{FIRST_CUSTOM_ID, builtin_code, builtin_id};
use super::xml::{Event, Reader, Tag, push_attr};
use opensheet_engine::style::{Border, BorderStyle, StylePatch, Underline};
use opensheet_engine::{Color, Locale};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::sync::Arc;

// ---- writing -------------------------------------------------------------------

/// The differential formats of a workbook being written: whoever needs
/// one asks for its id, and the list goes into `styles.xml` at the end.
pub(crate) struct Dxfs {
    list: Vec<StylePatch>,
    index: HashMap<StylePatch, u32>,
    /// The custom number formats of the cell formats, which have their
    /// ids in `styles.xml` already; a dxf with the same code uses the same
    /// id.
    cell_formats: Vec<String>,
    /// Custom number formats the cells do not use: those of differential
    /// formats, and those other parts asked for.
    own_formats: Vec<Arc<str>>,
    /// The formats that stand for the fill of cells (see
    /// [`Dxfs::cell_fill_id`]).
    cell_fills: HashSet<u32>,
    locale: &'static Locale,
}

fn border_style_name(style: BorderStyle) -> &'static str {
    match style {
        BorderStyle::None => "none",
        BorderStyle::Hair => "hair",
        BorderStyle::Thin => "thin",
        BorderStyle::Medium => "medium",
        BorderStyle::Thick => "thick",
        BorderStyle::Dashed => "dashed",
        BorderStyle::Dotted => "dotted",
        BorderStyle::Double => "double",
    }
}

impl Dxfs {
    pub fn new(cell_formats: Vec<String>, locale: &'static Locale) -> Dxfs {
        Dxfs {
            list: Vec::new(),
            index: HashMap::new(),
            cell_formats,
            own_formats: Vec::new(),
            cell_fills: HashSet::new(),
            locale,
        }
    }

    /// The `dxfId` of a format, which joins the list if it is new.
    pub fn id(&mut self, patch: &StylePatch) -> u32 {
        if let Some(id) = self.index.get(patch) {
            return *id;
        }
        let id = self.list.len() as u32;
        self.list.push(patch.clone());
        self.index.insert(patch.clone(), id);
        id
    }

    /// The `dxfId` of the fill a filter or a sort looks for in the cells
    /// (`None`: no fill). Excel compares it with the fill of a cell, whose
    /// colour is the foreground of a solid pattern, so this one is written
    /// with the colour on both sides: it reads the same as the fill of a
    /// conditional format, which may share it.
    pub fn cell_fill_id(&mut self, color: Option<Color>) -> u32 {
        let id = self.id(&StylePatch {
            fill: Some(color),
            ..StylePatch::default()
        });
        self.cell_fills.insert(id);
        id
    }

    /// The custom number formats the cells do not use, which follow
    /// theirs in the `<numFmts>` list of `styles.xml`. Complete once
    /// [`Dxfs::xml`] has run.
    pub fn own_formats(&self) -> &[Arc<str>] {
        &self.own_formats
    }

    /// The id a number format has in this workbook. Parts that name a
    /// format by id alone (PivotTables) ask here too.
    pub fn num_fmt_id(&mut self, code: &Arc<str>) -> u32 {
        if let Some(id) = builtin_id(code, self.locale) {
            return id;
        }
        if let Some(pos) = self.cell_formats.iter().position(|c| **c == **code) {
            return FIRST_CUSTOM_ID + pos as u32;
        }
        let pos = self
            .own_formats
            .iter()
            .position(|c| c == code)
            .unwrap_or_else(|| {
                self.own_formats.push(code.clone());
                self.own_formats.len() - 1
            });
        FIRST_CUSTOM_ID + (self.cell_formats.len() + pos) as u32
    }

    /// Append one format as an element called `element` (`dxf`, or
    /// `x14:dxf` for a rule of the extension list). The children are in
    /// the order of the schema: font, numFmt, fill, border.
    pub fn push_dxf(&mut self, x: &mut String, element: &str, patch: &StylePatch) {
        w!(x, "<{element}>");
        let font = patch.bold.is_some()
            || patch.italic.is_some()
            || patch.strike.is_some()
            || patch.underline.is_some()
            || patch.font_size.is_some()
            || patch.font_color.is_some()
            || patch.font_name.is_some();
        if font {
            x.push_str("<font>");
            for (name, flag) in [
                ("b", patch.bold),
                ("i", patch.italic),
                ("strike", patch.strike),
            ] {
                match flag {
                    Some(true) => {
                        w!(x, "<{name}/>");
                    }
                    Some(false) => {
                        w!(x, "<{name} val=\"0\"/>");
                    }
                    None => {}
                }
            }
            match patch.underline {
                Some(Underline::Single) => x.push_str("<u/>"),
                Some(Underline::Double) => x.push_str("<u val=\"double\"/>"),
                Some(Underline::None) => x.push_str("<u val=\"none\"/>"),
                None => {}
            }
            if let Some(size) = patch.font_size {
                w!(x, "<sz val=\"{}\"/>", size.max(2) as f64 / 2.0);
            }
            match patch.font_color {
                Some(Some(color)) => push_color(x, "color", color),
                Some(None) => x.push_str("<color auto=\"1\"/>"),
                None => {}
            }
            if let Some(name) = &patch.font_name {
                x.push_str("<name val=\"");
                push_attr(x, name);
                x.push_str("\"/>");
            }
            x.push_str("</font>");
        }
        if let Some(code) = &patch.num_fmt {
            w!(
                x,
                "<numFmt numFmtId=\"{}\" formatCode=\"",
                self.num_fmt_id(code)
            );
            push_attr(x, code);
            x.push_str("\"/>");
        }
        match patch.fill {
            Some(Some(color)) => {
                x.push_str("<fill><patternFill>");
                push_color(x, "bgColor", color);
                x.push_str("</patternFill></fill>");
            }
            Some(None) => x.push_str("<fill><patternFill patternType=\"none\"/></fill>"),
            None => {}
        }
        let sides = [
            ("left", patch.border_left),
            ("right", patch.border_right),
            ("top", patch.border_top),
            ("bottom", patch.border_bottom),
        ];
        if sides.iter().any(|(_, side)| side.is_some()) {
            x.push_str("<border>");
            for (name, side) in sides {
                // A side that is left alone is an element without a style;
                // one that is taken away says so.
                let Some(border) = side else {
                    w!(x, "<{name}/>");
                    continue;
                };
                w!(x, "<{name} style=\"{}\"", border_style_name(border.style));
                match (border.style, border.color) {
                    (BorderStyle::None, _) => x.push_str("/>"),
                    (_, Some(color)) => {
                        x.push('>');
                        push_color(x, "color", color);
                        w!(x, "</{name}>");
                    }
                    (_, None) => {
                        w!(x, "><color auto=\"1\"/></{name}>");
                    }
                }
            }
            x.push_str("</border>");
        }
        w!(x, "</{element}>");
    }

    /// The `<dxfs>` element of `styles.xml`.
    pub fn xml(&mut self) -> String {
        let mut x = String::new();
        if self.list.is_empty() {
            x.push_str("<dxfs count=\"0\"/>");
            return x;
        }
        w!(x, "<dxfs count=\"{}\">", self.list.len());
        for (id, patch) in self.list.clone().iter().enumerate() {
            match patch.fill {
                Some(Some(color)) if self.cell_fills.contains(&(id as u32)) => {
                    x.push_str("<dxf><fill><patternFill patternType=\"solid\">");
                    push_color(&mut x, "fgColor", color);
                    push_color(&mut x, "bgColor", color);
                    x.push_str("</patternFill></fill></dxf>");
                }
                _ => self.push_dxf(&mut x, "dxf", patch),
            }
        }
        x.push_str("</dxfs>");
        x
    }
}

/// Append a colour element: a colour of the theme by its number and its
/// tint, any other colour by its value.
pub(crate) fn push_color(x: &mut String, element: &str, color: Color) {
    match color.theme() {
        Some(reference) => {
            w!(x, "<{element} theme=\"{}\"", reference.slot.index());
            if reference.tint != 0.0 {
                w!(x, " tint=\"{}\"", reference.tint);
            }
            x.push_str("/>");
        }
        None => {
            w!(x, "<{element} rgb=\"FF{:06X}\"/>", color.value() & 0xFF_FFFF);
        }
    }
}

// ---- reading -------------------------------------------------------------------

fn border_style(name: &str) -> Option<BorderStyle> {
    Some(match name {
        "none" => BorderStyle::None,
        "hair" => BorderStyle::Hair,
        "thin" => BorderStyle::Thin,
        "medium" => BorderStyle::Medium,
        "thick" => BorderStyle::Thick,
        "double" => BorderStyle::Double,
        "dotted" | "dashDotDot" | "mediumDashDotDot" => BorderStyle::Dotted,
        "dashed" | "mediumDashed" | "dashDot" | "mediumDashDot" | "slantDashDot" => {
            BorderStyle::Dashed
        }
        _ => return None,
    })
}

/// The children of an element, each handed to `child` (which consumes it).
fn each_child<'a>(
    r: &mut Reader<'a>,
    tag: &Tag<'a>,
    mut child: impl FnMut(&mut Reader<'a>, &Tag<'a>),
) {
    if tag.empty {
        return;
    }
    loop {
        match r.next() {
            Event::Open(t) => child(r, &t),
            Event::Close(_) | Event::Eof => break,
            _ => {}
        }
    }
}

fn read_font<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, palette: &Palette, patch: &mut StylePatch) {
    each_child(r, tag, |r, t| {
        let on = t.attr_bool("val").unwrap_or(true);
        match t.name {
            "b" => patch.bold = Some(on),
            "i" => patch.italic = Some(on),
            "strike" => patch.strike = Some(on),
            "u" => {
                patch.underline = Some(match t.attr("val").as_deref() {
                    None | Some("single" | "singleAccounting") => Underline::Single,
                    Some("double" | "doubleAccounting") => Underline::Double,
                    Some(_) => Underline::None,
                })
            }
            "sz" => {
                patch.font_size = t
                    .attr_f64("val")
                    .map(|pt| (pt * 2.0).round().clamp(2.0, 818.0) as u16)
            }
            "name" => {
                patch.font_name = t
                    .attr("val")
                    .filter(|name| !name.is_empty())
                    .map(|name| Arc::from(name.as_ref()))
            }
            "color" => patch.font_color = Some(RawColor::from_tag(t).resolve(palette)),
            _ => {}
        }
        r.skip(t);
    });
}

fn read_fill<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, palette: &Palette, patch: &mut StylePatch) {
    each_child(r, tag, |r, t| match t.name {
        "patternFill" => {
            let pattern = t.attr("patternType");
            let mut fg = None;
            let mut bg = None;
            each_child(r, t, |r, c| {
                match c.name {
                    "fgColor" => fg = RawColor::from_tag(c).resolve(palette),
                    "bgColor" => bg = RawColor::from_tag(c).resolve(palette),
                    _ => {}
                }
                r.skip(c);
            });
            patch.fill = match pattern.as_deref() {
                Some("none") => Some(None),
                // The colour of a solid differential fill is the
                // background one; files from other writers have it in the
                // foreground, as for cells.
                None | Some("solid") => bg.or(fg).map(Some),
                // Hatch patterns are approximated by the colour they
                // average to.
                Some(other) => {
                    let density = match other {
                        "darkGray" => 0.75,
                        "mediumGray" => 0.5,
                        "lightGray" => 0.25,
                        "gray125" => 0.125,
                        "gray0625" => 0.0625,
                        p if p.starts_with("dark") => 0.5,
                        _ => 0.25,
                    };
                    Some(Some(Color(blend(
                        fg.map_or(0x000000, |c| c.value()),
                        bg.map_or(0xFFFFFF, |c| c.value()),
                        density,
                    ))))
                }
            };
        }
        "gradientFill" => {
            // The first stop stands for the whole gradient.
            let mut first = None;
            each_child(r, t, |r, stop| {
                each_child(r, stop, |r, c| {
                    if c.name == "color" && first.is_none() {
                        first = RawColor::from_tag(c).resolve(palette);
                    }
                    r.skip(c);
                });
            });
            patch.fill = first.map(Some);
        }
        _ => r.skip(t),
    });
}

fn read_border<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, palette: &Palette, patch: &mut StylePatch) {
    each_child(r, tag, |r, t| {
        // A side without a style is a side left alone.
        let style = t.attr("style").and_then(|name| border_style(&name));
        let mut color = None;
        each_child(r, t, |r, c| {
            if c.name == "color" {
                color = RawColor::from_tag(c).resolve(palette);
            }
            r.skip(c);
        });
        let border = style.map(|style| Border {
            style,
            color: color.filter(|_| style != BorderStyle::None),
        });
        match t.name {
            "left" | "start" => patch.border_left = border,
            "right" | "end" => patch.border_right = border,
            "top" => patch.border_top = border,
            "bottom" => patch.border_bottom = border,
            _ => {}
        }
    });
}

/// Read a `<dxf>` (or `<x14:dxf>`) element. What the engine has no place
/// for (alignment, protection, diagonal borders) is left out.
pub(crate) fn read_dxf<'a>(
    r: &mut Reader<'a>,
    tag: &Tag<'a>,
    palette: &Palette,
    locale: &Locale,
) -> StylePatch {
    let mut patch = StylePatch::default();
    each_child(r, tag, |r, t| match t.name {
        "font" => read_font(r, t, palette, &mut patch),
        "fill" => read_fill(r, t, palette, &mut patch),
        "border" => read_border(r, t, palette, &mut patch),
        "numFmt" => {
            let code: Option<Cow<str>> = t
                .attr("formatCode")
                .filter(|code| !code.trim().is_empty())
                .or_else(|| {
                    t.attr_u32("numFmtId")
                        .and_then(|id| builtin_code(id, locale))
                });
            patch.num_fmt = code.map(|code| {
                if code.eq_ignore_ascii_case("General") {
                    Arc::from("General")
                } else {
                    Arc::from(code.as_ref())
                }
            });
            r.skip(t);
        }
        _ => r.skip(t),
    });
    patch
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(patch: &StylePatch) -> String {
        let mut dxfs = Dxfs::new(vec!["0.000".to_string()], Locale::en());
        let mut x = String::new();
        dxfs.push_dxf(&mut x, "dxf", patch);
        x
    }

    fn read(xml: &str) -> StylePatch {
        let mut r = Reader::new(xml);
        let Event::Open(tag) = r.next() else {
            panic!("no element in {xml}");
        };
        read_dxf(&mut r, &tag, &Palette::default(), Locale::en())
    }

    #[test]
    fn formats_as_excel_writes_them() {
        // "Light Red Fill with Dark Red Text", the first preset of the
        // ribbon.
        let preset = StylePatch {
            font_color: Some(Some(Color(0x9C0006))),
            fill: Some(Some(Color(0xFFC7CE))),
            ..StylePatch::default()
        };
        let xml = written(&preset);
        assert_eq!(
            xml,
            "<dxf><font><color rgb=\"FF9C0006\"/></font><fill><patternFill><bgColor rgb=\"FFFFC7CE\"/></patternFill></fill></dxf>"
        );
        assert_eq!(read(&xml), preset);
        assert_eq!(written(&StylePatch::default()), "<dxf></dxf>");

        // Everything a patch can say, each property on, off or absent.
        let full = StylePatch {
            font_name: Some(Arc::from("Arial & Co")),
            font_size: Some(25),
            bold: Some(true),
            italic: Some(false),
            underline: Some(Underline::Double),
            strike: Some(true),
            font_color: Some(None),
            fill: Some(None),
            border_left: Some(Border::thin()),
            border_right: Some(Border {
                style: BorderStyle::Dashed,
                color: Some(Color(0x123456)),
            }),
            border_top: Some(Border::NONE),
            border_bottom: None,
            num_fmt: Some(Arc::from("0.0\" kg\"")),
        };
        let xml = written(&full);
        assert_eq!(
            xml,
            concat!(
                "<dxf><font><b/><i val=\"0\"/><strike/><u val=\"double\"/><sz val=\"12.5\"/>",
                "<color auto=\"1\"/><name val=\"Arial &amp; Co\"/></font>",
                "<numFmt numFmtId=\"165\" formatCode=\"0.0&quot; kg&quot;\"/>",
                "<fill><patternFill patternType=\"none\"/></fill>",
                "<border><left style=\"thin\"><color auto=\"1\"/></left>",
                "<right style=\"dashed\"><color rgb=\"FF123456\"/></right>",
                "<top style=\"none\"/><bottom/></border></dxf>"
            )
        );
        assert_eq!(read(&xml), full);
        for underline in [Underline::None, Underline::Single, Underline::Double] {
            let patch = StylePatch {
                underline: Some(underline),
                ..StylePatch::default()
            };
            assert_eq!(read(&written(&patch)), patch);
        }
    }

    #[test]
    fn number_formats_share_the_ids_of_the_workbook() {
        let mut dxfs = Dxfs::new(vec!["0.000".to_string()], Locale::en());
        let with = |code: &str| StylePatch {
            num_fmt: Some(Arc::from(code)),
            ..StylePatch::default()
        };
        let mut x = String::new();
        for code in ["0.00", "0.000", "0.0000", "m/d/yyyy", "0.0000", "[Red]0"] {
            dxfs.push_dxf(&mut x, "dxf", &with(code));
        }
        let ids: Vec<&str> = x
            .split("numFmtId=\"")
            .skip(1)
            .map(|rest| rest.split('"').next().unwrap())
            .collect();
        assert_eq!(ids, ["2", "164", "165", "14", "165", "166"]);
        // Each format is stored once.
        assert_eq!(dxfs.id(&with("0.00")), 0);
        assert_eq!(dxfs.id(&with("0.0000")), 1);
        assert_eq!(dxfs.id(&with("0.00")), 0);
        let xml = dxfs.xml();
        assert!(
            xml.starts_with("<dxfs count=\"2\"><dxf><numFmt numFmtId=\"2\""),
            "{xml}"
        );
        assert_eq!(
            Dxfs::new(Vec::new(), Locale::en()).xml(),
            "<dxfs count=\"0\"/>"
        );
    }

    #[test]
    fn formats_from_other_writers() {
        // Theme and indexed colours, a fill given as a cell fill, a
        // built-in number format without its code, sides left alone.
        let patch = read(concat!(
            "<dxf><font><b val=\"1\"/><i val=\"false\"/><color theme=\"5\" tint=\"-0.249977111117893\"/><family val=\"2\"/></font>",
            "<numFmt numFmtId=\"10\"/>",
            "<fill><patternFill patternType=\"solid\"><fgColor indexed=\"10\"/></patternFill></fill>",
            "<alignment horizontal=\"center\"/>",
            "<border diagonalUp=\"1\"><left/><right style=\"mediumDashed\"><color indexed=\"64\"/></right>",
            "<top style=\"bogus\"/><bottom style=\"thick\"/><diagonal style=\"thin\"/><vertical/><horizontal/></border>",
            "<protection locked=\"0\"/><extLst><ext><font><b val=\"0\"/></font></ext></extLst></dxf>"
        ));
        assert_eq!(patch.bold, Some(true));
        assert_eq!(patch.italic, Some(false));
        let font_color = patch.font_color.flatten().unwrap();
        assert_eq!(font_color.plain(), Color(0xC65911), "accent 2, darker 25 %");
        assert_eq!(font_color.theme().map(|t| t.slot.index()), Some(5));
        assert_eq!(patch.num_fmt.as_deref(), Some("0.00%"));
        assert_eq!(patch.fill, Some(Some(Color(0xFF0000))));
        assert_eq!(patch.border_left, None);
        assert_eq!(
            patch.border_right,
            Some(Border {
                style: BorderStyle::Dashed,
                color: None
            })
        );
        assert_eq!(patch.border_top, None);
        assert_eq!(
            patch.border_bottom,
            Some(Border {
                style: BorderStyle::Thick,
                color: None
            })
        );
        // Fills: none, a hatch, a gradient, and a fill that says nothing.
        assert_eq!(
            read("<dxf><fill><patternFill patternType=\"none\"/></fill></dxf>").fill,
            Some(None)
        );
        assert_eq!(
            read("<dxf><fill><patternFill patternType=\"mediumGray\"><fgColor rgb=\"FF000000\"/><bgColor rgb=\"FFFFFFFF\"/></patternFill></fill></dxf>").fill,
            Some(Some(Color(0x808080)))
        );
        assert_eq!(
            read("<dxf><fill><gradientFill><stop position=\"0\"><color rgb=\"FF00FF00\"/></stop><stop position=\"1\"><color rgb=\"FF0000FF\"/></stop></gradientFill></fill></dxf>").fill,
            Some(Some(Color(0x00FF00)))
        );
        assert_eq!(read("<dxf><fill><patternFill/></fill></dxf>").fill, None);
        // Cut short, or empty.
        assert_eq!(read("<dxf/>"), StylePatch::default());
        assert_eq!(read("<dxf><font><b/>").bold, Some(true));
        assert_eq!(
            read("<dxf><border><left style=\"thin\"><color rgb=\"zz").border_left,
            Some(Border::thin())
        );
    }
}
