//! The parts of a package beside the worksheets, and what ties them to the
//! sheets and to the workbook.
//!
//! Charts, notes and comments are kept in parts of their own. A worksheet
//! names such a part by the id of a relationship (`<drawing r:id="rId1"/>`),
//! its relationships part says which part that is, and `[Content_Types].xml`
//! says what kind of part it is. A feature states here what it adds; the
//! writer lays the package out from that.

use super::chart::write::Drawing;
use super::write::{NS_DOC_REL, NS_PKG_REL};
use super::xml::{XML_DECL, push_attr};
use opensheet_engine::Workbook;
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SheetLinks {
    /// The drawing part holding the charts (`<drawing>`).
    pub drawing: Option<usize>,
    /// The boxes of the notes, a VML drawing (`<legacyDrawing>`).
    pub vml: Option<usize>,
    /// The notes; the worksheet has no element for them.
    pub comments: Option<usize>,
    /// The comment threads; no element either.
    pub threads: Option<usize>,
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
            drawing: id(!sheet.charts.is_empty()),
            vml: id(annotated),
            comments: id(annotated),
            threads: id(threaded),
        }
    }

    /// Append the elements of a worksheet that name other parts. They come
    /// after the page setup, in this order.
    pub fn push_elements(&self, x: &mut String) {
        if let Some(id) = self.drawing {
            w!(x, "<drawing r:id=\"rId{id}\"/>");
        }
        if let Some(id) = self.vml {
            w!(x, "<legacyDrawing r:id=\"rId{id}\"/>");
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
    pub data: String,
}

/// Everything the features kept in parts of their own add to a package.
pub(crate) struct Extras {
    /// Content types given to all the parts with some extension:
    /// (extension, type).
    pub defaults: Vec<(&'static str, &'static str)>,
    /// What `xl/workbook.xml` relates to, beside its sheets and tables of
    /// styles and strings: (whole relationship type, target).
    pub workbook_rels: Vec<(String, String)>,
    pub parts: Vec<Part>,
    /// The relationships of each worksheet so far, as elements.
    sheet_rels: Vec<String>,
}

impl Extras {
    /// What the sheets of `wb` need beside the parts of the worksheets;
    /// `drawings` are the charts, which the writer stores itself.
    pub fn of(wb: &Workbook, drawings: &[Drawing]) -> Extras {
        let mut extras = Extras {
            defaults: Vec::new(),
            workbook_rels: Vec::new(),
            parts: Vec::new(),
            sheet_rels: vec![String::new(); wb.sheets.len()],
        };
        for drawing in drawings {
            let links = wb.sheets.get(drawing.sheet).map(SheetLinks::of);
            if let Some(id) = links.and_then(|links| links.drawing) {
                let target = format!("../drawings/drawing{}.xml", drawing.number);
                extras.relate_sheet(drawing.sheet, id, &format!("{NS_DOC_REL}/drawing"), &target);
            }
        }
        super::comments::add_parts(wb, &mut extras);

        for (index, rels) in std::mem::take(&mut extras.sheet_rels).iter().enumerate() {
            if rels.is_empty() {
                continue;
            }
            extras.parts.push(Part {
                name: format!("xl/worksheets/_rels/sheet{}.xml.rels", index + 1),
                content_type: None,
                data: format!(
                    "{XML_DECL}<Relationships xmlns=\"{NS_PKG_REL}\">{rels}</Relationships>"
                ),
            });
        }
        extras
    }

    /// Say that the parts named `*.{extension}` are of a type.
    pub fn add_default(&mut self, extension: &'static str, content_type: &'static str) {
        if !self.defaults.iter().any(|(known, _)| *known == extension) {
            self.defaults.push((extension, content_type));
        }
    }

    pub fn add_part(&mut self, name: String, content_type: Option<&'static str>, data: String) {
        self.parts.push(Part {
            name,
            content_type,
            data,
        });
    }

    /// Relate the worksheet of sheet `sheet` to a part, under the id
    /// [`SheetLinks`] gives it. `target` is relative to `xl/worksheets/`.
    pub fn relate_sheet(&mut self, sheet: usize, id: usize, kind: &str, target: &str) {
        if let Some(rels) = self.sheet_rels.get_mut(sheet) {
            push_relationship(rels, id, kind, target);
        }
    }

    /// Relate the workbook to a part; `target` is relative to `xl/`.
    pub fn relate_workbook(&mut self, kind: &str, target: &str) {
        self.workbook_rels
            .push((kind.to_string(), target.to_string()));
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
