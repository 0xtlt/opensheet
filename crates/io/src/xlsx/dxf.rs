//! Differential formats: the `<dxfs>` list of the styles part.
//!
//! A `<dxf>` holds the part of a format that some rule adds to cells: the
//! colour a filter or a sort goes by, the format of a conditional format.
//! Whatever uses one names it by its place in the list (`dxfId`), so the
//! package has a single list, [`Dxfs`], that every writer adds to.
//!
//! Reading is each user's own business: this module only reads the two
//! colours filters and sorts need, [`DxfColors`].

use super::color::{Palette, RawColor};
use super::xml::{Event, Reader};
use opensheet_engine::Color;
use std::collections::HashMap;
use std::fmt::Write as _;

/// The `<dxf>` elements of the package being written. An element is added
/// as its text, once: equal texts share an id.
#[derive(Default)]
pub(crate) struct Dxfs {
    list: Vec<String>,
    ids: HashMap<String, u32>,
}

/// The empty list the styles part is built with, before anyone knows what
/// goes in it.
const EMPTY: &str = "<dxfs count=\"0\"/>";

impl Dxfs {
    /// The id of a `<dxf>…</dxf>` element, added to the list when it is
    /// new.
    pub fn id(&mut self, dxf: String) -> u32 {
        if let Some(id) = self.ids.get(&dxf) {
            return *id;
        }
        let id = self.list.len() as u32;
        self.list.push(dxf.clone());
        self.ids.insert(dxf, id);
        id
    }

    /// The id of the format that is nothing but a fill colour (`None`:
    /// no fill). Both colours of the pattern say it: applications differ
    /// on which one a solid fill takes in a `<dxf>`.
    pub fn fill(&mut self, color: Option<Color>) -> u32 {
        self.id(match color {
            Some(color) => {
                let rgb = color.0 & 0xFF_FFFF;
                format!(
                    "<dxf><fill><patternFill patternType=\"solid\"><fgColor rgb=\"FF{rgb:06X}\"/><bgColor rgb=\"FF{rgb:06X}\"/></patternFill></fill></dxf>"
                )
            }
            None => "<dxf><fill><patternFill patternType=\"none\"/></fill></dxf>".to_string(),
        })
    }

    /// The id of the format that is nothing but a text colour (`None`:
    /// automatic).
    pub fn font_color(&mut self, color: Option<Color>) -> u32 {
        self.id(match color {
            Some(color) => format!(
                "<dxf><font><color rgb=\"FF{:06X}\"/></font></dxf>",
                color.0 & 0xFF_FFFF
            ),
            None => "<dxf><font><color auto=\"1\"/></font></dxf>".to_string(),
        })
    }

    /// The styles part with the list in it. `styles` is the part as it was
    /// built, with an empty list.
    pub fn apply(&self, styles: &str) -> String {
        if self.list.is_empty() {
            return styles.to_string();
        }
        let mut list = String::new();
        w!(list, "<dxfs count=\"{}\">", self.list.len());
        for dxf in &self.list {
            list.push_str(dxf);
        }
        list.push_str("</dxfs>");
        debug_assert!(
            styles.contains(EMPTY),
            "the styles part has no empty <dxfs>"
        );
        styles.replacen(EMPTY, &list, 1)
    }
}

/// The colours of one `<dxf>` of a file that is read.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct DxfColors {
    /// The colour of the text; `None` when the format has none or says
    /// automatic.
    pub font: Option<Color>,
    /// The fill; `None` when the format has none or says no fill.
    pub fill: Option<Color>,
}

