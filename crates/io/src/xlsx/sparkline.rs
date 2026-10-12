//! Sparklines: Excel keeps them in the extension list of the worksheet,
//!
//! ```xml
//! <ext xmlns:x14="…/spreadsheetml/2009/9/main" uri="{05C60535-1F16-4fd2-B633-F4F36F0B64E0}">
//!   <x14:sparklineGroups xmlns:xm="http://schemas.microsoft.com/office/excel/2006/main">
//!     <x14:sparklineGroup type="column" displayEmptyCellsAs="gap" negative="1">
//!       <x14:colorSeries rgb="FF376092"/> … <x14:colorLow rgb="FFD00000"/>
//!       <x14:sparklines>
//!         <x14:sparkline><xm:f>Sheet1!A1:D1</xm:f><xm:sqref>F1</xm:sqref></x14:sparkline>
//!       </x14:sparklines>
//!     </x14:sparklineGroup>
//!   </x14:sparklineGroups>
//! </ext>
//! ```
//!
//! with the attributes and the eight colours in the order Excel writes
//! them. A date axis (`dateAxis`, `<xm:f>` of the group) is not kept: such
//! a group is read with its general axis.

use super::color::{Palette, RawColor};
use super::write::push_range;
use super::xml::{Event, Reader, Tag, push_text};
use opensheet_engine::ast::write_sheet_name;
use opensheet_engine::refs::parse_cell_name;
use opensheet_engine::sparkline::{AxisBound, DataRef, EmptyCells, Sparkline, SparklineGroup, SparklineType};
use opensheet_engine::workbook::Sheet;
use opensheet_engine::{Color, Rect};
use std::fmt::Write as _;

const EXT_URI: &str = "{05C60535-1F16-4fd2-B633-F4F36F0B64E0}";
const NS_X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
const NS_XM: &str = "http://schemas.microsoft.com/office/excel/2006/main";

/// A number as a file writes it: no exponent, no trailing zeros.
fn number(n: f64) -> String {
    let text = format!("{n}");
    if text.contains(['e', 'E']) { format!("{n:.15}").trim_end_matches('0').trim_end_matches('.').to_string() } else { text }
}

/// The `<ext>` element (a child of the worksheet's `<extLst>`) holding
/// the sparklines of a sheet; empty when it has none.
pub(crate) fn ext(sheet: &Sheet) -> String {
    let mut x = String::new();
    if sheet.sparklines.is_empty() {
        return x;
    }
    w!(x, "<ext xmlns:x14=\"{NS_X14}\" uri=\"{EXT_URI}\"><x14:sparklineGroups xmlns:xm=\"{NS_XM}\">");
    for group in &sheet.sparklines {
        x.push_str("<x14:sparklineGroup");
        if let AxisBound::Custom(v) = group.max {
            w!(x, " manualMax=\"{}\"", number(v));
        }
        if let AxisBound::Custom(v) = group.min {
            w!(x, " manualMin=\"{}\"", number(v));
        }
        if (group.line_weight - 0.75).abs() > 1e-9 {
            w!(x, " lineWeight=\"{}\"", number(group.line_weight));
        }
        match group.kind {
            SparklineType::Line => {}
            SparklineType::Column => x.push_str(" type=\"column\""),
            SparklineType::WinLoss => x.push_str(" type=\"stacked\""),
        }
        // The schema's default is "zero" and Excel's is gaps: it always
        // says which.
        x.push_str(match group.empty {
            EmptyCells::Gaps => " displayEmptyCellsAs=\"gap\"",
            EmptyCells::Zero => " displayEmptyCellsAs=\"zero\"",
            EmptyCells::Connect => " displayEmptyCellsAs=\"span\"",
        });
        let flags = [
            ("markers", group.markers),
            ("high", group.high),
            ("low", group.low),
            ("first", group.first),
            ("last", group.last),
            ("negative", group.negative),
            ("displayXAxis", group.show_axis),
            ("displayHidden", group.show_hidden),
        ];
        for (name, on) in flags {
            if on {
                w!(x, " {name}=\"1\"");
            }
        }
        for (name, bound) in [("minAxisType", group.min), ("maxAxisType", group.max)] {
            match bound {
                AxisBound::Individual => {}
                AxisBound::Group => {
                    w!(x, " {name}=\"group\"");
                }
                AxisBound::Custom(_) => {
                    w!(x, " {name}=\"custom\"");
                }
            }
        }
        if group.right_to_left {
            x.push_str(" rightToLeft=\"1\"");
        }
        x.push('>');
        let c = &group.colors;
        let colors = [
            ("colorSeries", c.series),
            ("colorNegative", c.negative),
            ("colorAxis", c.axis),
            ("colorMarkers", c.markers),
            ("colorFirst", c.first),
            ("colorLast", c.last),
            ("colorHigh", c.high),
            ("colorLow", c.low),
        ];
        for (name, color) in colors {
            // A colour of the theme by its number and its tint, as Excel
            // writes it; any other by its value.
            super::dxf::push_color(&mut x, &format!("x14:{name}"), color);
        }
        x.push_str("<x14:sparklines>");
        for s in &group.sparklines {
            let mut formula = String::new();
            write_sheet_name(&s.data.sheet, &mut formula);
            formula.push('!');
            push_range(&mut formula, s.data.range);
            x.push_str("<x14:sparkline><xm:f>");
            push_text(&mut x, &formula);
            x.push_str("</xm:f><xm:sqref>");
            push_range(&mut x, Rect::cell(s.row, s.col));
            x.push_str("</xm:sqref></x14:sparkline>");
        }
        x.push_str("</x14:sparklines></x14:sparklineGroup>");
    }
    x.push_str("</x14:sparklineGroups></ext>");
    x
}

