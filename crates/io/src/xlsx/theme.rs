//! The theme part of a package (`xl/theme/theme1.xml`): the theme of the
//! workbook, read and written.
//!
//! The engine keeps the name of the theme, its twelve colours, its two
//! fonts and the name of its effects (`opensheet_engine::theme`). What a
//! theme part holds beyond that (the fills, lines and shadows of the
//! effects, the fonts of other scripts, object defaults) the engine has
//! no model of: the part of a file that was read is kept as it was
//! (`Theme::source`) and written back with the colours, fonts and names
//! of the theme as they are now. A workbook made here gets the format
//! scheme of the Office theme.
//!
//! Without the part Excel falls back on the Office theme of 2013 and a
//! table style or a theme colour comes out in other colours.

use super::color::parse_rgb;
use super::xml::{Event, Reader, XML_DECL, push_attr};
use opensheet_engine::style::ThemeSlot;
use opensheet_engine::theme::{Theme, ThemeColors, ThemeFonts, builtin};
use std::fmt::Write as _;
use std::sync::Arc;

pub(crate) const CT_THEME: &str = "application/vnd.openxmlformats-officedocument.theme+xml";

/// The slots of a colour scheme, in the order of a theme part.
const SLOTS: [&str; 12] = [
    "dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6", "hlink",
    "folHlink",
];

/// Fills, lines and effects of the theme (the content of
/// `<a:fmtScheme>`): what shapes with a theme style are drawn with. These
/// are the ones of the Office theme. Nothing the writer makes refers to
/// them, but a theme is not valid without the three of each.
const FORMATS: &str = concat!(
    "<a:fillStyleLst>",
    "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>",
    "<a:gradFill rotWithShape=\"1\"><a:gsLst>",
    "<a:gs pos=\"0\"><a:schemeClr val=\"phClr\"><a:lumMod val=\"110000\"/><a:satMod val=\"105000\"/><a:tint val=\"67000\"/></a:schemeClr></a:gs>",
    "<a:gs pos=\"50000\"><a:schemeClr val=\"phClr\"><a:lumMod val=\"105000\"/><a:satMod val=\"103000\"/><a:tint val=\"73000\"/></a:schemeClr></a:gs>",
    "<a:gs pos=\"100000\"><a:schemeClr val=\"phClr\"><a:lumMod val=\"105000\"/><a:satMod val=\"109000\"/><a:tint val=\"81000\"/></a:schemeClr></a:gs>",
    "</a:gsLst><a:lin ang=\"5400000\" scaled=\"0\"/></a:gradFill>",
    "<a:gradFill rotWithShape=\"1\"><a:gsLst>",
    "<a:gs pos=\"0\"><a:schemeClr val=\"phClr\"><a:satMod val=\"103000\"/><a:lumMod val=\"102000\"/><a:tint val=\"94000\"/></a:schemeClr></a:gs>",
    "<a:gs pos=\"50000\"><a:schemeClr val=\"phClr\"><a:satMod val=\"110000\"/><a:lumMod val=\"100000\"/><a:shade val=\"100000\"/></a:schemeClr></a:gs>",
    "<a:gs pos=\"100000\"><a:schemeClr val=\"phClr\"><a:lumMod val=\"99000\"/><a:satMod val=\"120000\"/><a:shade val=\"78000\"/></a:schemeClr></a:gs>",
    "</a:gsLst><a:lin ang=\"5400000\" scaled=\"0\"/></a:gradFill>",
    "</a:fillStyleLst>",
    "<a:lnStyleLst>",
    "<a:ln w=\"12700\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/><a:miter lim=\"800000\"/></a:ln>",
    "<a:ln w=\"19050\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/><a:miter lim=\"800000\"/></a:ln>",
    "<a:ln w=\"25400\" cap=\"flat\" cmpd=\"sng\" algn=\"ctr\"><a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill><a:prstDash val=\"solid\"/><a:miter lim=\"800000\"/></a:ln>",
    "</a:lnStyleLst>",
    "<a:effectStyleLst>",
    "<a:effectStyle><a:effectLst/></a:effectStyle>",
    "<a:effectStyle><a:effectLst/></a:effectStyle>",
    "<a:effectStyle><a:effectLst><a:outerShdw blurRad=\"57150\" dist=\"19050\" dir=\"5400000\" algn=\"ctr\" rotWithShape=\"0\"><a:srgbClr val=\"000000\"><a:alpha val=\"63000\"/></a:srgbClr></a:outerShdw></a:effectLst></a:effectStyle>",
    "</a:effectStyleLst>",
    "<a:bgFillStyleLst>",
    "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>",
    "<a:solidFill><a:schemeClr val=\"phClr\"><a:tint val=\"95000\"/><a:satMod val=\"170000\"/></a:schemeClr></a:solidFill>",
    "<a:gradFill rotWithShape=\"1\"><a:gsLst>",
    "<a:gs pos=\"0\"><a:schemeClr val=\"phClr\"><a:tint val=\"93000\"/><a:satMod val=\"150000\"/><a:shade val=\"98000\"/><a:lumMod val=\"102000\"/></a:schemeClr></a:gs>",
    "<a:gs pos=\"50000\"><a:schemeClr val=\"phClr\"><a:tint val=\"98000\"/><a:satMod val=\"130000\"/><a:shade val=\"90000\"/><a:lumMod val=\"103000\"/></a:schemeClr></a:gs>",
    "<a:gs pos=\"100000\"><a:schemeClr val=\"phClr\"><a:shade val=\"63000\"/><a:satMod val=\"120000\"/></a:schemeClr></a:gs>",
    "</a:gsLst><a:lin ang=\"5400000\" scaled=\"0\"/></a:gradFill>",
    "</a:bgFillStyleLst>",
);

