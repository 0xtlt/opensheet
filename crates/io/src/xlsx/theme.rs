//! The theme part of the packages the writer makes (`xl/theme/theme1.xml`).
//!
//! The engine has one theme, the Office theme of 2023: its table styles,
//! its colour pickers and its chart palette are computed from it. A file
//! names a table style ("TableStyleMedium2") and leaves the colours to the
//! theme of the workbook, so the package carries that theme. Without the
//! part Excel falls back on the Office theme of 2013 and the same style
//! comes out in other colours.
//!
//! The theme of a file that is read is not kept: its colours are resolved
//! where they are used, and what is written is always this one.

use super::xml::XML_DECL;
use opensheet_engine::table::OFFICE_THEME;
use std::fmt::Write as _;

pub(crate) const CT_THEME: &str = "application/vnd.openxmlformats-officedocument.theme+xml";

/// Hyperlinks and followed hyperlinks, which the engine's table of theme
/// colours does not go up to.
const LINKS: [u32; 2] = [0x467886, 0x96607D];

/// The colours of the theme in the order of a theme part: dark 1, light 1,
/// dark 2, light 2, the six accents, hyperlink, followed hyperlink. (The
/// engine's table swaps each dark with its light, as cell colours do.)
pub(crate) const COLORS: [u32; 12] = [
    OFFICE_THEME[1],
    OFFICE_THEME[0],
    OFFICE_THEME[3],
    OFFICE_THEME[2],
    OFFICE_THEME[4],
    OFFICE_THEME[5],
    OFFICE_THEME[6],
    OFFICE_THEME[7],
    OFFICE_THEME[8],
    OFFICE_THEME[9],
    LINKS[0],
    LINKS[1],
];

/// Fills, lines and effects of the theme (`<a:fmtScheme>`): what shapes
/// with a theme style are drawn with. Nothing the writer makes refers to
/// them, but a theme is not valid without the three of each.
const FORMATS: &str = concat!(
    "<a:fmtScheme name=\"Office Theme\">",
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
    "</a:fmtScheme>",
);

/// The theme part.
pub(crate) fn xml() -> String {
    let mut x = String::from(XML_DECL);
    x.push_str("<a:theme xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" name=\"Office Theme\"><a:themeElements><a:clrScheme name=\"Office\">");
    // The first two are the system's text and window colours, with the
    // colours they have where the system says nothing.
    w!(
        x,
        "<a:dk1><a:sysClr val=\"windowText\" lastClr=\"{:06X}\"/></a:dk1><a:lt1><a:sysClr val=\"window\" lastClr=\"{:06X}\"/></a:lt1>",
        COLORS[0],
        COLORS[1]
    );
    let names = [
        "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5", "accent6", "hlink",
        "folHlink",
    ];
    for (name, color) in names.iter().zip(&COLORS[2..]) {
        w!(x, "<a:{name}><a:srgbClr val=\"{color:06X}\"/></a:{name}>");
    }
    x.push_str("</a:clrScheme><a:fontScheme name=\"Office\">");
    // Headings, then body text: the font of the cells of a new workbook.
    for (role, font) in [
        ("majorFont", "Aptos Display"),
        ("minorFont", "Aptos Narrow"),
    ] {
        w!(
            x,
            "<a:{role}><a:latin typeface=\"{font}\" panose=\"02110004020202020204\"/><a:ea typeface=\"\"/><a:cs typeface=\"\"/></a:{role}>"
        );
    }
    x.push_str("</a:fontScheme>");
    x.push_str(FORMATS);
    x.push_str("</a:themeElements><a:objectDefaults/><a:extraClrSchemeLst/></a:theme>");
    x
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xlsx::color::parse_theme;

    #[test]
    fn the_part_says_the_colours_of_the_engine() {
        let xml = xml();
        assert_eq!(parse_theme(&xml), COLORS);
        assert_eq!(COLORS[0], 0x000000);
        assert_eq!(COLORS[1], 0xFFFFFF);
        assert_eq!(COLORS[4], 0x156082, "accent 1");
        assert!(xml.contains("<a:accent6><a:srgbClr val=\"4EA72E\"/></a:accent6>"));
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
}
