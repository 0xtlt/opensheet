//! What the data tools keep in a worksheet part: `<dataConsolidate>`, the
//! settings of Data > Consolidate; `<scenarios>`, the scenarios of the
//! Scenario Manager; and the data tables of What-If Analysis, which are a
//! formula of their own kind (`<f t="dataTable">`) on their first cell.
//!
//! And the text files a workbook imports, which a package spreads over
//! three places: a connection of `xl/connections.xml` says which file and
//! how it is cut (`<textPr>`), a query table part related to the sheet
//! names the connection, and a defined name of the sheet, called like the
//! query table, gives the cells the data fills.

use super::parts::{Extras, SheetLinks};
use super::read::{Package, parse_cell_ref, relationships};
use super::write::{NS_DOC_REL, NS_MAIN, push_number, push_range};
use super::xml::{Event, Reader, Tag, XML_DECL, push_attr, push_cell_text};
use opensheet_engine::external::{QueryItem, QueryProps, WebQuery};
use opensheet_engine::consolidate::{ConsolidateFunction, ConsolidateSource, Consolidation};
use opensheet_engine::input::parse_numeric_text;
use opensheet_engine::refs::cell_name;
use opensheet_engine::text_columns::{ColumnFormat, DateOrder, Delimiters, Layout, Spec, TextImport};
use opensheet_engine::whatif::{DataTable, Scenario, ScenarioCell};
use opensheet_engine::{Content, Locale, Rect, Sheet, Workbook};
use std::fmt::Write;
use std::io::{Read, Seek};

const CT_CONNECTIONS: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.connections+xml";
const CT_QUERY_TABLE: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.queryTable+xml";