/// The colour scheme of a theme, as an element.
fn color_scheme(colors: &ThemeColors) -> String {
    let mut x = String::from("<a:clrScheme name=\"");
    push_attr(&mut x, &colors.name);
    x.push_str("\">");
    for (name, color) in SLOTS.iter().zip(colors.part_order()) {
        // Black text on a white window are the colours of the system,
        // with the values they have where the system says nothing.
        match (*name, color) {
            ("dk1", 0x000000) => x.push_str("<a:dk1><a:sysClr val=\"windowText\" lastClr=\"000000\"/></a:dk1>"),
            ("lt1", 0xFFFFFF) => x.push_str("<a:lt1><a:sysClr val=\"window\" lastClr=\"FFFFFF\"/></a:lt1>"),
            _ => {
                w!(x, "<a:{name}><a:srgbClr val=\"{color:06X}\"/></a:{name}>");
            }
        }
    }
    x.push_str("</a:clrScheme>");
    x
}

/// The font scheme of a theme, as an element: headings, then body text.
fn font_scheme(fonts: &ThemeFonts) -> String {
    let mut x = String::from("<a:fontScheme name=\"");
    push_attr(&mut x, &fonts.name);
    x.push_str("\">");
    for (role, font) in [("majorFont", &fonts.major), ("minorFont", &fonts.minor)] {
        w!(x, "<a:{role}><a:latin typeface=\"");
        push_attr(&mut x, font);
        x.push('"');
        if let Some(panose) = builtin::panose(font) {
            w!(x, " panose=\"{panose}\"");
        }
        w!(x, "/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:{role}>");
    }
    x.push_str("</a:fontScheme>");
    x
}