/// `Sheet1!A1:D1`, `'My sheet'!$A$1:$A$9`: the sheet and the cells.
fn parse_data(text: &str) -> Option<DataRef> {
    let (sheet, cells) = text.rsplit_once('!')?;
    let sheet = match sheet.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        Some(quoted) => quoted.replace("''", "'"),
        None => sheet.to_string(),
    };
    let range = Rect::parse(&cells.replace('$', ""))?;
    (!sheet.is_empty()).then_some(DataRef { sheet, range })
}

/// Read `<x14:sparklineGroups>` into the sheet.
pub(crate) fn read<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, sheet: &mut Sheet, palette: &Palette) {
    if tag.empty {
        return;
    }
    loop {
        match r.next() {
            Event::Open(t) if t.name == "sparklineGroup" => {
                let group = read_group(r, &t, palette);
                if !group.sparklines.is_empty() {
                    sheet.sparklines.push(group);
                }
            }
            Event::Open(t) => r.skip(&t),
            Event::Close(_) | Event::Eof => break,
            _ => {}
        }
    }
}

fn read_group<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, palette: &Palette) -> SparklineGroup {
    let kind = match tag.attr("type").as_deref() {
        Some("column") => SparklineType::Column,
        Some("stacked") => SparklineType::WinLoss,
        _ => SparklineType::Line,
    };
    let mut group = SparklineGroup::new(kind);
    let flag = |name: &str| tag.attr_bool(name).unwrap_or(false);
    group.line_weight = tag.attr_f64("lineWeight").filter(|w| *w > 0.0).unwrap_or(0.75);
    group.markers = flag("markers");
    group.high = flag("high");
    group.low = flag("low");
    group.first = flag("first");
    group.last = flag("last");
    group.negative = flag("negative");
    group.show_axis = flag("displayXAxis");
    group.show_hidden = flag("displayHidden");
    group.right_to_left = flag("rightToLeft");
    group.empty = match tag.attr("displayEmptyCellsAs").as_deref() {
        Some("gap") => EmptyCells::Gaps,
        Some("span") => EmptyCells::Connect,
        _ => EmptyCells::Zero,
    };
    let bound = |kind: &str, manual: &str| match tag.attr(kind).as_deref() {
        Some("group") => AxisBound::Group,
        Some("custom") => tag.attr_f64(manual).map_or(AxisBound::Individual, AxisBound::Custom),
        _ => AxisBound::Individual,
    };
    group.min = bound("minAxisType", "manualMin");
    group.max = bound("maxAxisType", "manualMax");
    if tag.empty {
        return group;
    }
    loop {
        match r.next() {
            Event::Open(t) => {
                let color: Option<&mut Color> = match t.name {
                    "colorSeries" => Some(&mut group.colors.series),
                    "colorNegative" => Some(&mut group.colors.negative),
                    "colorAxis" => Some(&mut group.colors.axis),
                    "colorMarkers" => Some(&mut group.colors.markers),
                    "colorFirst" => Some(&mut group.colors.first),
                    "colorLast" => Some(&mut group.colors.last),
                    "colorHigh" => Some(&mut group.colors.high),
                    "colorLow" => Some(&mut group.colors.low),
                    _ => None,
                };
                match color {
                    Some(slot) => {
                        if let Some(read) = RawColor::from_tag(&t).resolve(palette) {
                            *slot = read;
                        }
                        r.skip(&t);
                    }
                    None if t.name == "sparklines" && !t.empty => read_sparklines(r, &mut group),
                    None => r.skip(&t),
                }
            }
            Event::Close(_) | Event::Eof => break,
            _ => {}
        }
    }
    group
}

fn read_sparklines(r: &mut Reader, group: &mut SparklineGroup) {
    loop {
        match r.next() {
            Event::Open(t) if t.name == "sparkline" && !t.empty => {
                let (mut data, mut cell) = (None, None);
                loop {
                    match r.next() {
                        Event::Open(inner) => match inner.name {
                            "f" => data = parse_data(&r.text(&inner)),
                            "sqref" => cell = parse_cell_name(r.text(&inner).trim()),
                            _ => r.skip(&inner),
                        },
                        Event::Close(_) | Event::Eof => break,
                        _ => {}
                    }
                }
                // A sparkline without data (Excel keeps the cell) draws nothing.
                if let (Some(data), Some((row, col))) = (data, cell) {
                    group.sparklines.push(Sparkline { row, col, data });
                }
            }
            Event::Open(t) => r.skip(&t),
            Event::Close(_) | Event::Eof => break,
            _ => {}
        }
    }
}
