//! What the data tools keep in a worksheet part: `<dataConsolidate>`, the
//! settings of Data > Consolidate; `<scenarios>`, the scenarios of the
//! Scenario Manager; and the data tables of What-If Analysis, which are a
//! formula of their own kind (`<f t="dataTable">`) on their first cell.

use super::read::parse_cell_ref;
use super::write::{push_number, push_range};
use super::xml::{Event, Reader, Tag, push_attr};
use opensheet_engine::consolidate::{ConsolidateFunction, ConsolidateSource, Consolidation};
use opensheet_engine::input::parse_numeric_text;
use opensheet_engine::refs::cell_name;
use opensheet_engine::whatif::{DataTable, Scenario, ScenarioCell};
use opensheet_engine::{Locale, Rect, Sheet};
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

/// Read `<scenarios>`. The value of a changing cell is kept as it would
/// be typed in `locale` (a file writes numbers with a point).
pub(crate) fn read_scenarios<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, sheet: &mut Sheet, locale: &Locale) {
    if tag.empty {
        return;
    }
    loop {
        match r.next() {
            Event::Open(t) if t.name == "scenario" => {
                let mut scenario = Scenario {
                    name: t.attr("name").unwrap_or_default().into_owned(),
                    comment: t.attr("comment").unwrap_or_default().into_owned(),
                    user: t.attr("user").unwrap_or_default().into_owned(),
                    locked: t.attr_bool("locked").unwrap_or(false),
                    hidden: t.attr_bool("hidden").unwrap_or(false),
                    cells: Vec::new(),
                };
                if !t.empty {
                    loop {
                        match r.next() {
                            Event::Open(c) => {
                                let cell = c.attr("r").and_then(|name| parse_cell_ref(&name));
                                if let (true, Some((row, col))) = (c.name == "inputCells", cell) {
                                    let value = c.attr("val").unwrap_or_default();
                                    let value = match value.trim().parse::<f64>() {
                                        Ok(n) if n.is_finite() => value.trim().replace('.', &locale.decimal_sep.to_string()),
                                        _ => value.into_owned(),
                                    };
                                    scenario.cells.push(ScenarioCell { row, col, value });
                                }
                                r.skip(&c);
                            }
                            Event::Close(_) | Event::Eof => break,
                            _ => {}
                        }
                    }
                }
                // A scenario needs a name of its own and cells to change.
                let known = sheet.scenarios.iter().any(|other| other.name.eq_ignore_ascii_case(&scenario.name));
                if !scenario.name.trim().is_empty() && !scenario.cells.is_empty() && !known {
                    sheet.scenarios.push(scenario);
                }
            }
            Event::Open(t) => r.skip(&t),
            Event::Close(_) | Event::Eof => break,
            _ => {}
        }
    }
}

/// Append `<scenarios>`.
pub(crate) fn write_scenarios(x: &mut String, sheet: &Sheet, locale: &Locale) {
    if sheet.scenarios.is_empty() {
        return;
    }
    x.push_str("<scenarios current=\"0\" show=\"0\">");
    for scenario in &sheet.scenarios {
        x.push_str("<scenario name=\"");
        push_attr(x, &scenario.name);
        x.push('"');
        if scenario.locked {
            x.push_str(" locked=\"1\"");
        }
        if scenario.hidden {
            x.push_str(" hidden=\"1\"");
        }
        w!(x, " count=\"{}\"", scenario.cells.len());
        for (name, text) in [("user", &scenario.user), ("comment", &scenario.comment)] {
            if !text.is_empty() {
                w!(x, " {name}=\"");
                push_attr(x, text);
                x.push('"');
            }
        }
        x.push('>');
        for cell in &scenario.cells {
            w!(x, "<inputCells r=\"{}\" val=\"", cell_name(cell.row, cell.col));
            // Numbers are written the way files write them.
            match parse_numeric_text(&cell.value, locale) {
                Some(n) if n.is_finite() => push_number(x, n),
                _ => push_attr(x, &cell.value),
            }
            x.push_str("\"/>");
        }
        x.push_str("</scenario>");
    }
    x.push_str("</scenarios>");
}

/// The `<f t="dataTable">` element of a cell, when it is the first result
/// cell of a data table.
pub(crate) fn table_formula(sheet: &Sheet, row: u32, col: u32) -> Option<String> {
    let table = sheet.data_tables.iter().find(|table| (table.range.r0, table.range.c0) == (row, col))?;
    let mut f = String::from("<f t=\"dataTable\" ref=\"");
    push_range(&mut f, table.range);
    f.push('"');
    let name = |(row, col): (u32, u32)| cell_name(row, col);
    match (table.row_input, table.col_input) {
        (Some(row_input), Some(col_input)) => {
            w!(f, " dt2D=\"1\" dtr=\"1\" r1=\"{}\" r2=\"{}\"", name(row_input), name(col_input));
        }
        (Some(row_input), None) => {
            w!(f, " dtr=\"1\" r1=\"{}\"", name(row_input));
        }
        (None, Some(col_input)) => {
            w!(f, " r1=\"{}\"", name(col_input));
        }
        (None, None) => return None,
    }
    f.push_str("/>");
    Some(f)
}

/// Read the `<f t="dataTable">` of a cell: `ref` is the range of the
/// results, `r1` and `r2` the input cells (row then column for a table
/// with both, else the one `dtr` says). A table whose input cell was
/// deleted (`del1`, `del2`) stays the constants the file caches.
pub(crate) fn read_table(f: &Tag, sheet: &mut Sheet) {
    let cell = |name: &str| f.attr(name).and_then(|text| parse_cell_ref(&text));
    let Some(range) = f.attr("ref").and_then(|text| Rect::parse(&text)) else {
        return;
    };
    if f.attr_bool("del1").unwrap_or(false) || f.attr_bool("del2").unwrap_or(false) {
        return;
    }
    let (row_input, col_input) = match (f.attr_bool("dt2D").unwrap_or(false), f.attr_bool("dtr").unwrap_or(false)) {
        (true, _) => (cell("r1"), cell("r2")),
        (false, true) => (cell("r1"), None),
        (false, false) => (None, cell("r1")),
    };
    let inputs = [row_input, col_input];
    let usable = range.r0 > 0
        && range.c0 > 0
        && inputs.iter().any(Option::is_some)
        && !inputs.iter().flatten().any(|(row, col)| range.contains(*row, *col))
        && !sheet.data_tables.iter().any(|table| table.range.intersects(&range));
    if usable {
        sheet.data_tables.push(DataTable {
            range,
            row_input,
            col_input,
        });
    }
}
