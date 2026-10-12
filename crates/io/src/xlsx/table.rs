//! Tables (`Sheet::tables`): one `xl/tables/tableN.xml` part each, named
//! by the worksheet in `<tableParts>`.
//!
//! The cells of a table are cells of the sheet like any others; the part
//! says where the table is, what it and its columns are called, what its
//! totals row computes, how it looks and what its filter shows.
//!
//! Kept: name, range, header and totals rows, the columns with their
//! totals function, label and formula, the formula of calculated columns,
//! the style and its four options, the filter with its criteria and sort.
//! Not kept: formats of whole columns (`dataDxfId` and the like), query
//! tables, XML maps, the comment of a table, custom table styles (the
//! name stays, and draws nothing).

use super::dxf::Dxfs;
use super::filter::{
    DxfColors, area, parse_auto_filter, parse_sort, push_auto_filter, push_text_attr,
};
use super::formula::{from_file_text, to_file_text};
use super::parts::{Extras, SheetLinks};
use super::read::{Package, relationships};
use super::write::{NS_DOC_REL, NS_MAIN};
use super::xml::{Event, Reader, XML_DECL, decode_cell_text, push_text};
use opensheet_engine::filter::FilterScope;
use opensheet_engine::table::{Table, TableColumn, TotalsFunction};
use opensheet_engine::{Rect, Workbook};
use std::fmt::Write as _;
use std::io::{Read, Seek};
use std::sync::Arc;

const CT_TABLE: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.table+xml";

// ---- writing -------------------------------------------------------------

/// The part of one table of sheet `sheet`.
fn table_xml(wb: &Workbook, sheet: usize, table: &Table, dxfs: &mut Dxfs) -> String {
    let mut x = String::from(XML_DECL);
    w!(x, "<table xmlns=\"{NS_MAIN}\" id=\"{}\"", table.id);
    // Formulas go by `displayName`; `name` is the same in files of Excel.
    push_text_attr(&mut x, "name", &table.name);
    push_text_attr(&mut x, "displayName", &table.name);
    w!(x, " ref=\"{}\"", area(table.range));
    if !table.header_row {
        x.push_str(" headerRowCount=\"0\"");
    }
    x.push_str(if table.totals_row {
        " totalsRowCount=\"1\""
    } else {
        " totalsRowShown=\"0\""
    });
    x.push('>');
    // The filter, and after it the sort made through it: in a table the
    // two are neighbours.
    push_auto_filter(&mut x, wb, sheet, FilterScope::Table(table.id), dxfs);
    w!(x, "<tableColumns count=\"{}\">", table.columns.len());
    for (index, column) in table.columns.iter().enumerate() {
        w!(x, "<tableColumn id=\"{}\"", index + 1);
        push_text_attr(&mut x, "name", &column.name);
        if let Some(label) = &column.totals_label {
            push_text_attr(&mut x, "totalsRowLabel", label);
        }
        if column.totals != TotalsFunction::None {
            w!(x, " totalsRowFunction=\"{}\"", column.totals.ooxml_name());
        }
        let calculated = wb.calculated_column_formula(sheet, table.id, index);
        let custom = matches!(column.totals, TotalsFunction::Custom(_))
            .then(|| wb.totals_formula(sheet, table.id, index))
            .flatten();
        if calculated.is_none() && custom.is_none() {
            x.push_str("/>");
            continue;
        }
        x.push('>');
        for (element, formula) in [
            ("calculatedColumnFormula", calculated),
            ("totalsRowFormula", custom),
        ] {
            if let Some(formula) = formula {
                w!(x, "<{element}>");
                push_text(&mut x, &to_file_text(&formula, wb));
                w!(x, "</{element}>");
            }
        }
        x.push_str("</tableColumn>");
    }
    x.push_str("</tableColumns><tableStyleInfo");
    if let Some(style) = &table.style {
        push_text_attr(&mut x, "name", style);
    }
    let flag = |on: bool| u8::from(on);
    w!(
        x,
        " showFirstColumn=\"{}\" showLastColumn=\"{}\" showRowStripes=\"{}\" showColumnStripes=\"{}\"/></table>",
        flag(table.first_col),
        flag(table.last_col),
        flag(table.banded_rows),
        flag(table.banded_cols)
    );
    x
}

/// What the tables of a workbook add to its package. The colours their
/// filters and sorts go by are added to `dxfs`.
pub(crate) fn add_parts(wb: &Workbook, extras: &mut Extras, dxfs: &mut Dxfs) {
    // Parts are numbered through the workbook, in the order of the sheets.
    let mut number = 0;
    for (index, sheet) in wb.sheets.iter().enumerate() {
        let links = SheetLinks::of(sheet);
        for (table, id) in sheet.tables.iter().zip(&links.tables) {
            number += 1;
            extras.add_part(
                format!("xl/tables/table{number}.xml"),
                Some(CT_TABLE),
                table_xml(wb, index, table, dxfs),
            );
            extras.relate_sheet(
                index,
                *id,
                &format!("{NS_DOC_REL}/table"),
                &format!("../tables/table{number}.xml"),
            );
        }
    }
}

// ---- reading -------------------------------------------------------------

