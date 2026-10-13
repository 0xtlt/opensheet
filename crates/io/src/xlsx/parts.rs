//! The parts of a package beside the worksheets, and what ties them to the
//! sheets and to the workbook.
//!
//! Charts, notes and comments, tables, PivotTables are kept in parts of
//! their own. A worksheet names such a part by the id of a relationship
//! (`<drawing r:id="rId1"/>`), its relationships part says which part that
//! is, and `[Content_Types].xml` says what kind of part it is. A feature
//! states here what it adds; the writer lays the package out from that.

use super::chart::write::Drawing;
use super::dxf::Dxfs;
use super::theme;
use super::write::{NS_DOC_REL, NS_PKG_REL};
use super::xml::{XML_DECL, push_attr};
use opensheet_engine::Workbook;
use opensheet_engine::slicer::SlicerSource;
use opensheet_engine::workbook::Sheet;
use std::fmt::Write as _;

/// The family of the relationship types Microsoft added in 2017 (comment
/// threads and their authors); the older ones are in `NS_DOC_REL`.
pub(crate) const NS_REL_2017: &str = "http://schemas.microsoft.com/office/2017/10/relationships";

/// Append one relationship. `kind` is the whole type, `id` the number in
/// `rId{id}`.
pub(crate) fn push_relationship(x: &mut String, id: usize, kind: &str, target: &str) {
    w!(x, "<Relationship Id=\"rId{id}\" Type=\"{kind}\" Target=\"");
    push_attr(x, target);
    x.push_str("\"/>");
}

/// The relationships a worksheet has, by what they lead to. The ids are
/// numbered in the order of the fields, skipping what the sheet lacks: the
/// worksheet, written first, and its relationships part agree on them
/// because both ask here.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SheetLinks {
    /// The drawing part holding the charts, the pictures and the shapes
    /// (`<drawing>`).
    pub drawing: Option<usize>,
    /// The boxes of the notes, a VML drawing (`<legacyDrawing>`).
    pub vml: Option<usize>,
    /// The notes; the worksheet has no element for them.
    pub comments: Option<usize>,
    /// The comment threads; no element either.
    pub threads: Option<usize>,
    /// The tables, in the order of `Sheet::tables` (`<tableParts>`).
    pub tables: Vec<usize>,
    /// The PivotTables, in the order of `Sheet::pivots`; no element.
    pub pivots: Vec<usize>,
    /// The hyperlinks leading outside the workbook, in the order of
    /// `Sheet::hyperlinks` (`<hyperlinks>`).
    pub hyperlinks: Vec<usize>,
    /// The query tables of the text imports, in the order of
    /// `Sheet::text_imports`; no element.
    pub queries: Vec<usize>,
    /// The part listing the slicers of PivotTables, the one listing the
    /// slicers of tables and the one listing the timelines; they are
    /// named in the extensions of the worksheet.
    pub slicers: Option<usize>,
    pub table_slicers: Option<usize>,
    pub timelines: Option<usize>,
    /// The image behind the cells (`<picture>`).
    pub background: Option<usize>,
}

impl SheetLinks {
    pub fn of(sheet: &Sheet) -> SheetLinks {
        let mut count = 0;
        let mut id = |wanted: bool| {
            wanted.then(|| {
                count += 1;
                count
            })
        };
        // A thread comes with a note standing in for it, so both kinds
        // need the parts of notes.
        let annotated = !sheet.comments.is_empty();
        let threaded = sheet.comments.values().any(|c| c.as_thread().is_some());
        SheetLinks {
            // Slicers and timelines are frames of the drawing too.
            drawing: id(!sheet.charts.is_empty()
                || !sheet.drawings.is_empty()
                || !sheet.inks.is_empty()
                || !sheet.slicers.is_empty()
                || !sheet.timelines.is_empty()),
            vml: id(annotated),
            comments: id(annotated),
            threads: id(threaded),
            tables: sheet.tables.iter().filter_map(|_| id(true)).collect(),
            pivots: sheet.pivots.iter().filter_map(|_| id(true)).collect(),
            hyperlinks: sheet.hyperlinks.iter().filter_map(|link| id(!link.address.is_empty())).collect(),
            queries: sheet.text_imports.iter().filter_map(|_| id(true)).collect(),
            slicers: id(sheet.slicers.iter().any(|s| matches!(s.source, SlicerSource::Pivot { .. }))),
            table_slicers: id(sheet.slicers.iter().any(|s| matches!(s.source, SlicerSource::Table { .. }))),
            timelines: id(!sheet.timelines.is_empty()),
            background: id(sheet.background.is_some()),
        }
    }