/// The colours of the `<dxf>` elements of a styles part, by `dxfId`.
pub(crate) fn parse_colors(styles: &str, palette: &Palette) -> Vec<DxfColors> {
    let mut out = Vec::new();
    let mut r = Reader::new(styles);
    loop {
        match r.next() {
            Event::Open(t) if t.name == "styleSheet" => {}
            Event::Open(t) if t.name == "dxfs" && !t.empty => loop {
                match r.next() {
                    Event::Open(dxf) if dxf.name == "dxf" => {
                        let mut colors = DxfColors::default();
                        // Foreground and background of the pattern, and
                        // whether it is one that shows.
                        let (mut fg, mut bg, mut solid) = (None, None, true);
                        let mut font = false;
                        let mut depth = usize::from(!dxf.empty);
                        while depth > 0 {
                            match r.next() {
                                Event::Open(t) => {
                                    match t.name {
                                        "font" => font = !t.empty,
                                        "color" if font => {
                                            colors.font = RawColor::from_tag(&t).resolve(palette);
                                        }
                                        "patternFill" => {
                                            solid =
                                                t.attr("patternType").as_deref() != Some("none");
                                        }
                                        "fgColor" => fg = RawColor::from_tag(&t).resolve(palette),
                                        "bgColor" => bg = RawColor::from_tag(&t).resolve(palette),
                                        _ => {}
                                    }
                                    depth += usize::from(!t.empty);
                                }
                                Event::Close(name) => {
                                    font &= name != "font";
                                    depth -= 1;
                                }
                                Event::Eof => break,
                                Event::Text(_) => {}
                            }
                        }
                        // Excel writes the colour a filter goes by as the
                        // foreground of a solid pattern, and the fill of a
                        // conditional format as the background.
                        colors.fill = fg.or(bg).filter(|_| solid);
                        out.push(colors);
                    }
                    Event::Open(t) => r.skip(&t),
                    Event::Close(_) | Event::Eof => break,
                    Event::Text(_) => {}
                }
            },
            Event::Open(t) => r.skip(&t),
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_formats_share_an_id() {
        let mut dxfs = Dxfs::default();
        let styles = format!("<styleSheet><cellXfs/>{EMPTY}<tableStyles/></styleSheet>");
        assert_eq!(dxfs.apply(&styles), styles);
        assert_eq!(dxfs.fill(Some(Color(0xFFFF00))), 0);
        assert_eq!(dxfs.font_color(Some(Color(0xFF0000))), 1);
        assert_eq!(dxfs.fill(Some(Color(0xFFFF00))), 0);
        assert_eq!(dxfs.fill(None), 2);
        assert_eq!(dxfs.font_color(None), 3);
        assert_eq!(dxfs.id("<dxf><font><b/></font></dxf>".to_string()), 4);
        let styles = dxfs.apply(&styles);
        assert!(styles.starts_with("<styleSheet><cellXfs/><dxfs count=\"5\"><dxf><fill>"));
        assert!(styles.ends_with("<dxf><font><b/></font></dxf></dxfs><tableStyles/></styleSheet>"));

        // And back.
        let colors = parse_colors(&styles, &Palette::default());
        let color = |font: Option<u32>, fill: Option<u32>| DxfColors {
            font: font.map(Color),
            fill: fill.map(Color),
        };
        assert_eq!(
            colors,
            [
                color(None, Some(0xFFFF00)),
                color(Some(0xFF0000), None),
                color(None, None),
                color(None, None),
                color(None, None),
            ]
        );
    }

    #[test]
    fn colours_of_formats_from_excel() {
        // A filter colour, a conditional format, theme and indexed colours,
        // and elements that are not colours of the format itself.
        let styles = r#"<styleSheet><fonts><font><color rgb="FF123456"/></font></fonts><dxfs count="5">
            <dxf><fill><patternFill patternType="solid"><fgColor rgb="FFFFFF00"/><bgColor rgb="FF000000"/></patternFill></fill></dxf>
            <dxf><font><b/><color rgb="FF9C0006"/></font><fill><patternFill><bgColor rgb="FFFFC7CE"/></patternFill></fill><border><left style="thin"><color rgb="FF00FF00"/></left></border></dxf>
            <dxf><font><color theme="4"/></font><fill><patternFill><bgColor indexed="10"/></patternFill></fill></dxf>
            <dxf/>
            <dxf><fill><patternFill patternType="none"><fgColor rgb="FFFF0000"/></patternFill></fill></dxf>
            </dxfs><extLst><ext><dxfs><dxf><font><color rgb="FF000001"/></font></dxf></dxfs></ext></extLst></styleSheet>"#;
        let colors = parse_colors(styles, &Palette::default());
        assert_eq!(colors.len(), 5);
        assert_eq!(
            colors[0],
            DxfColors {
                font: None,
                fill: Some(Color(0xFFFF00))
            }
        );
        assert_eq!(
            colors[1],
            DxfColors {
                font: Some(Color(0x9C0006)),
                fill: Some(Color(0xFFC7CE))
            }
        );
        assert_eq!(colors[2].font, Some(Color(0x4472C4)));
        assert_eq!(colors[2].fill, Some(Color(0xFF0000)));
        assert_eq!(colors[3], DxfColors::default());
        assert_eq!(colors[4], DxfColors::default());
        // Cut short anywhere, it still ends.
        for cut in (0..styles.len()).filter(|i| styles.is_char_boundary(*i)) {
            parse_colors(&styles[..cut], &Palette::default());
        }
    }
}
