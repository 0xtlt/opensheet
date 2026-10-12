//! Reading `.xlsx` packages into a [`Workbook`].

use super::chart::Track;
use super::chart::read::{parse_chart, parse_drawing};
use super::color::{Palette, RawColor, blend, parse_rgb, parse_theme};
use super::condfmt::{self, CondFormats};
use super::defaults::{self, Defaults};
use super::dxf::read_dxf;
use super::formula::from_file_text;
use super::numfmt::{builtin_code, shows_date};
use super::xml::{Event, Reader, Tag, decode_cell_text, parse_bool};
use super::{col_width_to_px, page, points_to_px, protection, validation};
use crate::{IoError, Result, decode_utf16};
use opensheet_engine::ast::Expr;
use opensheet_engine::chart::Chart;
use opensheet_engine::datetime::date_to_serial;
use opensheet_engine::protect::WorkbookProtection;
use opensheet_engine::refs::{MAX_COLS, MAX_ROWS, Rect};
use opensheet_engine::style::{
    Border, BorderStyle, Borders, DEFAULT_FONT, DEFAULT_FONT_SIZE, Font, HAlign, Style, StyleId,
    StylePatch, StyleTable, Underline, VAlign,
};
use opensheet_engine::value::{ErrorKind, Value};
use opensheet_engine::workbook::{Cell, Content, DefinedName, Sheet};
use opensheet_engine::{Color, Locale, Workbook};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;

/// Largest part read into memory (uncompressed). Real sheets stay far below
/// this; a crafted archive could otherwise inflate without end.
const MAX_PART_SIZE: u64 = 2 << 30;

/// Days between the 1900 and 1904 date systems.
const DATE_1904_OFFSET: f64 = 1462.0;

// ---- package ---------------------------------------------------------------

pub(super) struct Package<R: Read + Seek> {
    zip: zip::ZipArchive<R>,
    /// Normalised part name → index in the archive.
    index: HashMap<String, usize>,
}

/// Part names are case-insensitive and sometimes written with a leading
/// slash or backslashes.
fn normalize_part(name: &str) -> String {
    name.trim_start_matches(['/', '\\'])
        .replace('\\', "/")
        .to_lowercase()
}

fn decode_xml(bytes: Vec<u8>) -> String {
    match bytes.as_slice() {
        [0xFF, 0xFE, rest @ ..] => decode_utf16(rest, true),
        [0xFE, 0xFF, rest @ ..] => decode_utf16(rest, false),
        _ => String::from_utf8(bytes)
            .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
    }
}

impl<R: Read + Seek> Package<R> {
    fn open(reader: R) -> Result<Package<R>> {
        let zip = zip::ZipArchive::new(reader)?;
        let mut index = HashMap::new();
        for (i, name) in zip.file_names().enumerate() {
            if let Ok(name) = name {
                index.entry(normalize_part(&name)).or_insert(i);
            }
        }
        Ok(Package { zip, index })
    }

    pub(super) fn read(&mut self, part: &str) -> Result<Option<String>> {
        Ok(self.read_bytes(part)?.map(decode_xml))
    }