    /// Append the elements of a worksheet that name other parts. They come
    /// after the page setup, in this order (that of the schema).
    pub fn push_elements(&self, x: &mut String) {
        if let Some(id) = self.drawing {
            w!(x, "<drawing r:id=\"rId{id}\"/>");
        }
        if let Some(id) = self.vml {
            w!(x, "<legacyDrawing r:id=\"rId{id}\"/>");
        }
        if let Some(id) = self.background {
            w!(x, "<picture r:id=\"rId{id}\"/>");
        }
        if !self.tables.is_empty() {
            w!(x, "<tableParts count=\"{}\">", self.tables.len());
            for id in &self.tables {
                w!(x, "<tablePart r:id=\"rId{id}\"/>");
            }
            x.push_str("</tableParts>");
        }
    }
}

/// A part to store as it is.
pub(crate) struct Part {
    /// Its name in the archive, without a leading slash.
    pub name: String,
    /// `None` when the extension of the name says it all (see
    /// [`Extras::defaults`]).
    pub content_type: Option<&'static str>,
    /// Text for most parts, the bytes of the file for a picture.
    pub data: Vec<u8>,
}

/// Everything the features kept in parts of their own add to a package.
pub(crate) struct Extras {
    /// Content types given to all the parts with some extension:
    /// (extension, type).
    pub defaults: Vec<(&'static str, &'static str)>,
    /// What `xl/workbook.xml` relates to, beside its sheets and tables of
    /// styles and strings: (whole relationship type, target).
    workbook_rels: Vec<(String, String)>,
    /// The pivot caches the workbook part lists: (`cacheId`, place of the
    /// relationship in `workbook_rels`).
    pivot_caches: Vec<(usize, usize)>,
    /// The caches of slicers and timelines, which the workbook part lists
    /// in its extensions: (which list, place of the relationship in
    /// `workbook_rels`).
    listed_caches: Vec<(CacheList, usize)>,
    pub parts: Vec<Part>,
    /// The relationships of each worksheet so far, as elements.
    sheet_rels: Vec<String>,
}

/// The lists of the workbook part's extensions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CacheList {
    PivotSlicers,
    TableSlicers,
    Timelines,
}

impl Extras {
    /// What `wb` needs beside the parts of the worksheets, of the styles
    /// and of the strings; `drawings` are the charts, which the writer
    /// stores itself. The formats these parts refer to get their ids from
    /// `dxfs`.
    pub fn of(wb: &Workbook, drawings: &mut Vec<Drawing>, dxfs: &mut Dxfs) -> Extras {
        let mut extras = Extras {
            defaults: Vec::new(),
            workbook_rels: Vec::new(),
            pivot_caches: Vec::new(),
            listed_caches: Vec::new(),
            parts: Vec::new(),
            sheet_rels: vec![String::new(); wb.sheets.len()],
        };
        super::drawing::add_parts(wb, drawings, &mut extras);
        super::ink::add_parts(wb, drawings);
        for drawing in drawings.iter() {
            let links = wb.sheets.get(drawing.sheet).map(SheetLinks::of);
            if let Some(id) = links.and_then(|links| links.drawing) {
                let target = format!("../drawings/drawing{}.xml", drawing.number);
                extras.relate_sheet(drawing.sheet, id, &format!("{NS_DOC_REL}/drawing"), &target);
            }
        }
        super::comments::add_parts(wb, &mut extras);
        super::table::add_parts(wb, &mut extras, dxfs);
        super::pivot::add_parts(wb, &mut extras, dxfs);
        super::slicer::add_parts(wb, &mut extras);
        super::timeline::add_parts(wb, &mut extras);
        super::hyperlink::add_parts(wb, &mut extras);
        super::datatools::add_parts(wb, &mut extras);
        super::background::add_parts(wb, &mut extras);
        extras.add_part(
            "xl/theme/theme1.xml".to_string(),
            Some(theme::CT_THEME),
            theme::xml(wb.theme()),
        );
        extras.relate_workbook(&format!("{NS_DOC_REL}/theme"), "theme/theme1.xml");

        for (index, rels) in std::mem::take(&mut extras.sheet_rels).iter().enumerate() {
            if rels.is_empty() {
                continue;
            }
            extras.add_part(
                format!("xl/worksheets/_rels/sheet{}.xml.rels", index + 1),
                None,
                format!("{XML_DECL}<Relationships xmlns=\"{NS_PKG_REL}\">{rels}</Relationships>"),
            );
        }
        extras
    }

    /// Say that the parts named `*.{extension}` are of a type.
    pub fn add_default(&mut self, extension: &'static str, content_type: &'static str) {
        if !self.defaults.iter().any(|(known, _)| *known == extension) {
            self.defaults.push((extension, content_type));
        }
    }

    pub fn add_part(
        &mut self,
        name: String,
        content_type: Option<&'static str>,
        data: impl Into<Vec<u8>>,
    ) {
        self.parts.push(Part {
            name,
            content_type,
            data: data.into(),
        });
    }

    /// Relate the worksheet of sheet `sheet` to a part, under the id
    /// [`SheetLinks`] gives it. `target` is relative to `xl/worksheets/`.
    pub fn relate_sheet(&mut self, sheet: usize, id: usize, kind: &str, target: &str) {
        if let Some(rels) = self.sheet_rels.get_mut(sheet) {
            push_relationship(rels, id, kind, target);
        }
    }

