//! What the parts beside the worksheets name in the styles part of a file
//! that is read: number formats (`numFmtId`, in pivot caches and
//! PivotTables) and the colours of differential formats (`dxfId`, in
//! filters and sorts).
//!
//! The way out is the registry of `dxf.rs`, which hands out both kinds of
//! id while the parts are written.

use super::color::Palette;
use super::filter::{DxfColors, parse_colors};
use super::numfmt::builtin_code;
use super::xml::{Event, Reader};
use opensheet_engine::Locale;
use std::collections::HashMap;
use std::sync::Arc;

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
    use crate::xlsx::dxf::Dxfs;
    use opensheet_engine::Color;

    #[test]
    fn ids_come_back_as_what_they_stood_for() {
        let en = Locale::en();
        // The registry of a workbook whose cells use one custom format.
        let mut dxfs = Dxfs::new(vec!["0.0%".to_string()], en);
        let mut id = |code: &str| dxfs.num_fmt_id(&Arc::from(code));
        assert_eq!(id("General"), 0);
        assert_eq!(id("0.00"), 2);
        assert_eq!(id(en.short_date_format), 14);
        assert_eq!(id("0.0%"), 164);
        assert_eq!(id("0.000 \"kg\""), 165);
        assert_eq!(id("[Red]0"), 166);
        assert_eq!(id("0.000 \"kg\""), 165);
        assert_eq!(dxfs.cell_fill_id(Some(Color(0x00B050))), 0);
        assert_eq!(dxfs.own_formats().len(), 2);

        let styles = format!(
            "<styleSheet xmlns=\"x\"><numFmts count=\"3\"><numFmt numFmtId=\"164\" formatCode=\"0.0%\"/><numFmt numFmtId=\"165\" formatCode=\"0.000 &quot;kg&quot;\"/><numFmt numFmtId=\"166\" formatCode=\"[Red]0\"/></numFmts><fonts/>{}</styleSheet>",
            dxfs.xml()
        );
        let lookup = StyleLookup::read(Some(&styles), &Palette::default(), en);
        assert_eq!(&*lookup.num_fmt(165), "0.000 \"kg\"");
        assert_eq!(&*lookup.num_fmt(164), "0.0%");
        assert_eq!(&*lookup.num_fmt(2), "0.00");
        assert_eq!(&*lookup.num_fmt(14), en.short_date_format);
        assert_eq!(&*lookup.num_fmt(0), "General");
        assert_eq!(&*lookup.num_fmt(9999), "General");
        assert_eq!(
            lookup.dxf_colors,
            [DxfColors {
                font: None,
                fill: Some(Color(0x00B050))
            }]
        );

        // No styles part at all, and one cut anywhere.
        let lookup = StyleLookup::read(None, &Palette::default(), en);
        assert_eq!(&*lookup.num_fmt(164), "General");
        for cut in (0..styles.len()).filter(|i| styles.is_char_boundary(*i)) {
            StyleLookup::read(Some(&styles[..cut]), &Palette::default(), en);
        }
    }
}