    /// A part as it is stored: an image, or text before it is decoded.
    pub(super) fn read_bytes(&mut self, part: &str) -> Result<Option<Vec<u8>>> {
        let Some(&i) = self.index.get(&normalize_part(part)) else {
            return Ok(None);
        };
        let file = self.zip.by_index(i)?;
        let mut bytes = Vec::with_capacity((file.size() as usize).min(64 << 20));
        // Declared sizes cannot be trusted; stop inflating at the limit.
        file.take(MAX_PART_SIZE + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_PART_SIZE {
            return Err(IoError::Invalid(format!("the part {part} is too large")));
        }
        Ok(Some(bytes))
    }
}

pub(super) struct Rel {
    pub(super) id: String,
    /// Last segment of the relationship type ("worksheet", "styles", ...).
    pub(super) kind: String,
    /// Part name, resolved against the source part.
    pub(super) target: String,
}

fn dir_of(part: &str) -> &str {
    match part.rfind('/') {
        Some(i) => &part[..=i],
        None => "",
    }
}

fn rels_part(part: &str) -> String {
    match part.rfind('/') {
        Some(i) => format!("{}/_rels/{}.rels", &part[..i], &part[i + 1..]),
        None => format!("_rels/{part}.rels"),
    }
}

fn resolve_target(base_dir: &str, target: &str) -> String {
    let joined = match target.strip_prefix('/') {
        Some(absolute) => absolute.to_string(),
        None => format!("{base_dir}{target}"),
    };
    let mut parts: Vec<&str> = Vec::new();
    for segment in joined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn parse_rels(xml: &str, base_dir: &str) -> Vec<Rel> {
    let mut out = Vec::new();
    let mut r = Reader::new(xml);
    loop {
        match r.next() {
            Event::Open(t) if t.name == "Relationship" => {
                let external = t
                    .attr("TargetMode")
                    .is_some_and(|m| m.eq_ignore_ascii_case("External"));
                if let (Some(id), Some(kind), Some(target), false) =
                    (t.attr("Id"), t.attr("Type"), t.attr("Target"), external)
                {
                    out.push(Rel {
                        id: id.into_owned(),
                        kind: kind.rsplit('/').next().unwrap_or("").to_string(),
                        target: resolve_target(base_dir, &target),
                    });
                }
                r.skip(&t);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

/// The relationships of a part; none when it has no relationships part or
/// that part cannot be read.
pub(super) fn relationships<R: Read + Seek>(pkg: &mut Package<R>, part: &str) -> Vec<Rel> {
    match pkg.read(&rels_part(part)) {
        Ok(Some(xml)) => parse_rels(&xml, dir_of(part)),
        _ => Vec::new(),
    }
}

// ---- workbook part -----------------------------------------------------------

struct RawName {
    name: String,
    text: String,
    comment: String,
    /// For a sheet-scoped name, the position of its sheet in the file.
    local_sheet: Option<usize>,
}

#[derive(Default)]
struct WorkbookPart {
    /// (name, relationship id)
    sheets: Vec<(String, String)>,
    names: Vec<RawName>,
    date1904: bool,
    protection: WorkbookProtection,
}

fn parse_workbook_part(xml: &str) -> WorkbookPart {
    let mut out = WorkbookPart::default();
    let mut r = Reader::new(xml);
    loop {
        match r.next() {
            Event::Open(t) => match t.name {
                "workbook" | "sheets" | "definedNames" => {}
                "workbookPr" => {
                    out.date1904 = t.attr_bool("date1904").unwrap_or(false);
                    r.skip(&t);
                }
                "workbookProtection" => {
                    out.protection = protection::read_workbook(&t);
                    r.skip(&t);
                }
                "sheet" => {
                    let name = t.attr("name").map(Cow::into_owned).unwrap_or_default();
                    let rid = t
                        .attr("r:id")
                        .or_else(|| t.attr_local("id"))
                        .map(Cow::into_owned);
                    out.sheets.push((name, rid.unwrap_or_default()));
                    r.skip(&t);
                }
                "definedName" => {
                    let name = t.attr("name").map(Cow::into_owned).unwrap_or_default();
                    let local_sheet = t.attr_u32("localSheetId").map(|i| i as usize);
                    let comment = t.attr("comment").map(Cow::into_owned).unwrap_or_default();
                    let text = r.text(&t).into_owned();
                    out.names.push(RawName {
                        name,
                        text,
                        comment,
                        local_sheet,
                    });
                }
                _ => r.skip(&t),
            },
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

// ---- styles ------------------------------------------------------------------

#[derive(Default)]
struct RawFont {
    name: Option<String>,
    size: Option<f64>,
    bold: bool,
    italic: bool,
    strike: bool,
    underline: Underline,
    color: Option<RawColor>,
}

#[derive(Default)]
struct RawFill {
    /// `patternType` of a pattern fill (`None` for gradients).
    pattern: Option<String>,
    fg: Option<RawColor>,
    bg: Option<RawColor>,
    /// First stop of a gradient fill.
    gradient: Option<RawColor>,
}

#[derive(Default)]
struct RawBorder {
    /// left, right, top, bottom
    sides: [(BorderStyle, Option<RawColor>); 4],
}

struct RawXf {
    num_fmt: u32,
    font: usize,
    fill: usize,
    border: usize,
    h_align: HAlign,
    v_align: VAlign,
    wrap: bool,
    shrink: bool,
    indent: u8,
    rotation: i16,
    locked: bool,
    hidden: bool,
}

/// Parse the `item` children of a list element such as `<fonts>`.
fn parse_list<'a, T>(
    r: &mut Reader<'a>,
    container: &Tag<'a>,
    item: &str,
    mut parse: impl FnMut(&mut Reader<'a>, &Tag<'a>) -> T,
) -> Vec<T> {
    let mut out = Vec::new();
    if container.empty {
        return out;
    }
    loop {
        match r.next() {
            Event::Open(t) if t.name == item => out.push(parse(r, &t)),
            Event::Open(t) => r.skip(&t),
            Event::Close(_) | Event::Eof => break,
            _ => {}
        }
    }
    out
}

fn parse_font<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> RawFont {
    let mut f = RawFont::default();
    if tag.empty {
        return f;
    }
    loop {
        match r.next() {
            Event::Open(t) => {
                let on = t.attr_bool("val").unwrap_or(true);
                match t.name {
                    "b" => f.bold = on,
                    "i" => f.italic = on,
                    "strike" => f.strike = on,
                    "u" => {
                        f.underline = match t.attr("val").as_deref() {
                            None | Some("single") | Some("singleAccounting") => Underline::Single,
                            Some("double") | Some("doubleAccounting") => Underline::Double,
                            Some(_) => Underline::None,
                        }
                    }
                    "sz" => f.size = t.attr_f64("val"),
                    "name" => f.name = t.attr("val").map(Cow::into_owned),
                    "color" => f.color = Some(RawColor::from_tag(&t)),
                    _ => {}
                }
                r.skip(&t);
            }
            Event::Close(_) | Event::Eof => break,
            _ => {}
        }
    }
    f
}

fn parse_fill<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> RawFill {
    let mut f = RawFill::default();
    if tag.empty {
        return f;
    }
    loop {
        match r.next() {
            Event::Open(t) => match t.name {
                "patternFill" => {
                    f.pattern = Some(
                        t.attr("patternType")
                            .map_or_else(|| "none".to_string(), Cow::into_owned),
                    );
                }
                "gradientFill" | "stop" => {}
                "fgColor" => {
                    f.fg = Some(RawColor::from_tag(&t));
                    r.skip(&t);
                }
                "bgColor" => {
                    f.bg = Some(RawColor::from_tag(&t));
                    r.skip(&t);
                }
                "color" => {
                    f.gradient.get_or_insert_with(|| RawColor::from_tag(&t));
                    r.skip(&t);
                }
                _ => r.skip(&t),
            },
            Event::Close("fill") | Event::Eof => break,
            _ => {}
        }
    }
    f
}

fn border_style(name: Option<&str>) -> BorderStyle {
    match name {
        Some("hair") => BorderStyle::Hair,
        Some("thin") => BorderStyle::Thin,
        Some("medium") => BorderStyle::Medium,
        Some("thick") => BorderStyle::Thick,
        Some("double") => BorderStyle::Double,
        Some("dotted" | "dashDotDot" | "mediumDashDotDot") => BorderStyle::Dotted,
        Some("dashed" | "mediumDashed" | "dashDot" | "mediumDashDot" | "slantDashDot") => {
            BorderStyle::Dashed
        }
        _ => BorderStyle::None,
    }
}

fn parse_border<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> RawBorder {
    let mut b = RawBorder::default();
    if tag.empty {
        return b;
    }
    loop {
        match r.next() {
            Event::Open(t) => {
                let side = match t.name {
                    "left" | "start" => Some(0),
                    "right" | "end" => Some(1),
                    "top" => Some(2),
                    "bottom" => Some(3),
                    _ => None,
                };
                let Some(side) = side else {
                    r.skip(&t);
                    continue;
                };
                let style = border_style(t.attr("style").as_deref());
                let mut color = None;
                if !t.empty {
                    loop {
                        match r.next() {
                            Event::Open(c) => {
                                if c.name == "color" {
                                    color = Some(RawColor::from_tag(&c));
                                }
                                r.skip(&c);
                            }
                            Event::Close(_) | Event::Eof => break,
                            _ => {}
                        }
                    }
                }
                b.sides[side] = (style, color);
            }
            Event::Close(_) | Event::Eof => break,
            _ => {}
        }
    }
    b
}

fn parse_xf<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> RawXf {
    let mut xf = RawXf {
        num_fmt: tag.attr_u32("numFmtId").unwrap_or(0),
        font: tag.attr_u32("fontId").unwrap_or(0) as usize,
        fill: tag.attr_u32("fillId").unwrap_or(0) as usize,
        border: tag.attr_u32("borderId").unwrap_or(0) as usize,
        h_align: HAlign::General,
        v_align: VAlign::Bottom,
        wrap: false,
        shrink: false,
        indent: 0,
        rotation: 0,
        locked: true,
        hidden: false,
    };
    if tag.empty {
        return xf;
    }
    loop {
        match r.next() {
            Event::Open(t) => {
                match t.name {
                    "alignment" => {
                        xf.h_align = match t.attr("horizontal").as_deref() {
                            Some("left") => HAlign::Left,
                            Some("center") => HAlign::Center,
                            Some("right") => HAlign::Right,
                            Some("fill") => HAlign::Fill,
                            Some("justify" | "distributed") => HAlign::Justify,
                            Some("centerContinuous") => HAlign::CenterAcross,
                            _ => HAlign::General,
                        };
                        xf.v_align = match t.attr("vertical").as_deref() {
                            Some("top" | "justify") => VAlign::Top,
                            Some("center" | "distributed") => VAlign::Center,
                            _ => VAlign::Bottom,
                        };
                        xf.wrap = t.attr_bool("wrapText").unwrap_or(false);
                        xf.shrink = t.attr_bool("shrinkToFit").unwrap_or(false);
                        xf.indent = t.attr_u32("indent").unwrap_or(0).min(250) as u8;
                        // 0..=90 is counter-clockwise, 91..=180 clockwise by
                        // (n - 90); 255 (stacked letters) has no equivalent.
                        xf.rotation = match t.attr_u32("textRotation").unwrap_or(0) {
                            n @ 0..=90 => n as i16,
                            n @ 91..=180 => 90 - n as i16,
                            _ => 0,
                        };
                    }
                    "protection" => {
                        xf.locked = t.attr_bool("locked").unwrap_or(true);
                        xf.hidden = t.attr_bool("hidden").unwrap_or(false);
                    }
                    _ => {}
                }
                r.skip(&t);
            }
            Event::Close(_) | Event::Eof => break,
            _ => {}
        }
    }
    xf
}

fn resolve_fill(f: &RawFill, palette: &Palette) -> Option<Color> {
    let Some(pattern) = &f.pattern else {
        return f.gradient.and_then(|c| c.resolve(palette));
    };
    let fg = f.fg.and_then(|c| c.resolve(palette));
    let bg = f.bg.and_then(|c| c.resolve(palette));
    match pattern.as_str() {
        "none" => None,
        "solid" => fg,
        other => {
            // Hatch patterns are approximated by the colour they average to.
            let density = match other {
                "darkGray" => 0.75,
                "mediumGray" => 0.5,
                "lightGray" => 0.25,
                "gray125" => 0.125,
                "gray0625" => 0.0625,
                p if p.starts_with("dark") => 0.5,
                _ => 0.25,
            };
            Some(Color(blend(
                fg.map_or(0x000000, |c| c.0),
                bg.map_or(0xFFFFFF, |c| c.0),
                density,
            )))
        }
    }
}

// Built field by field so the engine can add fields to `Font` freely.
#[allow(clippy::field_reassign_with_default)]
fn resolve_font(f: &RawFont, palette: &Palette) -> Font {
    let color = f.color.and_then(|c| {
        let rgb = c.resolve(palette)?;
        // Plain "text 1" black is what the engine calls automatic.
        if c.is_default_text() && rgb == Color::BLACK {
            None
        } else {
            Some(rgb)
        }
    });
    let mut font = Font::default();
    font.name = Arc::from(
        f.name
            .as_deref()
            .filter(|n| !n.is_empty())
            .unwrap_or(DEFAULT_FONT),
    );
    font.size = match f.size {
        Some(pt) => (pt * 2.0).round().clamp(2.0, 818.0) as u16,
        None => DEFAULT_FONT_SIZE,
    };
    font.bold = f.bold;
    font.italic = f.italic;
    font.underline = f.underline;
    font.strike = f.strike;
    font.color = color;
    font
}

struct StylesPart {
    /// Engine style of every `cellXfs` entry.
    xfs: Vec<StyleId>,
    palette: Palette,
    /// The sizes that follow from the workbook's default font.
    defaults: Defaults,
    /// The differential formats, by `dxfId`.
    dxfs: Vec<StylePatch>,
}

// `Style` values are built field by field so the engine can add fields.
#[allow(clippy::field_reassign_with_default)]
fn parse_styles(
    xml: Option<&str>,
    theme: [u32; 12],
    locale: &Locale,
    table: &mut StyleTable,
) -> StylesPart {
    let mut palette = Palette {
        theme,
        ..Palette::default()
    };
    let mut num_fmts: HashMap<u32, String> = HashMap::new();
    let mut fonts: Vec<RawFont> = Vec::new();
    let mut fills: Vec<RawFill> = Vec::new();
    let mut borders: Vec<RawBorder> = Vec::new();
    let mut xfs: Vec<RawXf> = Vec::new();
    let mut style_xfs: Vec<RawXf> = Vec::new();
    // `xfId` of the cell style called Normal.
    let mut normal_xf = 0usize;
    // The differential formats use colours, which may be redefined after
    // them in the part: they are read once everything else is.
    let mut dxfs_at: Option<(&str, Tag)> = None;

    let mut r = Reader::new(xml.unwrap_or(""));
    loop {
        match r.next() {
            Event::Open(t) => match t.name {
                "styleSheet" => {}
                "numFmts" => {
                    let list = parse_list(&mut r, &t, "numFmt", |r, f| {
                        let entry = (
                            f.attr_u32("numFmtId"),
                            f.attr("formatCode").map(Cow::into_owned),
                        );
                        r.skip(f);
                        entry
                    });
                    for entry in list {
                        if let (Some(id), Some(code)) = entry {
                            num_fmts.insert(id, code);
                        }
                    }
                }
                "fonts" => fonts = parse_list(&mut r, &t, "font", parse_font),
                "fills" => fills = parse_list(&mut r, &t, "fill", parse_fill),
                "borders" => borders = parse_list(&mut r, &t, "border", parse_border),
                "cellStyleXfs" => style_xfs = parse_list(&mut r, &t, "xf", parse_xf),
                "cellXfs" => xfs = parse_list(&mut r, &t, "xf", parse_xf),
                "cellStyles" => {
                    let normal = parse_list(&mut r, &t, "cellStyle", |r, style| {
                        let is_normal = style.attr_u32("builtinId") == Some(0);
                        let xf = style.attr_u32("xfId").unwrap_or(0) as usize;
                        r.skip(style);
                        is_normal.then_some(xf)
                    });
                    normal_xf = normal.into_iter().flatten().next().unwrap_or(0);
                }
                "dxfs" => {
                    dxfs_at = Some((r.rest(), t));
                    r.skip(&t);
                }
                "colors" => {}
                "indexedColors" => {
                    let custom = parse_list(&mut r, &t, "rgbColor", |r, c| {
                        let rgb = c.attr("rgb").and_then(|v| parse_rgb(&v));
                        r.skip(c);
                        rgb
                    });
                    for (i, rgb) in custom.into_iter().enumerate() {
                        if let (Some(rgb), Some(slot)) = (rgb, palette.indexed.get_mut(i)) {
                            *slot = rgb;
                        }
                    }
                }
                _ => r.skip(&t),
            },
            Event::Eof => break,
            _ => {}
        }
    }

    let fonts: Vec<Font> = fonts.iter().map(|f| resolve_font(f, &palette)).collect();
    let fills: Vec<Option<Color>> = fills.iter().map(|f| resolve_fill(f, &palette)).collect();
    let borders: Vec<Borders> = borders
        .iter()
        .map(|b| {
            let side = |i: usize| {
                let (style, color) = b.sides[i];
                Border {
                    style,
                    color: if style == BorderStyle::None {
                        None
                    } else {
                        color.and_then(|c| c.resolve(&palette))
                    },
                }
            };
            Borders {
                left: side(0),
                right: side(1),
                top: side(2),
                bottom: side(3),
            }
        })
        .collect();

    // The first cell format is the format of every cell that was given
    // none; its font is the default font of the workbook. (Whatever else
    // it sets, which is next to never anything, stays with the cells that
    // name it.)
    if let Some(font) = xfs.first().and_then(|xf| fonts.get(xf.font)) {
        let mut default = Style::default();
        default.font = font.clone();
        table.set_default(default);
    }
    // Column widths count in digits of the font of the Normal style, which
    // is the same font in any file a spreadsheet application made.
    let normal_font = style_xfs
        .get(normal_xf)
        .or(style_xfs.first())
        .and_then(|xf| fonts.get(xf.font))
        .unwrap_or(&table.default_style().font);
    let defaults = Defaults::of(normal_font);

    let mut out: Vec<StyleId> = xfs
        .iter()
        .map(|xf| {
            let code: Cow<str> = match num_fmts.get(&xf.num_fmt) {
                Some(code) => Cow::Borrowed(code.as_str()),
                None => builtin_code(xf.num_fmt, locale).unwrap_or(Cow::Borrowed("General")),
            };
            let code = if code.trim().is_empty() || code.eq_ignore_ascii_case("General") {
                "General"
            } else {
                code.as_ref()
            };
            let mut style = Style::default();
            style.font = fonts.get(xf.font).cloned().unwrap_or_default();
            style.fill = fills.get(xf.fill).copied().flatten();
            style.h_align = xf.h_align;
            style.v_align = xf.v_align;
            style.wrap = xf.wrap;
            style.shrink_to_fit = xf.shrink;
            style.indent = xf.indent;
            style.rotation = xf.rotation;
            style.num_fmt = Arc::from(code);
            style.borders = borders.get(xf.border).copied().unwrap_or_default();
            style.locked = xf.locked;
            style.hidden = xf.hidden;
            table.intern(style)
        })
        .collect();
    if out.is_empty() {
        out.push(StyleId(0));
    }
    let dxfs = match dxfs_at {
        Some((rest, tag)) => parse_list(&mut Reader::new(rest), &tag, "dxf", |r, dxf| {
            read_dxf(r, dxf, &palette, locale)
        }),
        None => Vec::new(),
    };
    StylesPart {
        xfs: out,
        palette,
        defaults,
        dxfs,
    }
}

// ---- shared strings ------------------------------------------------------------

/// Text of an `<si>` / `<is>` element: the concatenation of its `<t>` runs,
/// without phonetic hints.
fn parse_rich_text<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> String {
    let mut s = String::new();
    if tag.empty {
        return s;
    }
    loop {
        match r.next() {
            Event::Open(t) => match t.name {
                "t" => s.push_str(&decode_cell_text(&r.text(&t))),
                "r" => {}
                _ => r.skip(&t),
            },
            Event::Close("r") => {}
            Event::Close(_) | Event::Eof => break,
            _ => {}
        }
    }
    s
}

fn parse_shared_strings(xml: &str) -> Vec<Arc<str>> {
    let mut out = Vec::new();
    let mut r = Reader::new(xml);
    loop {
        match r.next() {
            Event::Open(t) => match t.name {
                "sst" => {}
                "si" => out.push(Arc::from(parse_rich_text(&mut r, &t))),
                _ => r.skip(&t),
            },
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

// ---- worksheets ----------------------------------------------------------------

struct Ctx<'c> {
    xfs: &'c [StyleId],
    strings: &'c [Arc<str>],
    palette: &'c Palette,
    /// The differential formats of the workbook, by `dxfId`.
    dxfs: &'c [StylePatch],
    /// The colours filters and sorts go by, by `dxfId`.
    filter_colors: &'c [super::filter::DxfColors],
    locale: &'c Locale,
    /// The sizes that follow from the workbook's default font.
    defaults: Defaults,
    /// In a 1904-based workbook: the styles whose values are dates.
    shift_dates: Option<&'c HashSet<StyleId>>,
}

impl Ctx<'_> {
    fn style(&self, xf: u32) -> StyleId {
        self.xfs.get(xf as usize).copied().unwrap_or_default()
    }
}

/// Parse an A1-style cell reference into 0-based (row, col).
fn parse_cell_ref(s: &str) -> Option<(u32, u32)> {
    let mut col = 0u32;
    let mut row = 0u32;
    let mut letters = 0;
    let mut digits = 0;
    for b in s.bytes() {
        match b {
            b'$' => {}
            b'A'..=b'Z' | b'a'..=b'z' if digits == 0 => {
                letters += 1;
                if letters > 3 {
                    return None;
                }
                col = col * 26 + (b.to_ascii_uppercase() - b'A' + 1) as u32;
            }
            b'0'..=b'9' if letters > 0 => {
                digits += 1;
                if digits > 7 {
                    return None;
                }
                row = row * 10 + (b - b'0') as u32;
            }
            _ => return None,
        }
    }
    if row == 0 || row > MAX_ROWS || col == 0 || col > MAX_COLS {
        return None;
    }
    Some((row - 1, col - 1))
}

fn parse_error(text: &str) -> ErrorKind {
    let text = text.trim();
    ErrorKind::from_str(text).unwrap_or(if text.to_ascii_uppercase().starts_with("#N/A") {
        ErrorKind::NA
    } else {
        // Errors the engine has no name for (#GETTING_DATA, #FIELD!, ...).
        ErrorKind::Value
    })
}

/// ISO 8601 date / time of a `t="d"` cell → serial number.
fn parse_iso_date(text: &str) -> Option<f64> {
    let text = text.trim().trim_end_matches('Z');
    let (date, time) = match text.split_once('T') {
        Some((d, t)) => (Some(d), Some(t)),
        None if text.contains(':') => (None, Some(text)),
        None => (Some(text), None),
    };
    let mut serial = 0.0;
    if let Some(date) = date {
        let mut parts = date.splitn(3, '-').map(|p| p.parse::<i64>().ok());
        let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
        serial += date_to_serial(y, m, d)?;
    }
    if let Some(time) = time {
        let mut parts = time.splitn(3, ':').map(|p| p.parse::<f64>().ok());
        let h = parts.next()??;
        let m = parts.next().flatten().unwrap_or(0.0);
        let s = parts.next().flatten().unwrap_or(0.0);
        serial += (h * 3600.0 + m * 60.0 + s) / 86400.0;
    }
    Some(serial)
}

struct SheetReader<'c> {
    ctx: &'c Ctx<'c>,
    sheet: Sheet,
    /// Shared formula masters by `si`: (row, col, formula); `None` when the
    /// master could not be parsed.
    shared: HashMap<u32, Option<(u32, u32, Expr)>>,
    /// Multi-cell array formulas: covered range and anchor cell.
    arrays: Vec<(Rect, (u32, u32))>,
    /// The default row height exactly as the file states it, in pixels
    /// (the sheet keeps it in whole pixels).
    stated_row_height: f32,
    /// The conditional formats, gathered from two places of the part.
    cond_formats: CondFormats,
    /// Relationship id of the drawing part.
    drawing: Option<String>,
}

/// A worksheet part: the sheet, and what is needed to place its charts.
struct LoadedSheet {
    sheet: Sheet,
    /// Relationship id of the drawing part (charts, pictures, shapes).
    drawing: Option<String>,
}

impl<'c> SheetReader<'c> {
    fn read(xml: &str, name: String, ctx: &'c Ctx<'c>) -> LoadedSheet {
        // Until its `sheetFormatPr` says otherwise, the sheet has the
        // sizes of the workbook's default font.
        let mut sheet = Sheet::new(name);
        sheet.default_col_width = ctx.defaults.col_width;
        sheet.default_row_height = ctx.defaults.row_height;
        let mut this = SheetReader {
            ctx,
            stated_row_height: sheet.default_row_height,
            sheet,
            shared: HashMap::new(),
            arrays: Vec::new(),
            cond_formats: CondFormats::default(),
            drawing: None,
        };
        let mut r = Reader::new(xml);
        loop {
            match r.next() {
                // The children of `worksheet`, in the order of the schema.
                Event::Open(t) => match t.name {
                    "worksheet" => {}
                    "sheetPr" => this.sheet_pr(&mut r, &t),
                    "sheetViews" => this.sheet_views(&mut r, &t),
                    "sheetFormatPr" => {
                        if let Some(px) = defaults::read(&t, &ctx.defaults, &mut this.sheet) {
                            this.stated_row_height = px;
                        }
                        r.skip(&t);
                    }
                    "cols" => this.cols(&mut r, &t),
                    "sheetData" => this.sheet_data(&mut r, &t),
                    "sheetProtection" => {
                        this.sheet.protection = protection::read_sheet(&t);
                        r.skip(&t);
                    }
                    "autoFilter" => {
                        this.sheet.auto_filter =
                            super::filter::parse_auto_filter(&mut r, &t, ctx.filter_colors)
                    }
                    "mergeCells" => this.merges(&mut r, &t),
                    "conditionalFormatting" => {
                        let ctx = this.cond_format_context();
                        this.cond_formats.read(&mut r, &t, &ctx);
                    }
                    "dataValidations" => validation::read(&mut r, &t, &mut this.sheet),
                    // printOptions, pageMargins, pageSetup, headerFooter,
                    // rowBreaks, colBreaks
                    _ if page::read(&mut r, &t, &mut this.sheet) => {}
                    "drawing" => {
                        this.drawing = t.attr_local("id").map(Cow::into_owned);
                        r.skip(&t);
                    }
                    "extLst" => this.extensions(&mut r, &t),
                    _ => r.skip(&t),
                },
                Event::Eof => break,
                _ => {}
            }
        }
        this.finish()
    }

    /// The extension list of the worksheet: what later versions of the
    /// format added to features that already had an element of their own.
    /// Each `<ext>` holds one element.
    fn extensions<'a>(&mut self, r: &mut Reader<'a>, tag: &Tag<'a>) {
        if tag.empty {
            return;
        }
        loop {
            match r.next() {
                Event::Open(ext) if ext.name == "ext" && !ext.empty => loop {
                    match r.next() {
                        Event::Open(t) => match t.name {
                            "conditionalFormattings" => {
                                let ctx = self.cond_format_context();
                                self.cond_formats.read_ext(r, &t, &ctx);
                            }
                            "dataValidations" => validation::read(r, &t, &mut self.sheet),
                            _ => r.skip(&t),
                        },
                        Event::Close(_) | Event::Eof => break,
                        _ => {}
                    }
                },
                Event::Open(t) => r.skip(&t),
                Event::Close(_) | Event::Eof => break,
                _ => {}
            }
        }
    }

    fn cond_format_context(&self) -> condfmt::Context<'c> {
        condfmt::Context {
            palette: self.ctx.palette,
            dxfs: self.ctx.dxfs,
            locale: self.ctx.locale,
        }
    }

    fn sheet_pr<'a>(&mut self, r: &mut Reader<'a>, tag: &Tag<'a>) {
        if tag.empty {
            return;
        }
        loop {
            match r.next() {
                Event::Open(t) => {
                    match t.name {
                        // An automatic tab colour is no colour.
                        "tabColor" => {
                            self.sheet.tab_color = RawColor::from_tag(&t).resolve(self.ctx.palette)
                        }
                        "pageSetUpPr" => page::read_sheet_pr(&t, &mut self.sheet.page),
                        _ => {}
                    }
                    r.skip(&t);
                }
                Event::Close(_) | Event::Eof => break,
                _ => {}
            }
        }
    }

    fn sheet_views<'a>(&mut self, r: &mut Reader<'a>, tag: &Tag<'a>) {
        if tag.empty {
            return;
        }
        let mut first = true;
        loop {
            match r.next() {
                Event::Open(view) if view.name == "sheetView" && first => {
                    first = false;
                    self.sheet.show_gridlines = view.attr_bool("showGridLines").unwrap_or(true);
                    self.sheet.show_headers = view.attr_bool("showRowColHeaders").unwrap_or(true);
                    if view.empty {
                        continue;
                    }
                    loop {
                        match r.next() {
                            Event::Open(t) => {
                                let frozen = matches!(
                                    t.attr("state").as_deref(),
                                    Some("frozen" | "frozenSplit")
                                );
                                if t.name == "pane" && frozen {
                                    let split = |name: &str, max: u32| {
                                        let n = t.attr_f64(name).unwrap_or(0.0).round();
                                        if n >= 1.0 && n < max as f64 {
                                            n as u32
                                        } else {
                                            0
                                        }
                                    };
                                    self.sheet.frozen_cols = split("xSplit", MAX_COLS);
                                    self.sheet.frozen_rows = split("ySplit", MAX_ROWS);
                                }
                                r.skip(&t);
                            }
                            Event::Close(_) | Event::Eof => break,
                            _ => {}
                        }
                    }
                }
                Event::Open(t) => r.skip(&t),
                Event::Close(_) | Event::Eof => break,
                _ => {}
            }
        }
    }

    fn cols<'a>(&mut self, r: &mut Reader<'a>, tag: &Tag<'a>) {
        if tag.empty {
            return;
        }
        loop {
            match r.next() {
                Event::Open(t) => {
                    if t.name == "col" {
                        self.col(&t);
                    }
                    r.skip(&t);
                }
                Event::Close(_) | Event::Eof => break,
                _ => {}
            }
        }
    }

    fn col(&mut self, t: &Tag) {
        let (Some(min), Some(max)) = (t.attr_u32("min"), t.attr_u32("max")) else {
            return;
        };
        let (lo, hi) = (min.max(1) - 1, max.clamp(1, MAX_COLS) - 1);
        if lo > hi {
            return;
        }
        let hidden = t.attr_bool("hidden").unwrap_or(false);
        let custom = t.attr_bool("customWidth").unwrap_or(false);
        let default = self.sheet.default_col_width;
        let width = t
            .attr_f64("width")
            .map(|width| col_width_to_px(width, self.ctx.defaults.digit_width))
            // A hidden column stored with no width gets the default one
            // back when it is unhidden.
            .filter(|px| !(hidden && *px < 0.5))
            .filter(|px| custom || (px - default).abs() > 0.5);
        let style = t
            .attr_u32("style")
            .map(|xf| self.ctx.style(xf))
            .filter(|s| *s != StyleId(0));
        for c in lo..=hi {
            if let Some(px) = width {
                self.sheet.col_widths.insert(c, px.min(1800.0));
            }
            if hidden {
                self.sheet.hidden_cols.insert(c);
            }
            if let Some(style) = style {
                self.sheet.col_styles.insert(c, style);
            }
        }
    }

    fn sheet_data<'a>(&mut self, r: &mut Reader<'a>, tag: &Tag<'a>) {
        if tag.empty {
            return;
        }
        let mut next_row = 0u32;
        loop {
            match r.next() {
                Event::Open(t) if t.name == "row" => {
                    let row = match t.attr_u32("r") {
                        Some(n) if n >= 1 => n - 1,
                        _ => next_row,
                    };
                    next_row = row.saturating_add(1);
                    if row >= MAX_ROWS {
                        r.skip(&t);
                        continue;
                    }
                    self.row(&t, row);
                    if t.empty {
                        continue;
                    }
                    let mut next_col = 0u32;
                    loop {
                        match r.next() {
                            Event::Open(c) if c.name == "c" => self.cell(r, &c, row, &mut next_col),
                            Event::Open(c) => r.skip(&c),
                            Event::Close(_) | Event::Eof => break,
                            _ => {}
                        }
                    }
                }
                Event::Open(t) => r.skip(&t),
                Event::Close(_) | Event::Eof => break,
                _ => {}
            }
        }
    }

    fn row(&mut self, t: &Tag, row: u32) {
        let hidden = t.attr_bool("hidden").unwrap_or(false);
        if hidden {
            self.sheet.hidden_rows.insert(row);
        }
        if let Some(px) = t.attr_f64("ht").map(points_to_px) {
            let custom = t.attr_bool("customHeight").unwrap_or(false);
            // Heights without `customHeight` are auto-fit results; they are
            // kept when they differ from the default so the sheet looks the
            // same as where it was saved.
            let is_default = (px - self.stated_row_height).abs() <= 0.5
                || (px - self.sheet.default_row_height).abs() <= 0.5;
            if !(hidden && px < 0.5) && (custom || !is_default) {
                self.sheet.row_heights.insert(row, px.min(546.0));
            }
        }
        if t.attr_bool("customFormat").unwrap_or(false)
            && let Some(style) = t
                .attr_u32("s")
                .map(|xf| self.ctx.style(xf))
                .filter(|s| *s != StyleId(0))
        {
            self.sheet.row_styles.insert(row, style);
        }
    }

    fn cell<'a>(&mut self, r: &mut Reader<'a>, tag: &Tag<'a>, row: u32, next_col: &mut u32) {
        let mut pos = None;
        let mut xf = 0u32;
        let mut kind = "";
        for (k, v) in tag.attrs() {
            match k {
                "r" => pos = parse_cell_ref(v),
                "s" => xf = v.trim().parse().unwrap_or(0),
                "t" => kind = v,
                _ => {}
            }
        }
        let (row, col) = pos.unwrap_or((row, *next_col));
        *next_col = col.saturating_add(1);

        let mut value_text: Option<Cow<'a, str>> = None;
        let mut inline: Option<String> = None;
        let mut formula: Option<(Tag<'a>, Cow<'a, str>)> = None;
        if !tag.empty {
            loop {
                match r.next() {
                    Event::Open(t) => match t.name {
                        "v" => value_text = Some(r.text(&t)),
                        "f" => {
                            let text = r.text(&t);
                            formula = Some((t, text));
                        }
                        "is" => inline = Some(parse_rich_text(r, &t)),
                        _ => r.skip(&t),
                    },
                    Event::Close(_) | Event::Eof => break,
                    _ => {}
                }
            }
        }
        if row >= MAX_ROWS || col >= MAX_COLS {
            return;
        }

        let mut cached = match kind {
            "s" => value_text
                .and_then(|v| v.trim().parse::<usize>().ok())
                .and_then(|i| self.ctx.strings.get(i))
                .map_or(Value::Empty, |s| Value::Text(s.clone())),
            "str" => value_text.map_or(Value::Empty, |v| Value::text(decode_cell_text(&v))),
            "inlineStr" => match (inline, value_text) {
                (Some(s), _) => Value::from(s),
                (None, Some(v)) => Value::text(decode_cell_text(&v)),
                (None, None) => Value::Empty,
            },
            "b" => value_text.map_or(Value::Empty, |v| Value::Bool(parse_bool(&v))),
            "e" => value_text.map_or(Value::Empty, |v| Value::Error(parse_error(&v))),
            "d" => value_text
                .and_then(|v| parse_iso_date(&v))
                .map_or(Value::Empty, Value::Number),
            _ => match value_text {
                Some(v) if !v.trim().is_empty() => match v.trim().parse::<f64>() {
                    Ok(n) if n.is_finite() => Value::Number(n),
                    Ok(_) => Value::Error(ErrorKind::Num),
                    Err(_) => Value::text(v),
                },
                _ => Value::Empty,
            },
        };

        let mut expr: Option<Expr> = None;
        if let Some((f, text)) = formula {
            match f.attr("t").as_deref() {
                Some("shared") => {
                    let si = f.attr_u32("si");
                    if !text.trim().is_empty() {
                        expr = from_file_text(&text);
                        if let Some(si) = si {
                            self.shared
                                .entry(si)
                                .or_insert_with(|| expr.clone().map(|e| (row, col, e)));
                        }
                    } else if let Some(Some((r0, c0, master))) =
                        si.and_then(|si| self.shared.get(&si))
                    {
                        expr =
                            Some(master.shifted(row as i64 - *r0 as i64, col as i64 - *c0 as i64));
                    }
                }
                Some("array") => {
                    expr = from_file_text(&text);
                    let range = f.attr("ref").and_then(|s| Rect::parse(&s));
                    if let (Some(_), Some(range)) = (&expr, range)
                        && !range.is_single()
                        && range.contains(row, col)
                    {
                        self.arrays.push((range, (row, col)));
                    }
                }
                // What-if data tables have no formula the engine could run;
                // their cached results are kept as constants.
                Some("dataTable") => {}
                _ => expr = from_file_text(&text),
            }
        }

        let style = self.ctx.style(xf);
        let content = match expr {
            Some(e) => Content::Formula(Arc::new(e)),
            None => {
                if let (Value::Number(n), Some(dates)) = (&mut cached, self.ctx.shift_dates)
                    && dates.contains(&style)
                {
                    *n += DATE_1904_OFFSET;
                }
                if cached.is_empty() {
                    Content::Empty
                } else {
                    Content::Value(cached.clone())
                }
            }
        };
        // A cell with nothing in it and no formatting of its own is not
        // stored (same rule as `Workbook::put_cell`).
        if matches!(content, Content::Empty) && style == self.sheet.inherited_style(row, col) {
            return;
        }
        self.sheet.cells.insert(
            (row, col),
            Cell {
                content,
                value: cached,
                style,
            },
        );
    }

    fn merges<'a>(&mut self, r: &mut Reader<'a>, tag: &Tag<'a>) {
        if tag.empty {
            return;
        }
        loop {
            match r.next() {
                Event::Open(t) => {
                    // Single cells and overlapping areas are not merges.
                    if t.name == "mergeCell"
                        && let Some(rect) = t.attr("ref").and_then(|s| Rect::parse(&s))
                        && !rect.is_single()
                        && !self.sheet.merges.iter().any(|m| m.intersects(&rect))
                    {
                        self.sheet.merges.push(rect);
                    }
                    r.skip(&t);
                }
                Event::Close(_) | Event::Eof => break,
                _ => {}
            }
        }
    }

