//! Hyperlinks: the `<hyperlinks>` element of a worksheet. A link to a
//! place in the workbook is all there (`location`); one leading outside
//! names a relationship of the worksheet whose target is the address,
//! marked `TargetMode="External"`.

use super::parts::{Extras, SheetLinks};
use super::read::{Package, rels_part};
use super::write::{NS_DOC_REL, push_range};
use super::xml::{Event, Reader, push_attr};
use opensheet_engine::hyperlink::Hyperlink;
use opensheet_engine::workbook::Sheet;
use opensheet_engine::{Rect, Workbook};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{Read, Seek};

/// Append the `<hyperlinks>` element of a sheet (after the data
/// validations). The links leading outside take the relationship ids
/// `SheetLinks` keeps for them, in their order.
pub(crate) fn write(x: &mut String, sheet: &Sheet) {
    if sheet.hyperlinks.is_empty() {
        return;
    }
    let links = SheetLinks::of(sheet);
    let mut ids = links.hyperlinks.iter();
    x.push_str("<hyperlinks>");
    for link in &sheet.hyperlinks {
        x.push_str("<hyperlink ref=\"");
        push_range(x, link.range);
        x.push('"');
        if !link.address.is_empty() {
            if let Some(id) = ids.next() {
                w!(x, " r:id=\"rId{id}\"");
            }
        }
        if !link.location.is_empty() {
            x.push_str(" location=\"");
            push_attr(x, &link.location);
            x.push('"');
        }
        if !link.tooltip.is_empty() {
            x.push_str(" tooltip=\"");
            push_attr(x, &link.tooltip);
            x.push('"');
        }
        x.push_str("/>");
    }
    x.push_str("</hyperlinks>");
}

/// The relationships behind the links that lead outside the workbook.
pub(crate) fn add_parts(wb: &Workbook, extras: &mut Extras) {
    for (index, sheet) in wb.sheets.iter().enumerate() {
        let links = SheetLinks::of(sheet);
        let external = sheet.hyperlinks.iter().filter(|link| !link.address.is_empty());
        for (link, id) in external.zip(&links.hyperlinks) {
            extras.relate_sheet_outside(index, *id, &format!("{NS_DOC_REL}/hyperlink"), &link.address);
        }
    }
}

/// The targets of the relationships of a worksheet that lead outside the
/// package, by id.
fn outside_targets(xml: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut r = Reader::new(xml);
    loop {
        match r.next() {
            Event::Open(t) if t.name == "Relationship" => {
                let kind = t.attr("Type").unwrap_or_default();
                if kind.ends_with("/hyperlink") {
                    if let (Some(id), Some(target)) = (t.attr("Id"), t.attr("Target")) {
                        out.insert(id.into_owned(), target.into_owned());
                    }
                }
                r.skip(&t);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    out
}

/// Read the links of a worksheet from its XML and its relationships.
pub(super) fn read_sheet<R: Read + Seek>(pkg: &mut Package<R>, part: &str, xml: &str, sheet: &mut Sheet) {
    // Most sheets have none: no need to walk the cells again.
    let Some(start) = xml.find("<hyperlinks") else {
        return;
    };
    let targets = match pkg.read(&rels_part(part)) {
        Ok(Some(rels)) => outside_targets(&rels),
        _ => HashMap::new(),
    };
    let mut r = Reader::new(&xml[start..]);
    loop {
        match r.next() {
            Event::Open(t) if t.name == "hyperlink" => {
                let range = t.attr("ref").and_then(|range| Rect::parse(&range));
                let address = t.attr_local("id").and_then(|id| targets.get(id.as_ref()).cloned());
                let location = t.attr("location").map(|l| l.into_owned()).unwrap_or_default();
                r.skip(&t);
                // A link without a target, or to nothing we can place, is
                // dropped.
                let Some(range) = range else { continue };
                if address.is_none() && location.is_empty() {
                    continue;
                }
                sheet.hyperlinks.push(Hyperlink {
                    range,
                    address: address.unwrap_or_default(),
                    location,
                    tooltip: t.attr("tooltip").map(|tip| tip.into_owned()).unwrap_or_default(),
                });
            }
            Event::Close("hyperlinks") | Event::Eof => break,
            _ => {}
        }
    }
}