/// The theme part of a workbook.
pub(crate) fn xml(theme: &Theme) -> String {
    if let Some(part) = theme.source.as_deref().and_then(|source| respliced(source, theme)) {
        return part;
    }
    let mut x = String::from(XML_DECL);
    x.push_str("<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"");
    push_attr(&mut x, &theme.name);
    x.push_str("\"><a:themeElements>");
    x.push_str(&color_scheme(&theme.colors));
    x.push_str(&font_scheme(&theme.fonts));
    x.push_str("<a:fmtScheme name=\"");
    push_attr(&mut x, &theme.effects);
    x.push_str("\">");
    x.push_str(FORMATS);
    x.push_str("</a:fmtScheme></a:themeElements><a:objectDefaults/><a:extraClrSchemeLst/></a:theme>");
    x
}

/// The span of the element `<a:name ...>...</a:name>` in `xml`, and the
/// span of the value of its `name` attribute.
fn element(xml: &str, name: &str) -> Option<(std::ops::Range<usize>, std::ops::Range<usize>)> {
    let open = format!("<a:{name}");
    let mut start = xml.find(&open)?;
    // "<a:theme" also begins "<a:themeElements".
    while !xml[start + open.len()..].starts_with([' ', '>', '\t', '\r', '\n']) {
        start += open.len() + xml[start + open.len()..].find(&open)?;
    }
    let tag_end = start + xml[start..].find('>')?;
    let close = format!("</a:{name}>");
    let end = tag_end + xml[tag_end..].find(&close)? + close.len();
    let attr = xml[start..tag_end].find(" name=\"")? + start + " name=\"".len();
    let attr_end = attr + xml[attr..tag_end].find('"')?;
    Some((start..end, attr..attr_end))
}

/// The part a theme was read from, with the colours, fonts and names the
/// theme has now. `None` when the part is not laid out as expected or
/// refers to other parts of its package (the pictures some themes fill
/// shapes with), which are not kept: the theme is then written afresh.
fn respliced(source: &str, theme: &Theme) -> Option<String> {
    let was = parse(source);
    let mut part = source.to_string();
    // From the last element to the first, so that the spans found in the
    // part stay good.
    let (format, format_name) = element(&part, "fmtScheme")?;
    if part[format.clone()].contains(" r:") {
        let mut stock = String::from("<a:fmtScheme name=\"");
        push_attr(&mut stock, &theme.effects);
        stock.push_str("\">");
        stock.push_str(FORMATS);
        stock.push_str("</a:fmtScheme>");
        part.replace_range(format, &stock);
    } else if was.effects != theme.effects {
        let mut name = String::new();
        push_attr(&mut name, &theme.effects);
        part.replace_range(format_name, &name);
    }
    if was.fonts != theme.fonts {
        let (fonts, _) = element(&part, "fontScheme")?;
        part.replace_range(fonts, &font_scheme(&theme.fonts));
    }
    if was.colors != theme.colors {
        let (colors, _) = element(&part, "clrScheme")?;
        part.replace_range(colors, &color_scheme(&theme.colors));
    }
    if was.name != theme.name {
        let (_, name) = element(&part, "theme")?;
        let mut text = String::new();
        push_attr(&mut text, &theme.name);
        part.replace_range(name, &text);
    }
    let whole = parse(&part);
    let same = whole.name == theme.name
        && whole.colors == theme.colors
        && whole.fonts == theme.fonts
        && whole.effects == theme.effects;
    (same && !part.contains(" r:embed=") && !part.contains(" r:link=") && !part.contains(" r:id=")).then_some(part)
}

/// Read a theme part. What it does not say is as in the Office theme of
/// 2013 to 2022, the theme of the files that have no theme part.
pub(crate) fn parse(xml: &str) -> Theme {
    let mut theme = Theme::office_2013();
    let mut colors = theme.colors.part_order();
    let mut r = Reader::new(xml);
    // The element of the colour scheme or the font of the font scheme
    // being read.
    let mut slot: Option<usize> = None;
    let mut role: Option<bool> = None;
    let (mut in_colors, mut in_fonts) = (false, false);
    loop {
        match r.next() {
            Event::Open(t) => match t.name {
                "theme" => {
                    if let Some(name) = t.attr("name") {
                        theme.name = name.into_owned();
                    }
                }
                "clrScheme" => {
                    in_colors = !t.empty;
                    if let Some(name) = t.attr("name") {
                        theme.colors.name = name.into_owned();
                    }
                }
                "fontScheme" => {
                    in_fonts = !t.empty;
                    if let Some(name) = t.attr("name") {
                        theme.fonts.name = name.into_owned();
                    }
                }
                "fmtScheme" => {
                    if let Some(name) = t.attr("name") {
                        theme.effects = name.into_owned();
                    }
                    // Its fills and lines have colours of their own.
                    r.skip(&t);
                }
                "objectDefaults" | "extraClrSchemeLst" | "custClrLst" | "extLst" => r.skip(&t),
                "srgbClr" if in_colors => {
                    if let (Some(i), Some(rgb)) = (slot, t.attr("val").and_then(|v| parse_rgb(&v))) {
                        colors[i] = rgb;
                    }
                }
                "sysClr" if in_colors => {
                    if let Some(i) = slot {
                        let rgb = t.attr("lastClr").and_then(|v| parse_rgb(&v)).or_else(|| {
                            match t.attr("val").as_deref() {
                                Some("windowText") => Some(0x000000),
                                Some("window") => Some(0xFFFFFF),
                                _ => None,
                            }
                        });
                        if let Some(rgb) = rgb {
                            colors[i] = rgb;
                        }
                    }
                }
                name if in_colors => {
                    if let Some(i) = SLOTS.iter().position(|s| *s == name) {
                        slot = Some(i);
                    }
                }
                "majorFont" if in_fonts => role = Some(true),
                "minorFont" if in_fonts => role = Some(false),
                "latin" if in_fonts => {
                    let typeface = t.attr("typeface").filter(|name| !name.trim().is_empty());
                    if let (Some(major), Some(name)) = (role, typeface) {
                        let font = if major { &mut theme.fonts.major } else { &mut theme.fonts.minor };
                        *font = Arc::from(name.as_ref());
                    }
                }
                _ => {}
            },
            Event::Close("clrScheme") => in_colors = false,
            Event::Close("fontScheme") => in_fonts = false,
            Event::Close("majorFont" | "minorFont") => role = None,
            Event::Eof => break,
            _ => {}
        }
    }
    theme.colors = ThemeColors::from_part_order(std::mem::take(&mut theme.colors.name), colors);
    debug_assert_eq!(theme.colors.get(ThemeSlot::Dark1), colors[0]);
    theme.source = Some(Arc::from(xml));
    theme
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_part_says_the_theme_of_the_workbook() {
        let office = Theme::office();
        let xml = xml(&office);
        let read = parse(&xml);
        assert_eq!(read.colors.part_order()[0], 0x000000);
        assert_eq!(read.colors.part_order()[1], 0xFFFFFF);
        assert_eq!(read.colors.part_order()[4], 0x156082, "accent 1");
        assert_eq!((read.name.as_str(), read.colors.name.as_str(), read.effects.as_str()), ("Office Theme", "Office", "Office"));
        assert_eq!(read.fonts, office.fonts);
        assert_eq!(read.colors, office.colors);
        assert!(xml.contains("<a:accent6><a:srgbClr val=\"4EA72E\"/></a:accent6>"));
        assert!(xml.contains("<a:dk1><a:sysClr val=\"windowText\" lastClr=\"000000\"/></a:dk1>"));
        assert!(xml.contains("<a:latin typeface=\"Aptos Narrow\" panose=\"02110004020202020204\"/>"));
        // Three fills, three lines, three effects and three background
        // fills: a theme with fewer is refused.
        let children = |list: &str| {
            let start = xml.find(&format!("<a:{list}>")).unwrap();
            let end = xml.find(&format!("</a:{list}>")).unwrap();
            let body = &xml[start..end];
            ["<a:solidFill>", "<a:gradFill ", "<a:ln ", "<a:effectStyle>"]
                .map(|item| body.matches(item).count())
        };
        assert_eq!(children("fillStyleLst"), [1, 2, 0, 0]);
        // Each line has a fill of its own.
        assert_eq!(children("lnStyleLst"), [3, 0, 3, 0]);
        assert_eq!(children("effectStyleLst"), [0, 0, 0, 3]);
        assert_eq!(children("bgFillStyleLst"), [2, 1, 0, 0]);
    }

    #[test]
    fn every_built_in_theme_comes_back() {
        for stock in builtin::THEMES {
            let theme = Theme::from(stock);
            let mut read = parse(&xml(&theme));
            read.source = None;
            assert_eq!(read, theme, "{}", stock.name);
        }
        // A theme whose first colours are not the system's.
        let mut odd = Theme::office();
        odd.colors.set(ThemeSlot::Dark1, 0x112233);
        odd.colors.set(ThemeSlot::Light1, 0xFFFFF0);
        odd.name = "A & B <\"mine\">".to_string();
        let part = xml(&odd);
        assert!(part.contains("<a:dk1><a:srgbClr val=\"112233\"/></a:dk1>"));
        let mut read = parse(&part);
        read.source = None;
        assert_eq!(read, odd);
    }

    /// A theme part the way another application writes one: other fonts
    /// for other scripts, effects of its own, an extension list.
    fn foreign() -> String {
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n",
            "<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"Corporate\">",
            "<a:themeElements><a:clrScheme name=\"Corporate colours\">",
            "<a:dk1><a:srgbClr val=\"101820\"/></a:dk1><a:lt1><a:srgbClr val=\"FFFFFF\"/></a:lt1>",
            "<a:dk2><a:srgbClr val=\"003057\"/></a:dk2><a:lt2><a:srgbClr val=\"F2F2F2\"/></a:lt2>",
            "<a:accent1><a:srgbClr val=\"E4002B\"/></a:accent1><a:accent2><a:srgbClr val=\"00A3E0\"/></a:accent2>",
            "<a:accent3><a:srgbClr val=\"84BD00\"/></a:accent3><a:accent4><a:srgbClr val=\"FFB81C\"/></a:accent4>",
            "<a:accent5><a:srgbClr val=\"753BBD\"/></a:accent5><a:accent6><a:srgbClr val=\"00B2A9\"/></a:accent6>",
            "<a:hlink><a:srgbClr val=\"0000EE\"/></a:hlink><a:folHlink><a:srgbClr val=\"551A8B\"/></a:folHlink>",
            "</a:clrScheme><a:fontScheme name=\"Corporate fonts\">",
            "<a:majorFont><a:latin typeface=\"Georgia\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/>",
            "<a:font script=\"Jpan\" typeface=\"Yu Gothic\"/></a:majorFont>",
            "<a:minorFont><a:latin typeface=\"Verdana\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/>",
            "<a:font script=\"Jpan\" typeface=\"Yu Gothic\"/></a:minorFont>",
            "</a:fontScheme><a:fmtScheme name=\"Corporate effects\"><a:fillStyleLst>",
            "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>",
            "<a:solidFill><a:srgbClr val=\"ABCDEF\"/></a:solidFill>",
            "<a:solidFill><a:schemeClr val=\"phClr\"/></a:solidFill>",
            "</a:fillStyleLst><a:lnStyleLst/><a:effectStyleLst/><a:bgFillStyleLst/></a:fmtScheme>",
            "</a:themeElements><a:objectDefaults/><a:extraClrSchemeLst/>",
            "<a:extLst><a:ext uri=\"{05A4C25C-085E-4340-85A3-A5531E510DB2}\"/></a:extLst></a:theme>",
        )
        .to_string()
    }

    #[test]
    fn a_theme_from_a_file_is_written_back_as_it_was() {
        let part = foreign();
        let theme = parse(&part);
        assert_eq!(theme.name, "Corporate");
        assert_eq!((theme.colors.name.as_str(), theme.rgb(ThemeSlot::Accent1)), ("Corporate colours", 0xE4002B));
        // The colour of a fill of the effects is not a colour of the theme.
        assert_eq!(theme.rgb(ThemeSlot::Dark1), 0x101820);
        assert_eq!(theme.rgb(ThemeSlot::FollowedHyperlink), 0x551A8B);
        assert_eq!((&*theme.fonts.major, &*theme.fonts.minor), ("Georgia", "Verdana"));
        assert_eq!(theme.effects, "Corporate effects");
        // Untouched: the same part, to the letter.
        assert_eq!(xml(&theme), part);

        // Other colours: the rest of the part stays.
        let mut recoloured = theme.clone();
        recoloured.colors = ThemeColors::from(&builtin::COLOR_SETS[3]);
        let written = xml(&recoloured);
        assert!(written.contains("<a:clrScheme name=\"Blue Warm\">"));
        assert!(written.contains("<a:accent1><a:srgbClr val=\"4A66AC\"/></a:accent1>"));
        assert!(written.contains("<a:font script=\"Jpan\" typeface=\"Yu Gothic\"/>"));
        assert!(written.contains("<a:srgbClr val=\"ABCDEF\"/>") && written.contains("<a:extLst>"));
        let mut read = parse(&written);
        read.source = recoloured.source.clone();
        assert_eq!(read, recoloured);

        // Other fonts, another name, other effects.
        let mut changed = theme.clone();
        changed.fonts = ThemeFonts::new("Arial", "Arial", "Arial");
        changed.name = "Mine".to_string();
        changed.effects = "Glossy".to_string();
        let written = xml(&changed);
        assert!(written.contains("<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"Mine\">"));
        assert!(written.contains("<a:fmtScheme name=\"Glossy\"><a:fillStyleLst>"));
        assert!(written.contains("<a:latin typeface=\"Arial\" panose=\"020B0604020202020204\"/>"));
        assert!(!written.contains("Yu Gothic") && written.contains("E4002B"));
        let mut read = parse(&written);
        read.source = changed.source.clone();
        assert_eq!(read, changed);
    }

    #[test]
    fn pictures_of_a_theme_are_not_referred_to() {
        // The fills of some themes are pictures kept in other parts,
        // which the package written here does not have.
        let part = foreign().replace(
            "<a:solidFill><a:srgbClr val=\"ABCDEF\"/></a:solidFill>",
            "<a:blipFill><a:blip xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" r:embed=\"rId1\"/></a:blipFill>",
        );
        let theme = parse(&part);
        let written = xml(&theme);
        assert!(!written.contains("r:embed"));
        assert!(written.contains("<a:fmtScheme name=\"Corporate effects\"><a:fillStyleLst>"));
        assert!(written.contains("Yu Gothic") && written.contains("<a:extLst>"));
        let mut read = parse(&written);
        read.source = theme.source.clone();
        assert_eq!(read, theme);
        // A part that cannot be taken apart is written afresh.
        let odd = parse("<theme name=\"Bare\"><clrScheme name=\"c\"><accent1><srgbClr val=\"010203\"/></accent1></clrScheme></theme>");
        assert_eq!((odd.name.as_str(), odd.rgb(ThemeSlot::Accent1)), ("Bare", 0x010203));
        assert!(xml(&odd).starts_with(XML_DECL));
    }
}