    fn finish(mut self) -> LoadedSheet {
        std::mem::take(&mut self.cond_formats).finish(&mut self.sheet);
        // The cells an array formula spills into are stored in the file with
        // their cached values. As constants they would block the spill, so
        // they are dropped: the engine recomputes them.
        for (range, anchor) in std::mem::take(&mut self.arrays) {
            let covered: Vec<(u32, u32)> = self
                .sheet
                .cells
                .range((range.r0, range.c0)..=(range.r1, range.c1))
                .filter(|((_, c), cell)| {
                    *c >= range.c0 && *c <= range.c1 && matches!(cell.content, Content::Value(_))
                })
                .map(|(k, _)| *k)
                .filter(|k| *k != anchor)
                .collect();
            for key in covered {
                let inherited = self.sheet.inherited_style(key.0, key.1);
                let formatted = self
                    .sheet
                    .cells
                    .get(&key)
                    .is_some_and(|c| c.style != inherited);
                if !formatted {
                    self.sheet.cells.remove(&key);
                } else if let Some(cell) = self.sheet.cells.get_mut(&key) {
                    cell.content = Content::Empty;
                    cell.value = Value::Empty;
                }
            }
        }
        LoadedSheet {
            sheet: self.sheet,
            drawing: self.drawing,
        }
    }
}

