//! What the parts beside the worksheets name in the styles part: number
//! formats (`numFmtId`, in pivot caches and PivotTables) and differential
//! formats (`dxfId`, in filters and sorts).
//!
//! The styles part is built from the formats of the cells before anyone
//! knows what else the package will refer to. [`StyleRefs`] hands out the
//! ids while the other parts are written and puts what was added into the
//! styles part at the end; [`StyleLookup`] is the way back when reading.

use super::color::Palette;
use super::dxf::{DxfColors, Dxfs, parse_colors};
use super::numfmt::{FIRST_CUSTOM_ID, builtin_code, builtin_id};
use super::xml::{Event, Reader, push_attr};
use opensheet_engine::Locale;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

/// The number formats of the package being written, by id.
pub(crate) struct NumFmts {
    locale: &'static Locale,
    /// The custom formats: those of the styles part as it was built, then
    /// those added since.
    custom: Vec<(u32, String)>,
    /// How many of them the styles part already lists.
    listed: usize,
}

/// The `<numFmt>` elements of a styles part: (id, format code).
fn parse_num_fmts(styles: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    let mut r = Reader::new(styles);
    loop {
        match r.next() {
            Event::Open(t) if t.name == "styleSheet" => {}
            Event::Open(t) if t.name == "numFmts" && !t.empty => loop {
                match r.next() {
                    Event::Open(f) => {
                        if let (true, Some(id), Some(code)) = (
                            f.name == "numFmt",
                            f.attr_u32("numFmtId"),
                            f.attr("formatCode"),
                        ) {
                            out.push((id, code.into_owned()));
                        }
                        r.skip(&f);
                    }
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

impl NumFmts {
    fn of(styles: &str, locale: &'static Locale) -> NumFmts {
        let custom = parse_num_fmts(styles);
        NumFmts {
            locale,
            listed: custom.len(),
            custom,
        }
    }

    /// The id of a format code: the built-in one when it has one, the id
    /// the styles part gives it, or a new one.
    pub fn id(&mut self, code: &str) -> u32 {
        if let Some(id) = builtin_id(code, self.locale) {
            return id;
        }
        if let Some((id, _)) = self.custom.iter().find(|(_, known)| known == code) {
            return *id;
        }
        let taken = self.custom.iter().map(|(id, _)| *id).max();
        let id = taken.map_or(FIRST_CUSTOM_ID, |id| id.max(FIRST_CUSTOM_ID - 1) + 1);
        self.custom.push((id, code.to_string()));
        id
    }

    /// The styles part with the formats added since it was built.
    fn apply(&self, styles: &str) -> String {
        let added = &self.custom[self.listed..];
        if added.is_empty() {
            return styles.to_string();
        }
        let mut list = String::new();
        for (id, code) in added {
            w!(list, "<numFmt numFmtId=\"{id}\" formatCode=\"");
            push_attr(&mut list, code);
            list.push_str("\"/>");
        }
        match (styles.find("<numFmts count=\""), styles.find("</numFmts>")) {
            // A list to extend: its count, and its end.
            (Some(start), Some(end)) if start < end => {
                let count_at = start + "<numFmts count=\"".len();
                let count_end = count_at + styles[count_at..].find('"').unwrap_or(0);
                format!(
                    "{}{}{}{list}{}",
                    &styles[..count_at],
                    self.custom.len(),
                    &styles[count_end..end],
                    &styles[end..]
                )
            }
            // No list yet: it is the first thing in a styles part.
            _ => {
                let root = styles.find("<styleSheet").unwrap_or(0);
                let at = root + styles[root..].find('>').map_or(0, |i| i + 1);
                format!(
                    "{}<numFmts count=\"{}\">{list}</numFmts>{}",
                    &styles[..at],
                    added.len(),
                    &styles[at..]
                )
            }
        }
    }
}

/// The ids other parts take in the styles part being written.
pub(crate) struct StyleRefs {
    pub dxfs: Dxfs,
    pub num_fmts: NumFmts,
}

impl StyleRefs {
    /// `styles` is the styles part as it was built from the cells.
    pub fn of(styles: &str, locale: &'static Locale) -> StyleRefs {
        StyleRefs {
            dxfs: Dxfs::default(),
            num_fmts: NumFmts::of(styles, locale),
        }
    }

    /// The styles part with everything the other parts came to name.
    pub fn apply(&self, styles: &str) -> String {
        self.dxfs.apply(&self.num_fmts.apply(styles))
    }
}

/// What the ids found in other parts of a file stand for.
pub(crate) struct StyleLookup {
    /// The colours of the differential formats, by `dxfId`.
    pub dxf_colors: Vec<DxfColors>,
    custom: HashMap<u32, Arc<str>>,
    locale: &'static Locale,
}

impl StyleLookup {
    /// `styles` is the styles part of the file, if it has one.
    pub fn read(styles: Option<&str>, palette: &Palette, locale: &'static Locale) -> StyleLookup {
        let styles = styles.unwrap_or("");
        StyleLookup {
            dxf_colors: parse_colors(styles, palette),
            custom: parse_num_fmts(styles)
                .into_iter()
                .map(|(id, code)| (id, Arc::from(code)))
                .collect(),
            locale,
        }
    }

    /// The format code of a `numFmtId`; "General" for one nothing defines.
    pub fn num_fmt(&self, id: u32) -> Arc<str> {
        if let Some(code) = self.custom.get(&id) {
            return code.clone();
        }
        match builtin_code(id, self.locale) {
            Some(code) if !code.trim().is_empty() => Arc::from(code.as_ref()),
            _ => Arc::from("General"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_formats_get_the_ids_of_the_styles_part() {
        let en = Locale::en();
        let styles = "<?xml version=\"1.0\"?>\n<styleSheet xmlns=\"x\"><numFmts count=\"2\"><numFmt numFmtId=\"164\" formatCode=\"0.0%\"/><numFmt numFmtId=\"170\" formatCode=\"&quot;x&quot; 0\"/></numFmts><fonts/><dxfs count=\"0\"/></styleSheet>";
        let mut refs = StyleRefs::of(styles, en);
        assert_eq!(refs.apply(styles), styles);
        assert_eq!(refs.num_fmts.id("General"), 0);
        assert_eq!(refs.num_fmts.id("0.00"), 2);
        assert_eq!(refs.num_fmts.id(en.short_date_format), 14);
        assert_eq!(refs.num_fmts.id("0.0%"), 164);
        assert_eq!(refs.num_fmts.id("\"x\" 0"), 170);
        assert_eq!(refs.num_fmts.id("0.000 \"kg\""), 171);
        assert_eq!(refs.num_fmts.id("[Red]0"), 172);
        assert_eq!(refs.num_fmts.id("0.000 \"kg\""), 171);
        assert_eq!(refs.dxfs.fill(None), 0);
        let applied = refs.apply(styles);
        assert_eq!(
            applied,
            "<?xml version=\"1.0\"?>\n<styleSheet xmlns=\"x\"><numFmts count=\"4\"><numFmt numFmtId=\"164\" formatCode=\"0.0%\"/><numFmt numFmtId=\"170\" formatCode=\"&quot;x&quot; 0\"/><numFmt numFmtId=\"171\" formatCode=\"0.000 &quot;kg&quot;\"/><numFmt numFmtId=\"172\" formatCode=\"[Red]0\"/></numFmts><fonts/><dxfs count=\"1\"><dxf><fill><patternFill patternType=\"none\"/></fill></dxf></dxfs></styleSheet>"
        );
        // And back.
        let lookup = StyleLookup::read(Some(&applied), &Palette::default(), en);
        assert_eq!(&*lookup.num_fmt(171), "0.000 \"kg\"");
        assert_eq!(&*lookup.num_fmt(164), "0.0%");
        assert_eq!(&*lookup.num_fmt(2), "0.00");
        assert_eq!(&*lookup.num_fmt(14), en.short_date_format);
        assert_eq!(&*lookup.num_fmt(0), "General");
        assert_eq!(&*lookup.num_fmt(9999), "General");
        assert_eq!(lookup.dxf_colors.len(), 1);

        // A styles part without custom formats gets its list.
        let bare = "<styleSheet xmlns=\"x\"><fonts/></styleSheet>";
        let mut refs = StyleRefs::of(bare, en);
        assert_eq!(refs.num_fmts.id("0.0%"), 164);
        assert_eq!(
            refs.apply(bare),
            "<styleSheet xmlns=\"x\"><numFmts count=\"1\"><numFmt numFmtId=\"164\" formatCode=\"0.0%\"/></numFmts><fonts/></styleSheet>"
        );
        let nothing = StyleLookup::read(None, &Palette::default(), en);
        assert_eq!(&*nothing.num_fmt(164), "General");
    }
}
