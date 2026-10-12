//! What the data tools keep in a worksheet part: `<dataConsolidate>`, the
//! settings of Data > Consolidate.

use super::write::push_range;
use super::xml::{Event, Reader, Tag, push_attr};
use opensheet_engine::consolidate::{ConsolidateFunction, ConsolidateSource, Consolidation};
use opensheet_engine::{Rect, Sheet};
use std::fmt::Write;

/// Read `<dataConsolidate>`: the function, the labels used and the
/// references. References to other workbooks (`r:id`) and to names are
/// left out; "Create links to source data" is not modelled.
pub(crate) fn read_consolidate<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, sheet: &mut Sheet) {
    let mut spec = Consolidation {
        function: tag
            .attr("function")
            .and_then(|name| ConsolidateFunction::from_ooxml_name(&name))
            .unwrap_or_default(),
        sources: Vec::new(),
        top_row: tag.attr_bool("topLabels").unwrap_or(false),
        left_column: tag.attr_bool("leftLabels").unwrap_or(false),
    };
    if !tag.empty {
        loop {
            match r.next() {
                Event::Open(t) => {
                    if t.name == "dataRef" && t.attr_local("id").is_none() {
                        let range = t.attr("ref").and_then(|text| Rect::parse(&text.replace('$', "")));
                        if let (Some(range), Some(name)) = (range, t.attr("sheet")) {
                            spec.sources.push(ConsolidateSource {
                                sheet: name.into_owned(),
                                range,
                            });
                        }
                    }
                    if t.name != "dataRefs" {
                        r.skip(&t);
                    }
                }
                Event::Close(name) if name == "dataRefs" => {}
                Event::Close(_) | Event::Eof => break,
                _ => {}
            }
        }
    }
    sheet.consolidation = Some(spec);
}

/// Append `<dataConsolidate>`.
pub(crate) fn write_consolidate(x: &mut String, sheet: &Sheet) {
    let Some(spec) = &sheet.consolidation else {
        return;
    };
    x.push_str("<dataConsolidate");
    if spec.function != ConsolidateFunction::Sum {
        w!(x, " function=\"{}\"", spec.function.ooxml_name());
    }
    if spec.left_column {
        x.push_str(" leftLabels=\"1\"");
    }
    if spec.top_row {
        x.push_str(" topLabels=\"1\"");
    }
    if spec.sources.is_empty() {
        x.push_str("/>");
        return;
    }
    w!(x, "><dataRefs count=\"{}\">", spec.sources.len());
    for source in &spec.sources {
        x.push_str("<dataRef ref=\"");
        push_range(x, source.range);
        x.push_str("\" sheet=\"");
        push_attr(x, &source.sheet);
        x.push_str("\"/>");
    }
    x.push_str("</dataRefs></dataConsolidate>");
}