/// Read `<dataConsolidate>`: the function, the labels used and the
/// references. References to other workbooks (`r:id`) and to names are
/// left out.
pub(crate) fn read_consolidate<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, sheet: &mut Sheet) {
    let mut spec = Consolidation {
        function: tag
            .attr("function")
            .and_then(|name| ConsolidateFunction::from_ooxml_name(&name))
            .unwrap_or_default(),
        sources: Vec::new(),
        top_row: tag.attr_bool("topLabels").unwrap_or(false),
        left_column: tag.attr_bool("leftLabels").unwrap_or(false),
        link: tag.attr_bool("link").unwrap_or(false),
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
    if spec.link {
        x.push_str(" link=\"1\"");
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
/// deleted (`del1`, `del2`), or without the formula it shows, stays the
/// constants the file caches.
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
    // The formula the first result shows: in the corner of a table with
    // both inputs, else at the head of the column or of the row. Without
    // it the results are only what the file caches.
    let head = match (row_input, col_input) {
        (Some(_), Some(_)) => (range.r0.wrapping_sub(1), range.c0.wrapping_sub(1)),
        (None, _) => (range.r0.wrapping_sub(1), range.c0),
        (Some(_), None) => (range.r0, range.c0.wrapping_sub(1)),
    };
    let has_formula = sheet.cells.get(&head).is_some_and(|cell| matches!(cell.content, Content::Formula(_)));
    let usable = range.r0 > 0
        && range.c0 > 0
        && has_formula
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

// ---- text imports ----------------------------------------------------------

/// The `type` of a `<textField>`.
fn field_type(format: ColumnFormat) -> &'static str {
    match format {
        ColumnFormat::General => "general",
        ColumnFormat::Text => "text",
        ColumnFormat::Skip => "skip",
        ColumnFormat::Date(order) => order.letters(),
    }
}

fn field_format(name: &str) -> ColumnFormat {
    match name {
        "text" => ColumnFormat::Text,
        "skip" => ColumnFormat::Skip,
        other => DateOrder::ALL.into_iter().find(|order| order.letters() == other).map_or(ColumnFormat::General, ColumnFormat::Date),
    }
}

/// `<connection>` of a query, numbered `id`: a text import (type 6) or a
/// web query (type 4), with the properties of the connection in the
/// order Excel writes its attributes.
fn push_connection(x: &mut String, id: usize, import: &TextImport) {
    w!(x, "<connection id=\"{id}\" name=\"");
    push_attr(x, &import.name);
    x.push('"');
    if !import.props.description.is_empty() {
        x.push_str(" description=\"");
        push_attr(x, &import.props.description);
        x.push('"');
    }
    w!(x, " type=\"{}\" refreshedVersion=\"8\"", if import.web.is_some() { 4 } else { 6 });
    if import.props.background {
        x.push_str(" background=\"1\"");
    }
    if import.props.refresh_on_open {
        x.push_str(" refreshOnLoad=\"1\"");
    }
    if import.props.refresh_minutes > 0 {
        w!(x, " interval=\"{}\"", import.props.refresh_minutes);
    }
    x.push_str(" saveData=\"1\">");
    if let Some(web) = &import.web {
        // What Excel writes for a web query made in its dialog: the
        // data of the page, its preformatted blocks cut into columns.
        x.push_str("<webPr sourceData=\"1\" parsePre=\"1\" consecutive=\"1\" xl2000=\"1\" url=\"");
        push_attr(x, &import.path);
        match &web.item {
            None => x.push_str("\"/>"),
            Some(item) => {
                x.push_str("\" htmlTables=\"1\"><tables count=\"1\">");
                match item {
                    QueryItem::Index(n) => {
                        w!(x, "<x v=\"{n}\"/>");
                    }
                    QueryItem::Name(name) => {
                        x.push_str("<s v=\"");
                        push_attr(x, name);
                        x.push_str("\"/>");
                    }
                }
                x.push_str("</tables></webPr>");
            }
        }
        x.push_str("</connection>");
        return;
    }
    // Excel repairs a workbook whose text connection names no code page
    // or no fields; files are read as UTF-8 (65001).
    x.push_str("<textPr codePage=\"65001\" sourceFile=\"");
    push_attr(x, &import.path);
    x.push('"');
    if import.first_row > 1 {
        w!(x, " firstRow=\"{}\"", import.first_row);
    }
    // Only what differs from the defaults of the schema is said: tab
    // delimited, `"` as the qualifier.
    let mut positions = Vec::new();
    match &import.spec.layout {
        Layout::Delimited(d) => {
            if !d.tab {
                x.push_str(" tab=\"0\"");
            }
            for (name, on) in [("semicolon", d.semicolon), ("comma", d.comma), ("space", d.space), ("consecutive", d.consecutive_as_one)] {
                if on {
                    w!(x, " {name}=\"1\"");
                }
            }
            match d.qualifier {
                Some('"') => {}
                Some(_) => x.push_str(" qualifier=\"singleQuote\""),
                None => x.push_str(" qualifier=\"none\""),
            }
            if let Some(other) = d.other {
                x.push_str(" delimiter=\"");
                push_attr(x, &other.to_string());
                x.push('"');
            }
        }
        Layout::FixedWidth(breaks) => {
            x.push_str(" delimited=\"0\"");
            positions.push(0);
            positions.extend(breaks.iter().copied());
        }
    }
    // One field per column: of a fixed-width file (its position), of the
    // formats chosen, of the cells the import fills.
    let columns = (import.range.c1 - import.range.c0 + 1) as usize;
    let count = positions.len().max(import.spec.formats.len()).max(columns);
    w!(x, "><textFields count=\"{count}\">");
    for i in 0..count {
        x.push_str("<textField");
        let format = import.spec.format(i);
        if format != ColumnFormat::General {
            w!(x, " type=\"{}\"", field_type(format));
        }
        if let Some(position) = positions.get(i).filter(|position| **position > 0) {
            w!(x, " position=\"{position}\"");
        }
        x.push_str("/>");
    }
    x.push_str("</textFields></textPr></connection>");
}

/// The parts of the text imports: the connections of the workbook and a
/// query table for each import, related to its sheet.
pub(crate) fn add_parts(wb: &Workbook, extras: &mut Extras) {
    let mut connections = String::new();
    let mut number = 0;
    for (index, sheet) in wb.sheets.iter().enumerate() {
        let links = SheetLinks::of(sheet);
        for (import, id) in sheet.text_imports.iter().zip(&links.queries) {
            number += 1;
            push_connection(&mut connections, number, import);
            let mut table = format!("{XML_DECL}<queryTable xmlns=\"{NS_MAIN}\" name=\"");
            push_attr(&mut table, &import.name);
            table.push('"');
            if import.props.refresh_on_open {
                table.push_str(" refreshOnLoad=\"1\"");
            }
            if !import.props.preserve_formatting {
                table.push_str(" preserveFormatting=\"0\"");
            }
            if !import.props.adjust_width {
                table.push_str(" adjustColumnWidth=\"0\"");
            }
            w!(table, " connectionId=\"{number}\" autoFormatId=\"16\" applyNumberFormats=\"0\" applyBorderFormats=\"0\" applyFontFormats=\"0\" applyPatternFormats=\"0\" applyAlignmentFormats=\"0\" applyWidthHeightFormats=\"0\"/>");
            extras.add_part(format!("xl/queryTables/queryTable{number}.xml"), Some(CT_QUERY_TABLE), table);
            extras.relate_sheet(index, *id, &format!("{NS_DOC_REL}/queryTable"), &format!("../queryTables/queryTable{number}.xml"));
        }
    }
    if number > 0 {
        extras.add_part(
            "xl/connections.xml".to_string(),
            Some(CT_CONNECTIONS),
            format!("{XML_DECL}<connections xmlns=\"{NS_MAIN}\">{connections}</connections>"),
        );
        extras.relate_workbook(&format!("{NS_DOC_REL}/connections"), "connections.xml");
    }
}

/// Whether a sheet imports a text file, whose range a file names (see
/// [`push_names`]).
pub(crate) fn has_names(wb: &Workbook) -> bool {
    wb.sheets.iter().any(|sheet| !sheet.text_imports.is_empty())
}

/// Append the defined names that give the range of each text import
/// (children of `<definedNames>`).
pub(crate) fn push_names(x: &mut String, wb: &Workbook) {
    for (index, sheet) in wb.sheets.iter().enumerate() {
        for import in &sheet.text_imports {
            x.push_str("<definedName name=\"");
            push_attr(x, &import.name);
            w!(x, "\" localSheetId=\"{index}\">");
            let mut text = String::new();
            opensheet_engine::ast::write_sheet_name(&sheet.name, &mut text);
            text.push('!');
            let (first, last) = (cell_name(import.range.r0, import.range.c0), cell_name(import.range.r1, import.range.c1));
            for (n, name) in [first, last].iter().enumerate() {
                if n == 1 {
                    text.push(':');
                }
                let letters = name.trim_end_matches(|c: char| c.is_ascii_digit());
                w!(text, "${letters}${}", &name[letters.len()..]);
            }
            push_cell_text(x, &text);
            x.push_str("</definedName>");
        }
    }
}

/// The text connections of `xl/connections.xml`, by id. `range` and
/// `name` are those of the query table that uses one.
fn parse_connections(xml: &str) -> Vec<(u32, TextImport)> {
    let mut found = Vec::new();
    let mut r = Reader::new(xml);
    // The connection being read, until its `<textPr>` is seen.
    let mut id = None;
    let mut props = QueryProps::default();
    let mut current: Option<TextImport> = None;
    let mut breaks: Vec<usize> = Vec::new();
    loop {
        match r.next() {
            Event::Open(t) => match t.name {
                "connections" | "textFields" | "tables" => {}
                "connection" => {
                    id = t.attr_u32("id");
                    props = QueryProps {
                        description: t.attr("description").map(|text| text.into_owned()).unwrap_or_default(),
                        background: t.attr_bool("background").unwrap_or(false),
                        refresh_on_open: t.attr_bool("refreshOnLoad").unwrap_or(false),
                        refresh_minutes: t.attr_u32("interval").unwrap_or(0),
                        ..QueryProps::default()
                    };
                    if t.empty {
                        id = None;
                    }
                }
                "textPr" => {
                    let on = |name: &str, default: bool| t.attr_bool(name).unwrap_or(default);
                    let layout = if on("delimited", true) {
                        Layout::Delimited(Delimiters {
                            tab: on("tab", true),
                            semicolon: on("semicolon", false),
                            comma: on("comma", false),
                            space: on("space", false),
                            other: t.attr("delimiter").and_then(|text| text.chars().next()),
                            consecutive_as_one: on("consecutive", false),
                            qualifier: match t.attr("qualifier").as_deref() {
                                Some("singleQuote") => Some('\''),
                                Some("none") => None,
                                _ => Some('"'),
                            },
                        })
                    } else {
                        Layout::FixedWidth(Vec::new())
                    };
                    breaks.clear();
                    current = t.attr("sourceFile").map(|path| TextImport {
                        name: String::new(),
                        path: path.into_owned(),
                        range: Rect::cell(0, 0),
                        first_row: t.attr_u32("firstRow").unwrap_or(1).max(1),
                        spec: Spec::new(layout),
                        web: None,
                        props: props.clone(),
                    });
                    if t.empty {
                        if let (Some(id), Some(import)) = (id, current.take()) {
                            found.push((id, import));
                        }
                    }
                }
                "webPr" => {
                    current = t.attr("url").map(|url| TextImport {
                        name: String::new(),
                        path: url.into_owned(),
                        range: Rect::cell(0, 0),
                        first_row: 1,
                        spec: Spec::new(Layout::Delimited(Delimiters::default())),
                        web: Some(WebQuery::default()),
                        props: props.clone(),
                    });
                    if t.empty {
                        if let (Some(id), Some(import)) = (id, current.take()) {
                            found.push((id, import));
                        }
                    }
                }
                // The table of the page a web query takes: its number
                // (`x`) or its name (`s`). Only the first is kept.
                "x" | "s" => {
                    if let Some(web) = current.as_mut().and_then(|import| import.web.as_mut()).filter(|web| web.item.is_none()) {
                        web.item = match t.name {
                            "x" => t.attr_u32("v").map(QueryItem::Index),
                            _ => t.attr("v").map(|name| QueryItem::Name(name.into_owned())),
                        };
                    }
                    r.skip(&t);
                }
                "textField" => {
                    if let Some(import) = current.as_mut() {
                        import.spec.formats.push(field_format(t.attr("type").as_deref().unwrap_or("general")));
                        if let Some(position) = t.attr_u32("position").filter(|position| *position > 0) {
                            breaks.push(position as usize);
                        }
                    }
                    r.skip(&t);
                }
                _ => r.skip(&t),
            },
            Event::Close(name) if name == "textPr" => {
                if let (Some(id), Some(mut import)) = (id, current.take()) {
                    // A field that says nothing is a General column, which
                    // is also what a column without a format is.
                    while import.spec.formats.last() == Some(&ColumnFormat::General) {
                        import.spec.formats.pop();
                    }
                    if let Layout::FixedWidth(positions) = &mut import.spec.layout {
                        breaks.sort_unstable();
                        breaks.dedup();
                        *positions = std::mem::take(&mut breaks);
                    }
                    found.push((id, import));
                }
            }
            Event::Close(name) if name == "webPr" => {
                if let (Some(id), Some(import)) = (id, current.take()) {
                    found.push((id, import));
                }
            }
            Event::Close(name) if name == "connection" => id = None,
            Event::Eof => break,
            _ => {}
        }
    }
    found
}

/// The text imports of a file being read, until the defined names say
/// where the data of each is.
pub(crate) struct PendingImports {
    /// The sheet (its place in `sheet_parts`) and the import, its range
    /// still unknown.
    imports: Vec<(usize, TextImport, bool)>,
}

impl PendingImports {
    /// Read the connections of the workbook and the query tables of the
    /// sheets. `sheet_parts` names the worksheet part of each sheet, in
    /// order. Queries on anything but a text file are left out.
    pub fn read<R: Read + Seek>(pkg: &mut Package<R>, sheet_parts: &[String]) -> PendingImports {
        let mut pending = PendingImports { imports: Vec::new() };
        let Ok(Some(xml)) = pkg.read("xl/connections.xml") else {
            return pending;
        };
        let connections = parse_connections(&xml);
        if connections.is_empty() {
            return pending;
        }
        for (index, part) in sheet_parts.iter().enumerate() {
            for rel in relationships(pkg, part).into_iter().filter(|rel| rel.kind == "queryTable") {
                let Ok(Some(xml)) = pkg.read(&rel.target) else {
                    continue;
                };
                let mut r = Reader::new(&xml);
                let root = loop {
                    match r.next() {
                        Event::Open(t) if t.name == "queryTable" => break Some(t),
                        Event::Open(t) => r.skip(&t),
                        Event::Eof => break None,
                        _ => {}
                    }
                };
                let Some(root) = root else { continue };
                let connection = root.attr_u32("connectionId").and_then(|id| connections.iter().find(|(known, _)| *known == id));
                if let (Some((_, import)), Some(name)) = (connection, root.attr("name")) {
                    let mut import = import.clone();
                    import.name = name.into_owned();
                    import.props.preserve_formatting = root.attr_bool("preserveFormatting").unwrap_or(true);
                    import.props.adjust_width = root.attr_bool("adjustColumnWidth").unwrap_or(true);
                    pending.imports.push((index, import, false));
                }
            }
        }
        pending
    }

    /// Take a defined name of the workbook part if it is the range of a
    /// text import of `sheet` (`name` in upper case). Returns whether it
    /// was.
    pub fn read_name(&mut self, name: &str, text: &str, sheet: Option<usize>) -> bool {
        let Some(entry) = self
            .imports
            .iter_mut()
            .find(|(on, import, placed)| Some(*on) == sheet && !placed && import.name.to_uppercase() == name)
        else {
            return false;
        };
        let cells = text.rsplit('!').next().unwrap_or(text);
        if let Some(range) = Rect::parse(&cells.replace('$', "")) {
            entry.1.range = range;
            entry.2 = true;
        }
        true
    }

    /// Give the sheets the imports whose data was found.
    pub fn finish(self, wb: &mut Workbook) {
        for (sheet, import, placed) in self.imports {
            if let (true, Some(sheet)) = (placed, wb.sheets.get_mut(sheet)) {
                sheet.text_imports.push(import);
            }
        }
    }
}