/// A table part. `None` when it does not say where the table is.
fn parse_table(xml: &str, colors: &[DxfColors]) -> Option<Table> {
    let mut r = Reader::new(xml);
    let root = loop {
        match r.next() {
            Event::Open(t) if t.name == "table" => break t,
            Event::Open(t) => r.skip(&t),
            Event::Eof => return None,
            _ => {}
        }
    };
    let text = |name: &str| root.attr(name).map(|v| decode_cell_text(&v).into_owned());
    let range = root.attr("ref").and_then(|name| Rect::parse(&name))?;
    let mut table = Table {
        id: root.attr_u32("id").unwrap_or(0),
        name: text("displayName")
            .or_else(|| text("name"))
            .unwrap_or_default(),
        range,
        header_row: root.attr_u32("headerRowCount").unwrap_or(1) > 0,
        totals_row: root.attr_u32("totalsRowCount").unwrap_or(0) > 0,
        ..Table::default()
    };
    let mut sort = None;
    if !root.empty {
        loop {
            match r.next() {
                Event::Open(t) => match t.name {
                    "autoFilter" => table.auto_filter = parse_auto_filter(&mut r, &t, colors),
                    "sortState" => sort = Some(parse_sort(&mut r, &t, range, colors)),
                    "tableColumns" if !t.empty => loop {
                        match r.next() {
                            Event::Open(c) if c.name == "tableColumn" => {
                                table.columns.push(parse_column(&mut r, &c));
                            }
                            Event::Open(c) => r.skip(&c),
                            Event::Close(_) | Event::Eof => break,
                            Event::Text(_) => {}
                        }
                    },
                    "tableStyleInfo" => {
                        table.style = t
                            .attr("name")
                            .map(|name| decode_cell_text(&name).into_owned())
                            .filter(|name| !name.is_empty());
                        let flag = |name: &str| t.attr_bool(name).unwrap_or(false);
                        table.first_col = flag("showFirstColumn");
                        table.last_col = flag("showLastColumn");
                        table.banded_rows = flag("showRowStripes");
                        table.banded_cols = flag("showColumnStripes");
                        r.skip(&t);
                    }
                    _ => r.skip(&t),
                },
                Event::Close(_) | Event::Eof => break,
                Event::Text(_) => {}
            }
        }
    }
    // A table without a header row has no buttons to filter with.
    if !table.header_row {
        table.auto_filter = None;
    }
    if let (Some(filter), Some(sort)) = (&mut table.auto_filter, sort)
        && filter.sort.is_none()
        && !sort.levels.is_empty()
    {
        filter.sort = Some(sort);
    }
    Some(table)
}

/// A `<tableColumn>`. The formula of a calculated column is not read: the
/// cells of the column hold it.
fn parse_column<'a>(r: &mut Reader<'a>, tag: &super::xml::Tag<'a>) -> TableColumn {
    let text = |name: &str| tag.attr(name).map(|v| decode_cell_text(&v).into_owned());
    let function = tag.attr("totalsRowFunction");
    let mut column = TableColumn {
        name: text("name").unwrap_or_default(),
        totals: function
            .as_deref()
            .and_then(TotalsFunction::from_ooxml_name)
            .unwrap_or_default(),
        totals_label: text("totalsRowLabel"),
    };
    if tag.empty {
        return column;
    }
    loop {
        match r.next() {
            Event::Open(t) => {
                if t.name == "totalsRowFormula" && function.as_deref() == Some("custom") {
                    // A formula the engine cannot read leaves the column
                    // without a total; the cell keeps its last result.
                    if let Some(formula) = from_file_text(&r.text(&t)) {
                        column.totals = TotalsFunction::Custom(Arc::new(formula));
                    }
                } else {
                    r.skip(&t);
                }
            }
            Event::Close(_) | Event::Eof => break,
            Event::Text(_) => {}
        }
    }
    column
}

/// Read the tables of every sheet into the workbook. `sheet_parts` names
/// the worksheet part of each sheet of `wb`, in order. A table part that
/// is missing, makes no sense or lies over another table is left out, and
/// costs nothing else. Afterwards the rows a table filter hides are known
/// as filtered rather than hidden by hand.
pub(crate) fn read_all<R: Read + Seek>(
    pkg: &mut Package<R>,
    wb: &mut Workbook,
    sheet_parts: &[String],
    colors: &[DxfColors],
) {
    for (index, part) in sheet_parts.iter().enumerate() {
        let mut rels = relationships(pkg, part);
        rels.retain(|rel| rel.kind == "table");
        // In the order of `<tableParts>`, which Excel numbers its
        // relationships in: when two tables want the same name or the same
        // cells, the first one has them.
        rels.sort_by_key(|rel| {
            let digits = rel.id.trim_start_matches(|c: char| !c.is_ascii_digit());
            digits.parse::<u32>().unwrap_or(u32::MAX)
        });
        for rel in rels {
            let Ok(Some(xml)) = pkg.read(&rel.target) else {
                continue;
            };
            if let Some(table) = parse_table(&xml, colors) {
                // Refused when it is off the sheet, has no data row or
                // overlaps a table read before it.
                let _ = wb.insert_table(index, table);
            }
        }
        // The rows the filters of the sheet hide, its own and those of
        // its tables, are hidden rows in the file.
        wb.adopt_filtered_rows(index);
    }
}