// ---- charts ------------------------------------------------------------------------

/// The charts of a sheet: its drawing part says where they sit, a part per
/// chart says what they plot. A part that is missing or makes no sense
/// costs the charts it describes, never the workbook.
fn read_charts<R: Read + Seek>(
    pkg: &mut Package<R>,
    sheet_part: &str,
    loaded: &LoadedSheet,
    palette: &Palette,
    locale: &Locale,
    next_id: &mut u32,
) -> Vec<Chart> {
    let Some(drawing_id) = &loaded.drawing else {
        return Vec::new();
    };
    let Some(drawing_part) = relationships(pkg, sheet_part)
        .into_iter()
        .find(|rel| rel.id == *drawing_id && rel.kind == "drawing")
        .map(|rel| rel.target)
    else {
        return Vec::new();
    };
    let Ok(Some(drawing_xml)) = pkg.read(&drawing_part) else {
        return Vec::new();
    };
    let frames = parse_drawing(&drawing_xml);
    if frames.is_empty() {
        return Vec::new();
    }
    let rels = relationships(pkg, &drawing_part);

    let sheet = &loaded.sheet;
    let cols = Track {
        default: sheet.default_col_width,
        sizes: &sheet.col_widths,
        hidden: &sheet.hidden_cols,
        max: f32::INFINITY,
        count: MAX_COLS,
    };
    let rows = Track {
        default: sheet.default_row_height,
        sizes: &sheet.row_heights,
        hidden: &sheet.hidden_rows,
        max: f32::INFINITY,
        count: MAX_ROWS,
    };
    let mut charts = Vec::new();
    for frame in &frames {
        for id in &frame.charts {
            // Charts of the newer kinds (waterfall, sunburst, ...) live in
            // parts of another type; the engine has nothing like them.
            let Some(rel) = rels.iter().find(|rel| rel.id == *id && rel.kind == "chart") else {
                continue;
            };
            let Ok(Some(xml)) = pkg.read(&rel.target) else {
                continue;
            };
            let Some(mut chart) = parse_chart(&xml, palette, locale) else {
                continue;
            };
            let place = frame.placement(&cols, &rows);
            chart.id = *next_id;
            chart.anchor = place.anchor;
            chart.offset = place.offset;
            chart.size = place.size;
            charts.push(chart);
            *next_id += 1;
        }
    }
    charts
}

