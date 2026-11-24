//! Checkboxes in cells (Insert > Checkbox). Excel keeps them in a part of
//! "feature property bags": a cell format that shows a checkbox carries an
//! `<xfpb:xfComplement i="n"/>` extension, and complement n leads, from bag
//! to bag, to a bag of type `Checkbox`:
//!
//! ```text
//! XFComplements ─MappedFeaturePropertyBags[n]→ XFComplement
//!   ─XFControls→ XFControls ─CellControl→ Checkbox
//! ```
//!
//! What is written is what Excel writes for a workbook with checkboxes.

use super::parts::Extras;
use super::xml::{Event, Reader, XML_DECL};
use opensheet_engine::Workbook;
use std::collections::HashSet;

/// The relationship type, from the workbook part.
pub(crate) const REL: &str =
    "http://schemas.microsoft.com/office/2022/11/relationships/FeaturePropertyBag";
const CT: &str = "application/vnd.ms-excel.featurepropertybag+xml";
const NS: &str = "http://schemas.microsoft.com/office/spreadsheetml/2022/featurepropertybag";
const PART: &str = "featurePropertyBag/featurePropertyBag.xml";

/// The extension of a cell format that shows a checkbox (complement 0 of
/// the part this module writes).
pub(crate) fn xf_extension() -> String {
    format!(
        "<extLst><ext uri=\"{{C7286773-470A-42A8-94C5-96B5CB345126}}\" xmlns:xfpb=\"{NS}\"><xfpb:xfComplement i=\"0\"/></ext></extLst>"
    )
}

/// Whether a cell, a row or a column of the workbook is formatted with a
/// checkbox.
fn used(wb: &Workbook) -> bool {
    let mut seen = HashSet::new();
    wb.sheets.iter().any(|sheet| {
        sheet
            .cells
            .values()
            .map(|cell| cell.style)
            .chain(sheet.col_styles.values().copied())
            .chain(sheet.row_styles.values().copied())
            .any(|id| seen.insert(id) && wb.styles.get(id).checkbox)
    })
}

pub(crate) fn add_parts(wb: &Workbook, extras: &mut Extras) {
    if !used(wb) {
        return;
    }
    let xml = format!(
        "{XML_DECL}<FeaturePropertyBags xmlns=\"{NS}\"><bag type=\"Checkbox\"/><bag type=\"XFControls\"><bagId k=\"CellControl\">0</bagId></bag><bag type=\"XFComplement\"><bagId k=\"XFControls\">1</bagId></bag><bag type=\"XFComplements\" extRef=\"XFComplementsMapperExtRef\"><a k=\"MappedFeaturePropertyBags\"><bagId>2</bagId></a></bag></FeaturePropertyBags>"
    );
    extras.add_part(format!("xl/{PART}"), Some(CT), xml);
    extras.relate_workbook(REL, PART);
}

/// One `<bag>`: its type, the bags it names by key, and the list of
/// mapped bags of the `XFComplements` bag.
#[derive(Default)]
struct Bag {
    kind: String,
    ids: Vec<(String, usize)>,
    mapped: Vec<usize>,
}

fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// For each `xfComplement` index, whether it stands for a checkbox.
pub(crate) fn parse(xml: &str) -> Vec<bool> {
    let mut bags: Vec<Bag> = Vec::new();
    let mut r = Reader::new(xml);
    // Inside a bag: depth of the elements open in it, and the key of the
    // `<a>` array being read.
    let mut depth = 0usize;
    let mut array: Option<String> = None;
    loop {
        match r.next() {
            Event::Open(t) => {
                let name = local(t.name);
                if depth == 0 {
                    if name == "bag" {
                        bags.push(Bag {
                            kind: t.attr("type").map(|k| k.into_owned()).unwrap_or_default(),
                            ..Bag::default()
                        });
                        if !t.empty {
                            depth = 1;
                        }
                    }
                    continue;
                }
                match name {
                    "a" => array = t.attr("k").map(|k| k.into_owned()),
                    "bagId" => {
                        let key = t.attr("k").map(|k| k.into_owned());
                        let id = r.text(&t).trim().parse::<usize>().ok();
                        if let (Some(id), Some(bag)) = (id, bags.last_mut()) {
                            match key {
                                Some(key) => bag.ids.push((key, id)),
                                None if array.as_deref() == Some("MappedFeaturePropertyBags") => {
                                    bag.mapped.push(id)
                                }
                                None => {}
                            }
                        }
                        continue;
                    }
                    _ => {}
                }
                if !t.empty {
                    depth += 1;
                }
            }
            Event::Close(name) => {
                if local(name) == "a" {
                    array = None;
                }
                depth = depth.saturating_sub(1);
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let follow = |bag: usize, key: &str| -> Option<usize> {
        bags.get(bag)?
            .ids
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, id)| *id)
    };
    let is_checkbox = |complement: usize| -> bool {
        follow(complement, "XFControls")
            .and_then(|controls| follow(controls, "CellControl"))
            .and_then(|control| bags.get(control))
            .is_some_and(|bag| bag.kind == "Checkbox")
    };
    bags.iter()
        .find(|bag| bag.kind == "XFComplements")
        .map(|bag| bag.mapped.iter().map(|b| is_checkbox(*b)).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_part_excel_writes_maps_complement_0_to_a_checkbox() {
        let xml = format!(
            "{XML_DECL}<FeaturePropertyBags xmlns=\"{NS}\"><bag type=\"Checkbox\"/><bag type=\"XFControls\"><bagId k=\"CellControl\">0</bagId></bag><bag type=\"XFComplement\"><bagId k=\"XFControls\">1</bagId></bag><bag type=\"XFComplements\" extRef=\"XFComplementsMapperExtRef\"><a k=\"MappedFeaturePropertyBags\"><bagId>2</bagId></a></bag></FeaturePropertyBags>"
        );
        assert_eq!(parse(&xml), vec![true]);
        assert!(parse("").is_empty());
    }
}