    /// Relate the worksheet of sheet `sheet` to something outside the
    /// package (the address of a hyperlink), under the id [`SheetLinks`]
    /// gives it.
    pub fn relate_sheet_outside(&mut self, sheet: usize, id: usize, kind: &str, target: &str) {
        if let Some(rels) = self.sheet_rels.get_mut(sheet) {
            w!(rels, "<Relationship Id=\"rId{id}\" Type=\"{kind}\" Target=\"");
            push_attr(rels, target);
            rels.push_str("\" TargetMode=\"External\"/>");
        }
    }

    /// Relate the workbook to a part; `target` is relative to `xl/`.
    pub fn relate_workbook(&mut self, kind: &str, target: &str) {
        self.workbook_rels
            .push((kind.to_string(), target.to_string()));
    }

    /// Relate the workbook to the definition of a pivot cache, which the
    /// workbook part also lists under `cache_id`.
    pub fn add_pivot_cache(&mut self, cache_id: usize, target: &str) {
        self.pivot_caches.push((cache_id, self.workbook_rels.len()));
        self.relate_workbook(&format!("{NS_DOC_REL}/pivotCacheDefinition"), target);
    }

    /// Relate the workbook to the cache of a slicer or of a timeline,
    /// which the workbook part also lists in the extension `list`.
    pub fn add_listed_cache(&mut self, list: CacheList, kind: &str, target: &str) {
        self.listed_caches.push((list, self.workbook_rels.len()));
        self.relate_workbook(kind, target);
    }

    /// The relationships of the workbook part to these parts, numbered
    /// from `first`: the writer has given the ids before it to the sheets
    /// and to its own parts.
    pub fn workbook_relationships(&self, first: usize) -> String {
        let mut x = String::new();
        for (n, (kind, target)) in self.workbook_rels.iter().enumerate() {
            push_relationship(&mut x, first + n, kind, target);
        }
        x
    }

    /// The elements of the workbook part that name these parts, under the
    /// ids [`Extras::workbook_relationships`] gives them: `<pivotCaches>`,
    /// which comes after the calculation properties.
    pub fn workbook_elements(&self, first: usize) -> String {
        let mut x = String::new();
        if !self.pivot_caches.is_empty() {
            x.push_str("<pivotCaches>");
            for (cache_id, rel) in &self.pivot_caches {
                w!(
                    x,
                    "<pivotCache cacheId=\"{cache_id}\" r:id=\"rId{}\"/>",
                    first + rel
                );
            }
            x.push_str("</pivotCaches>");
        }
        // extLst: the caches of the slicers of PivotTables, then those of
        // the slicers of tables, then those of the timelines.
        let mut lists = String::new();
        for list in [CacheList::PivotSlicers, CacheList::TableSlicers, CacheList::Timelines] {
            let ids: Vec<usize> =
                self.listed_caches.iter().filter(|(l, _)| *l == list).map(|(_, rel)| first + rel).collect();
            if !ids.is_empty() {
                super::slicer::push_workbook_ext(&mut lists, list, &ids);
            }
        }
        if !lists.is_empty() {
            w!(x, "<extLst>{lists}</extLst>");
        }
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::Locale;
    use opensheet_engine::comment::Person;

    #[test]
    fn relationship_ids_follow_what_a_sheet_has() {
        let mut wb = Workbook::new(Locale::en());
        assert_eq!(SheetLinks::of(&wb.sheets[0]), SheetLinks::default());
        let mut x = String::new();
        SheetLinks::of(&wb.sheets[0]).push_elements(&mut x);
        assert_eq!(x, "");

        assert!(wb.set_note(0, 0, 0, "Ann", "Ann:\nhello"));
        let links = SheetLinks::of(&wb.sheets[0]);
        assert_eq!(
            (links.drawing, links.vml, links.comments, links.threads),
            (None, Some(1), Some(2), None)
        );
        links.push_elements(&mut x);
        assert_eq!(x, "<legacyDrawing r:id=\"rId1\"/>");

        // A thread alone still needs the two parts of notes, for the note
        // that stands in for it.
        assert!(wb.delete_comment(0, 0, 0));
        assert!(wb.add_comment(0, 2, 2, &Person::new("Bob"), 45000.5, "why?"));
        let links = SheetLinks::of(&wb.sheets[0]);
        assert_eq!(
            (links.drawing, links.vml, links.comments, links.threads),
            (None, Some(1), Some(2), Some(3))
        );
    }

    #[test]
    fn relationships_are_escaped() {
        let mut x = String::new();
        push_relationship(&mut x, 3, "urn:kind", "../a&b \"c\".xml");
        assert_eq!(
            x,
            "<Relationship Id=\"rId3\" Type=\"urn:kind\" Target=\"../a&amp;b &quot;c&quot;.xml\"/>"
        );
    }
}