// ---- entry point -----------------------------------------------------------------

/// Sheet names must be unique and non-empty for references to resolve.
fn unique_sheet_name(name: &str, taken: &[Sheet], index: usize) -> String {
    let base = if name.trim().is_empty() {
        format!("Sheet{}", index + 1)
    } else {
        name.to_string()
    };
    let exists = |n: &str| {
        taken
            .iter()
            .any(|s| s.name.to_lowercase() == n.to_lowercase())
    };
    if !exists(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base} ({n})"))
        .find(|n| !exists(n))
        .unwrap_or(base)
}

pub(crate) fn read_workbook<R: Read + Seek>(
    mut reader: R,
    locale: &'static Locale,
) -> Result<Workbook> {
    // Password-protected workbooks and legacy .xls files are OLE compound
    // documents, not zip archives.
    let mut magic = [0u8; 8];
    let mut got = 0;
    while got < magic.len() {
        match reader.read(&mut magic[got..])? {
            0 => break,
            n => got += n,
        }
    }
    reader.seek(SeekFrom::Start(0))?;
    if magic[..got] == [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1] {
        return Err(IoError::Unsupported(
            "password-protected workbooks and legacy .xls files cannot be opened".into(),
        ));
    }

    let mut pkg = Package::open(reader)?;
    let workbook_part = match pkg.read("_rels/.rels")? {
        Some(xml) => parse_rels(&xml, "")
            .into_iter()
            .find(|r| r.kind == "officeDocument")
            .map(|r| r.target),
        None => None,
    }
    .unwrap_or_else(|| "xl/workbook.xml".to_string());
    let workbook_xml = pkg.read(&workbook_part)?.ok_or_else(|| {
        IoError::Invalid("not a spreadsheet: the workbook part is missing".into())
    })?;
    let info = parse_workbook_part(&workbook_xml);
    let base = dir_of(&workbook_part).to_string();
    let rels = match pkg.read(&rels_part(&workbook_part))? {
        Some(xml) => parse_rels(&xml, &base),
        None => Vec::new(),
    };
    let part_of = |kind: &str, fallback: &str| {
        rels.iter()
            .find(|r| r.kind == kind)
            .map_or_else(|| format!("{base}{fallback}"), |r| r.target.clone())
    };

    let theme = match pkg.read(&part_of("theme", "theme/theme1.xml"))? {
        Some(xml) => parse_theme(&xml),
        None => super::color::OFFICE_THEME,
    };
    let mut wb = Workbook::new(locale);
    wb.protection = info.protection;
    let styles_xml = pkg.read(&part_of("styles", "styles.xml"))?;
    let styles = parse_styles(styles_xml.as_deref(), theme, locale, &mut wb.styles);
    let style_lookup =
        super::stylerefs::StyleLookup::read(styles_xml.as_deref(), &styles.palette, locale);
    drop(styles_xml);
    let strings = match pkg.read(&part_of("sharedStrings", "sharedStrings.xml"))? {
        Some(xml) => parse_shared_strings(&xml),
        None => Vec::new(),
    };

    let date_styles: HashSet<StyleId> = if info.date1904 {
        styles
            .xfs
            .iter()
            .copied()
            .filter(|id| shows_date(&wb.styles.get(*id).num_fmt))
            .collect()
    } else {
        HashSet::new()
    };
    let ctx = Ctx {
        xfs: &styles.xfs,
        strings: &strings,
        palette: &styles.palette,
        dxfs: &styles.dxfs,
        filter_colors: &style_lookup.dxf_colors,
        locale,
        defaults: styles.defaults,
        shift_dates: info.date1904.then_some(&date_styles),
    };

    let mut sheets: Vec<Sheet> = Vec::new();
    // The worksheet part of each sheet that was loaded.
    let mut sheet_parts: Vec<String> = Vec::new();
    // Where each sheet of the file ended up (`None` when it was not loaded).
    let mut loaded_as: Vec<Option<usize>> = vec![None; info.sheets.len()];
    // Chart ids are unique in the workbook; files have no such thing.
    let mut next_chart_id = 1;
    let comments = super::comments::Context::read(
        &mut pkg,
        &rels,
        &styles.palette,
        &wb.styles.get(ctx.style(0)).font,
    );
    let mut drawings = super::drawing::Loader::new(&mut pkg, &rels);
    for (i, (name, rid)) in info.sheets.iter().enumerate() {
        let part = match rels.iter().find(|r| r.id == *rid) {
            Some(rel) if rel.kind == "worksheet" => rel.target.clone(),
            // Chart sheets, dialog sheets and macro sheets have no cells.
            Some(_) => continue,
            None => format!("{base}worksheets/sheet{}.xml", i + 1),
        };
        let Some(xml) = pkg.read(&part)? else {
            continue;
        };
        let name = unique_sheet_name(name, &sheets, i);
        let mut loaded = SheetReader::read(&xml, name, &ctx);
        drop(xml);
        loaded.sheet.charts = read_charts(
            &mut pkg,
            &part,
            &loaded,
            &styles.palette,
            locale,
            &mut next_chart_id,
        );
        // Note boxes are placed with the sizes the file gives its columns
        // and rows.
        let default_sizes = (
            loaded.sheet.default_col_width,
            loaded.sheet.default_row_height,
        );
        super::comments::read_sheet(&mut pkg, &part, &mut loaded.sheet, default_sizes, &comments);
        drawings.read_sheet(
            &mut pkg,
            &part,
            loaded.drawing.as_deref(),
            &mut loaded.sheet,
            &styles.palette,
        );
        loaded_as[i] = Some(sheets.len());
        sheets.push(loaded.sheet);
        sheet_parts.push(part);
    }
    if !sheets.is_empty() {
        wb.sheets = sheets;
        // What needs the whole workbook: tables have names and ids no
        // other table of any sheet may have, PivotTables share caches and
        // write their reports into the cells.
        super::table::read_all(&mut pkg, &mut wb, &sheet_parts, &style_lookup.dxf_colors);
        super::pivot::read_all(&mut pkg, &mut wb, &sheet_parts, &style_lookup);
    } else {
        loaded_as.clear();
    }

    // Workbook-scoped names first: a name scoped to a sheet that was not
    // loaded (a chart sheet) is kept for the workbook, where it may only
    // take a spelling that is still free.
    let mut names = info.names;
    names.sort_by_key(|n| n.local_sheet.is_some());
    for raw in names {
        let key = raw.name.to_uppercase();
        let scope = raw
            .local_sheet
            .and_then(|i| loaded_as.get(i).copied().flatten());
        // The print area and the print titles of a sheet are part of its
        // page setup. The other `_xlnm.` names (filter ranges and the
        // like) belong to features that keep their ranges elsewhere.
        let sheet = scope.and_then(|sheet| wb.sheets.get_mut(sheet));
        if page::read_name(&key, &raw.text, sheet)
            || raw.name.is_empty()
            || key.starts_with("_XLNM.")
        {
            continue;
        }
        let Some(expr) = from_file_text(&raw.text) else {
            continue;
        };
        let names = match scope {
            Some(sheet) => &mut wb.sheets[sheet].names,
            None => &mut wb.names,
        };
        names.entry(key).or_insert(DefinedName {
            name: raw.name,
            expr: Arc::new(expr),
            comment: raw.comment,
        });
    }

    // Formulas the engine cannot calculate keep their cached result.
    super::formula::keep_cached_results(&mut wb);

    wb.recalc();
    wb.clear_history();
    wb.modified = false;
    Ok(wb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_refs() {
        assert_eq!(parse_cell_ref("A1"), Some((0, 0)));
        assert_eq!(parse_cell_ref("$AB$12"), Some((11, 27)));
        assert_eq!(parse_cell_ref("XFD1048576"), Some((1_048_575, 16_383)));
        assert_eq!(parse_cell_ref("XFE1"), None);
        assert_eq!(parse_cell_ref("A0"), None);
        assert_eq!(parse_cell_ref("A1048577"), None);
        assert_eq!(parse_cell_ref("1A"), None);
        assert_eq!(parse_cell_ref("A1B"), None);
        assert_eq!(parse_cell_ref(""), None);
    }

    #[test]
    fn paths() {
        assert_eq!(
            resolve_target("xl/", "worksheets/sheet1.xml"),
            "xl/worksheets/sheet1.xml"
        );
        assert_eq!(resolve_target("xl/", "/xl/styles.xml"), "xl/styles.xml");
        assert_eq!(
            resolve_target("xl/worksheets/", "../theme/theme1.xml"),
            "xl/theme/theme1.xml"
        );
        assert_eq!(rels_part("xl/workbook.xml"), "xl/_rels/workbook.xml.rels");
        assert_eq!(rels_part("workbook.xml"), "_rels/workbook.xml.rels");
        assert_eq!(dir_of("xl/workbook.xml"), "xl/");
        assert_eq!(normalize_part("/XL\\Workbook.xml"), "xl/workbook.xml");
    }

    #[test]
    fn iso_dates() {
        assert_eq!(parse_iso_date("2024-02-29"), Some(45351.0));
        assert_eq!(parse_iso_date("2024-02-29T12:00:00Z"), Some(45351.5));
        assert_eq!(parse_iso_date("06:00:00"), Some(0.25));
        assert_eq!(parse_iso_date("nonsense"), None);
    }

    #[test]
    fn rich_text() {
        let xml = r#"<sst><si><t>plain</t></si><si><r><rPr><b/></rPr><t xml:space="preserve">bold </t></r><r><t>tail</t></r><rPh sb="0" eb="1"><t>phonetic</t></rPh></si><si/><si><t>a_x000D_b &amp; c</t></si></sst>"#;
        let strings = parse_shared_strings(xml);
        let strings: Vec<&str> = strings.iter().map(|s| s.as_ref()).collect();
        assert_eq!(strings, ["plain", "bold tail", "", "a\rb & c"]);
    }
}
