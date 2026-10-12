//! Writing a [`Workbook`] as an `.xlsx` package.

use super::chart::write::{CT_CHART, CT_DRAWING, drawings};
use super::defaults::{self, Defaults};
use super::dxf::Dxfs;
use super::formula::{is_valid_name, needs_array_evaluation, to_file_text};
use super::numfmt::{FIRST_CUSTOM_ID, builtin_id};
use super::parts::{Extras, SheetLinks, push_relationship};
use super::xml::{XML_DECL, push_attr, push_cell_text, push_text};
use super::{condfmt, page, protection, px_to_col_width, px_to_points, validation};
use crate::Result;
use opensheet_engine::names::NameScope;
use opensheet_engine::refs::{MAX_COLS, MAX_ROWS, Rect};
use opensheet_engine::style::{
    Border, BorderStyle, Borders, Font, HAlign, Style, StyleId, Underline, VAlign,
};
use opensheet_engine::value::{ErrorKind, Value};
use opensheet_engine::workbook::{Cell, Content, Sheet};
use opensheet_engine::{CalcMode, Color, IterativeCalc, Workbook};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::io::{Seek, Write};
use std::sync::Arc;

pub(crate) const NS_MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
pub(crate) const NS_DOC_REL: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
pub(crate) const NS_PKG_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const CT_SHEET: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml";

/// Widest column Excel accepts: 255 characters, here of a font whose
/// digits are 7 pixels wide (see `Defaults::max_col_width`).
pub(crate) const MAX_COL_WIDTH_PX: f32 = 255.0 * 7.0 + 5.0;
/// Tallest row Excel accepts, in points.
pub(crate) const MAX_ROW_HEIGHT_PT: f64 = 409.5;

fn push_rgb(x: &mut String, element: &str, color: Color) {
    w!(x, "<{element} rgb=\"FF{:06X}\"/>", color.0 & 0xFF_FFFF);
}

/// Append "A1" for a 0-based (row, col).
fn push_cell_ref(x: &mut String, row: u32, col: u32) {
    let mut letters = [0u8; 4];
    let mut i = letters.len();
    let mut n = col + 1;
    while n > 0 && i > 0 {
        i -= 1;
        letters[i] = b'A' + ((n - 1) % 26) as u8;
        n = (n - 1) / 26;
    }
    for b in &letters[i..] {
        x.push(*b as char);
    }
    w!(x, "{}", row + 1);
}

/// Append "A1" or "A1:B2" (always with explicit corners).
pub(crate) fn push_range(x: &mut String, rect: Rect) {
    push_cell_ref(x, rect.r0, rect.c0);
    if !rect.is_single() {
        x.push(':');
        push_cell_ref(x, rect.r1, rect.c1);
    }
}

/// Append a number in the shortest form that reads back identically.
pub(crate) fn push_number(x: &mut String, n: f64) {
    if n == n.trunc() && n.abs() < 1e15 {
        w!(x, "{}", n as i64);
    } else {
        w!(x, "{n:?}");
    }
}

// ---- styles ------------------------------------------------------------------

struct StyleBook {
    /// `styles.xml` in three pieces. Between the first two go the custom
    /// number formats and between the last two the differential formats:
    /// both lists are only complete once everything that names a format
    /// is written.
    head: String,
    body: String,
    tail: String,
    /// `cellXfs` index of every engine style (0 for styles not in use).
    xf: Vec<u32>,
    /// The custom number formats, in the order of their ids.
    formats: Vec<String>,
}

fn push_font(x: &mut String, f: &Font) {
    x.push_str("<font>");
    if f.bold {
        x.push_str("<b/>");
    }
    if f.italic {
        x.push_str("<i/>");
    }
    if f.strike {
        x.push_str("<strike/>");
    }
    match f.underline {
        Underline::None => {}
        Underline::Single => x.push_str("<u/>"),
        Underline::Double => x.push_str("<u val=\"double\"/>"),
    }
    w!(x, "<sz val=\"{}\"/>", f.size.max(2) as f64 / 2.0);
    if let Some(color) = f.color {
        push_rgb(x, "color", color);
    }
    x.push_str("<name val=\"");
    push_attr(x, &f.name);
    x.push_str("\"/></font>");
}

fn push_border_side(x: &mut String, name: &str, b: &Border) {
    let style = match b.style {
        BorderStyle::None => {
            w!(x, "<{name}/>");
            return;
        }
        BorderStyle::Hair => "hair",
        BorderStyle::Thin => "thin",
        BorderStyle::Medium => "medium",
        BorderStyle::Thick => "thick",
        BorderStyle::Dashed => "dashed",
        BorderStyle::Dotted => "dotted",
        BorderStyle::Double => "double",
    };
    w!(x, "<{name} style=\"{style}\">");
    match b.color {
        Some(color) => push_rgb(x, "color", color),
        None => x.push_str("<color auto=\"1\"/>"),
    }
    w!(x, "</{name}>");
}

/// The attributes of an `<alignment>` element; empty when all are default.
fn alignment_attrs(s: &Style) -> String {
    let mut a = String::new();
    let horizontal = match s.h_align {
        HAlign::General => None,
        HAlign::Left => Some("left"),
        HAlign::Center => Some("center"),
        HAlign::Right => Some("right"),
        HAlign::Fill => Some("fill"),
        HAlign::Justify => Some("justify"),
        HAlign::CenterAcross => Some("centerContinuous"),
    };
    if let Some(h) = horizontal {
        w!(a, " horizontal=\"{h}\"");
    }
    match s.v_align {
        VAlign::Bottom => {}
        VAlign::Top => a.push_str(" vertical=\"top\""),
        VAlign::Center => a.push_str(" vertical=\"center\""),
    }
    // Files count counter-clockwise angles 1..=90 and clockwise ones as
    // 90 + angle.
    let rotation = s.rotation.clamp(-90, 90);
    if rotation != 0 {
        w!(
            a,
            " textRotation=\"{}\"",
            if rotation > 0 {
                rotation
            } else {
                90 - rotation
            }
        );
    }
    if s.wrap {
        a.push_str(" wrapText=\"1\"");
    }
    if s.indent > 0 {
        w!(a, " indent=\"{}\"", s.indent.min(250));
    }
    if s.shrink_to_fit {
        a.push_str(" shrinkToFit=\"1\"");
    }
    a
}

impl StyleBook {
    fn xf(&self, id: StyleId) -> u32 {
        self.xf.get(id.0 as usize).copied().unwrap_or(0)
    }

    fn build(wb: &Workbook) -> StyleBook {
        let mut used = vec![false; wb.styles.len().max(1)];
        used[0] = true;
        {
            let mut mark = |id: &StyleId| {
                if let Some(u) = used.get_mut(id.0 as usize) {
                    *u = true;
                }
            };
            for sheet in &wb.sheets {
                sheet.cells.values().for_each(|c| mark(&c.style));
                sheet.col_styles.values().for_each(&mut mark);
                sheet.row_styles.values().for_each(&mut mark);
            }
        }

        // Component tables. Fill 0 (none) and 1 (gray125) are reserved by
        // the format; border 0 is "no borders".
        let mut fonts: Vec<&Font> = Vec::new();
        let mut font_ids: HashMap<&Font, u32> = HashMap::new();
        let mut fills: Vec<Color> = Vec::new();
        let mut borders: Vec<Borders> = vec![Borders::default()];
        let mut formats: Vec<&str> = Vec::new();

        let mut xf_of = vec![0u32; used.len()];
        let mut xfs = String::new();
        let mut xf_count = 0u32;
        for (i, _) in used.iter().enumerate().filter(|(_, used)| **used) {
            let style = wb.styles.get(StyleId(i as u32));
            let next_font = fonts.len() as u32;
            let font = *font_ids.entry(&style.font).or_insert_with(|| {
                fonts.push(&style.font);
                next_font
            });
            let fill = match style.fill {
                None => 0,
                Some(color) => {
                    let pos = fills.iter().position(|c| *c == color).unwrap_or_else(|| {
                        fills.push(color);
                        fills.len() - 1
                    });
                    pos as u32 + 2
                }
            };
            let border = borders
                .iter()
                .position(|b| *b == style.borders)
                .unwrap_or_else(|| {
                    borders.push(style.borders);
                    borders.len() - 1
                }) as u32;
            let num_fmt = builtin_id(&style.num_fmt, wb.locale).unwrap_or_else(|| {
                let code: &str = &style.num_fmt;
                let pos = formats.iter().position(|c| *c == code).unwrap_or_else(|| {
                    formats.push(code);
                    formats.len() - 1
                });
                FIRST_CUSTOM_ID + pos as u32
            });

            xf_of[i] = xf_count;
            xf_count += 1;
            w!(
                xfs,
                "<xf numFmtId=\"{num_fmt}\" fontId=\"{font}\" fillId=\"{fill}\" borderId=\"{border}\" xfId=\"0\""
            );
            if num_fmt != 0 {
                xfs.push_str(" applyNumberFormat=\"1\"");
            }
            if font != 0 {
                xfs.push_str(" applyFont=\"1\"");
            }
            if fill != 0 {
                xfs.push_str(" applyFill=\"1\"");
            }
            if border != 0 {
                xfs.push_str(" applyBorder=\"1\"");
            }
            let alignment = alignment_attrs(style);
            if !alignment.is_empty() {
                xfs.push_str(" applyAlignment=\"1\"");
            }
            // Cells are locked and show their formula unless told otherwise.
            let protection = !style.locked || style.hidden;
            if protection {
                xfs.push_str(" applyProtection=\"1\"");
            }
            if alignment.is_empty() && !protection {
                xfs.push_str("/>");
            } else {
                xfs.push('>');
                if !alignment.is_empty() {
                    w!(xfs, "<alignment{alignment}/>");
                }
                if protection {
                    xfs.push_str("<protection");
                    if !style.locked {
                        xfs.push_str(" locked=\"0\"");
                    }
                    if style.hidden {
                        xfs.push_str(" hidden=\"1\"");
                    }
                    xfs.push_str("/>");
                }
                xfs.push_str("</xf>");
            }
        }

        let mut head = String::from(XML_DECL);
        w!(head, "<styleSheet xmlns=\"{NS_MAIN}\">");
        let mut x = String::new();
        w!(x, "<fonts count=\"{}\">", fonts.len());
        for font in &fonts {
            push_font(&mut x, font);
        }
        x.push_str("</fonts>");
        w!(x, "<fills count=\"{}\">", fills.len() + 2);
        x.push_str("<fill><patternFill patternType=\"none\"/></fill>");
        x.push_str("<fill><patternFill patternType=\"gray125\"/></fill>");
        for color in &fills {
            x.push_str("<fill><patternFill patternType=\"solid\">");
            push_rgb(&mut x, "fgColor", *color);
            x.push_str("<bgColor indexed=\"64\"/></patternFill></fill>");
        }
        x.push_str("</fills>");
        w!(x, "<borders count=\"{}\">", borders.len());
        for b in &borders {
            x.push_str("<border>");
            push_border_side(&mut x, "left", &b.left);
            push_border_side(&mut x, "right", &b.right);
            push_border_side(&mut x, "top", &b.top);
            push_border_side(&mut x, "bottom", &b.bottom);
            x.push_str("<diagonal/></border>");
        }
        x.push_str("</borders>");
        x.push_str("<cellStyleXfs count=\"1\"><xf numFmtId=\"0\" fontId=\"0\" fillId=\"0\" borderId=\"0\"/></cellStyleXfs>");
        w!(x, "<cellXfs count=\"{xf_count}\">{xfs}</cellXfs>");
        x.push_str("<cellStyles count=\"1\"><cellStyle name=\"Normal\" xfId=\"0\" builtinId=\"0\"/></cellStyles>");
        let tail = concat!(
            "<tableStyles count=\"0\" defaultTableStyle=\"TableStyleMedium9\" defaultPivotStyle=\"PivotStyleLight16\"/>",
            "</styleSheet>"
        );
        StyleBook {
            head,
            body: x,
            tail: tail.to_string(),
            xf: xf_of,
            formats: formats.iter().map(|code| code.to_string()).collect(),
        }
    }

    /// The registry of the differential formats of this workbook.
    fn dxfs(&self, wb: &Workbook) -> Dxfs {
        Dxfs::new(self.formats.clone(), wb.locale)
    }

    /// `styles.xml`, once every part that uses a differential format has
    /// asked for its id.
    fn xml(&self, dxfs: &mut Dxfs) -> String {
        // The differential formats first: writing them gives the last of
        // their number formats an id.
        let dxfs_xml = dxfs.xml();
        let mut x = self.head.clone();
        let formats = self
            .formats
            .iter()
            .map(String::as_str)
            .chain(dxfs.own_formats().iter().map(|code| &**code));
        let count = self.formats.len() + dxfs.own_formats().len();
        if count > 0 {
            w!(x, "<numFmts count=\"{count}\">");
            for (i, code) in formats.enumerate() {
                w!(
                    x,
                    "<numFmt numFmtId=\"{}\" formatCode=\"",
                    FIRST_CUSTOM_ID + i as u32
                );
                push_attr(&mut x, code);
                x.push_str("\"/>");
            }
            x.push_str("</numFmts>");
        }
        x.push_str(&self.body);
        x.push_str(&dxfs_xml);
        x.push_str(&self.tail);
        x
    }
}

// ---- shared strings ------------------------------------------------------------

#[derive(Default)]
struct SharedStrings {
    index: HashMap<Arc<str>, u32>,
    list: Vec<Arc<str>>,
    /// Number of cells referring to the table.
    refs: usize,
}

impl SharedStrings {
    fn id(&mut self, s: &Arc<str>) -> u32 {
        self.refs += 1;
        if let Some(id) = self.index.get(s) {
            return *id;
        }
        let id = self.list.len() as u32;
        self.list.push(s.clone());
        self.index.insert(s.clone(), id);
        id
    }

    fn xml(&self) -> String {
        let mut x = String::from(XML_DECL);
        w!(
            x,
            "<sst xmlns=\"{NS_MAIN}\" count=\"{}\" uniqueCount=\"{}\">",
            self.refs,
            self.list.len()
        );
        for s in &self.list {
            x.push_str("<si>");
            push_t(&mut x, s);
            x.push_str("</si>");
        }
        x.push_str("</sst>");
        x
    }
}

/// Append a `<t>` element, asking readers to keep significant whitespace.
pub(crate) fn push_t(x: &mut String, s: &str) {
    let preserve = s.starts_with(char::is_whitespace)
        || s.ends_with(char::is_whitespace)
        || s.contains(['\n', '\t', '\r'])
        || s.contains("  ");
    x.push_str(if preserve {
        "<t xml:space=\"preserve\">"
    } else {
        "<t>"
    });
    push_cell_text(x, s);
    x.push_str("</t>");
}

// ---- worksheets ----------------------------------------------------------------

fn push_sheet_views(x: &mut String, sheet: &Sheet, selected: bool) {
    x.push_str("<sheetViews><sheetView");
    if !sheet.show_gridlines {
        x.push_str(" showGridLines=\"0\"");
    }
    if !sheet.show_headers {
        x.push_str(" showRowColHeaders=\"0\"");
    }
    if selected {
        x.push_str(" tabSelected=\"1\"");
    }
    x.push_str(" workbookViewId=\"0\"");
    let rows = sheet.frozen_rows.min(MAX_ROWS - 1);
    let cols = sheet.frozen_cols.min(MAX_COLS - 1);
    if rows == 0 && cols == 0 {
        x.push_str("/></sheetViews>");
        return;
    }
    x.push_str("><pane");
    if cols > 0 {
        w!(x, " xSplit=\"{cols}\"");
    }
    if rows > 0 {
        w!(x, " ySplit=\"{rows}\"");
    }
    x.push_str(" topLeftCell=\"");
    push_cell_ref(x, rows, cols);
    let active = match (rows > 0, cols > 0) {
        (true, true) => "bottomRight",
        (true, false) => "bottomLeft",
        _ => "topRight",
    };
    w!(x, "\" activePane=\"{active}\" state=\"frozen\"/>");
    if rows > 0 && cols > 0 {
        x.push_str("<selection pane=\"topRight\"/><selection pane=\"bottomLeft\"/>");
    }
    w!(x, "<selection pane=\"{active}\"/></sheetView></sheetViews>");
}

fn push_cols(x: &mut String, sheet: &Sheet, styles: &StyleBook, defaults: &Defaults) {
    #[derive(Clone, Copy, Default, PartialEq)]
    struct Col {
        width: Option<f32>,
        hidden: bool,
        xf: u32,
    }
    let mut cols: BTreeMap<u32, Col> = BTreeMap::new();
    for (c, width) in &sheet.col_widths {
        cols.entry(*c).or_default().width = Some(*width);
    }
    for c in &sheet.hidden_cols {
        cols.entry(*c).or_default().hidden = true;
    }
    for (c, style) in &sheet.col_styles {
        cols.entry(*c).or_default().xf = styles.xf(*style);
    }
    cols.retain(|c, col| *c < MAX_COLS && *col != Col::default());
    if cols.is_empty() {
        return;
    }
    x.push_str("<cols>");
    let mut iter = cols.into_iter().peekable();
    while let Some((first, col)) = iter.next() {
        // Neighbouring columns with the same properties share an element.
        let mut last = first;
        while iter
            .peek()
            .is_some_and(|(c, next)| *c == last + 1 && *next == col)
        {
            last += 1;
            iter.next();
        }
        let width = px_to_col_width(
            col.width
                .unwrap_or(sheet.default_col_width)
                .min(defaults.max_col_width()),
            defaults.digit_width,
        );
        w!(
            x,
            "<col min=\"{}\" max=\"{}\" width=\"{width}\"",
            first + 1,
            last + 1
        );
        if col.xf != 0 {
            w!(x, " style=\"{}\"", col.xf);
        }
        if col.hidden {
            x.push_str(" hidden=\"1\"");
        }
        if col.width.is_some() {
            x.push_str(" customWidth=\"1\"");
        }
        x.push_str("/>");
    }
    x.push_str("</cols>");
}

/// The cell type attribute (with leading space) a calculated value needs;
/// numbers need none.
fn value_type(value: &Value) -> &'static str {
    match value {
        Value::Text(_) => " t=\"str\"",
        Value::Bool(_) => " t=\"b\"",
        Value::Error(_) => " t=\"e\"",
        Value::Number(n) if !n.is_finite() => " t=\"e\"",
        _ => "",
    }
}

/// Append `<v>…</v>` for a calculated value (nothing for an empty one).
fn push_v(x: &mut String, value: &Value) {
    match value {
        Value::Empty | Value::Array(_) => {}
        Value::Number(n) if !n.is_finite() => {
            w!(x, "<v>{}</v>", ErrorKind::Num.as_str());
        }
        Value::Number(n) => {
            x.push_str("<v>");
            push_number(x, *n);
            x.push_str("</v>");
        }
        Value::Text(s) => {
            x.push_str("<v>");
            push_cell_text(x, s);
            x.push_str("</v>");
        }
        Value::Bool(b) => x.push_str(if *b { "<v>1</v>" } else { "<v>0</v>" }),
        Value::Error(e) => {
            x.push_str("<v>");
            push_text(x, e.as_str());
            x.push_str("</v>");
        }
    }
}

struct SheetWriter<'a> {
    wb: &'a Workbook,
    styles: &'a StyleBook,
    /// The sizes that follow from the workbook's default font.
    defaults: Defaults,
    strings: &'a mut SharedStrings,
    dxfs: &'a mut Dxfs,
    /// Set when a dynamic array formula was written (the package then needs
    /// the metadata part that marks such formulas).
    dynamic_arrays: bool,
}

impl SheetWriter<'_> {
    /// A constant or a spilled result.
    fn push_constant(&mut self, x: &mut String, row: u32, col: u32, xf: u32, value: &Value) {
        x.push_str("<c r=\"");
        push_cell_ref(x, row, col);
        x.push('"');
        if xf != 0 {
            w!(x, " s=\"{xf}\"");
        }
        match value {
            Value::Empty | Value::Array(_) => x.push_str("/>"),
            Value::Text(s) => {
                w!(x, " t=\"s\"><v>{}</v></c>", self.strings.id(s));
            }
            other => {
                x.push_str(value_type(other));
                x.push('>');
                push_v(x, other);
                x.push_str("</c>");
            }
        }
    }

    fn push_formula(
        &mut self,
        x: &mut String,
        (row, col): (u32, u32),
        xf: u32,
        cell: &Cell,
        formula: &str,
        array: Option<Rect>,
    ) {
        x.push_str("<c r=\"");
        push_cell_ref(x, row, col);
        x.push('"');
        if xf != 0 {
            w!(x, " s=\"{xf}\"");
        }
        x.push_str(value_type(&cell.value));
        match array {
            Some(range) => {
                // `cm="1"` points at the metadata record that makes this a
                // dynamic (spilling) array rather than a legacy one.
                self.dynamic_arrays = true;
                x.push_str(" cm=\"1\"><f t=\"array\" ref=\"");
                push_range(x, range);
                x.push_str("\">");
            }
            None => x.push_str("><f>"),
        }
        push_text(x, formula);
        x.push_str("</f>");
        push_v(x, &cell.value);
        x.push_str("</c>");
    }

    fn push_sheet_data(&mut self, x: &mut String, sheet: &Sheet) {
        // Spilled results are written as cached values, so applications that
        // do not recalculate on load still show them.
        let mut spilled: BTreeMap<(u32, u32), &Value> = BTreeMap::new();
        let mut anchors: HashMap<(u32, u32), Rect> = HashMap::new();
        for sp in &sheet.spills {
            let anchor = (sp.row, sp.col);
            let is_formula = matches!(
                sheet.cells.get(&anchor),
                Some(Cell {
                    content: Content::Formula(_),
                    ..
                })
            );
            if !is_formula
                || sp.array.rows == 0
                || sp.array.cols == 0
                || sp.row >= MAX_ROWS
                || sp.col >= MAX_COLS
            {
                continue;
            }
            let r1 = (sp.row as u64 + sp.array.rows as u64 - 1).min(MAX_ROWS as u64 - 1) as u32;
            let c1 = (sp.col as u64 + sp.array.cols as u64 - 1).min(MAX_COLS as u64 - 1) as u32;
            anchors.insert(anchor, Rect::new(sp.row, sp.col, r1, c1));
            for r in sp.row..=r1 {
                for c in sp.col..=c1 {
                    let value = sp.array.get((r - sp.row) as usize, (c - sp.col) as usize);
                    let occupied = sheet
                        .cells
                        .get(&(r, c))
                        .is_some_and(|cell| !matches!(cell.content, Content::Empty));
                    if (r, c) != anchor && !value.is_empty() && !occupied {
                        spilled.insert((r, c), value);
                    }
                }
            }
        }
        // Rows without cells still need an element when they carry a
        // height, a style or are hidden.
        let bare_rows: BTreeSet<u32> = sheet
            .row_heights
            .keys()
            .chain(sheet.hidden_rows.iter())
            .chain(sheet.filtered_rows.iter())
            .chain(sheet.row_styles.keys())
            .copied()
            .filter(|r| *r < MAX_ROWS)
            .collect();

        let mut cells = sheet.cells.iter().peekable();
        let mut spills = spilled.iter().peekable();
        let mut bare = bare_rows.iter().copied().peekable();
        x.push_str("<sheetData>");
        loop {
            let next = [
                cells.peek().map(|((r, _), _)| *r),
                spills.peek().map(|((r, _), _)| *r),
                bare.peek().copied(),
            ];
            let Some(row) = next.into_iter().flatten().min() else {
                break;
            };
            if bare.peek() == Some(&row) {
                bare.next();
            }
            w!(x, "<row r=\"{}\"", row + 1);
            if let Some(xf) = sheet
                .row_styles
                .get(&row)
                .map(|s| self.styles.xf(*s))
                .filter(|xf| *xf != 0)
            {
                w!(x, " s=\"{xf}\" customFormat=\"1\"");
            }
            if let Some(height) = sheet.row_heights.get(&row) {
                w!(
                    x,
                    " ht=\"{}\" customHeight=\"1\"",
                    px_to_points(*height).min(MAX_ROW_HEIGHT_PT)
                );
            }
            // Hidden by hand or by a filter: a file does not say which.
            if sheet.hidden_rows.contains(&row) || sheet.filtered_rows.contains(&row) {
                x.push_str(" hidden=\"1\"");
            }
            x.push('>');
            loop {
                let cell_col = cells
                    .peek()
                    .filter(|((r, _), _)| *r == row)
                    .map(|((_, c), _)| *c);
                let spill_col = spills
                    .peek()
                    .filter(|((r, _), _)| *r == row)
                    .map(|((_, c), _)| *c);
                let Some(col) = [cell_col, spill_col].into_iter().flatten().min() else {
                    break;
                };
                let cell = if cell_col == Some(col) {
                    cells.next().map(|(_, c)| c)
                } else {
                    None
                };
                let spill = if spill_col == Some(col) {
                    spills.next().map(|(_, v)| *v)
                } else {
                    None
                };
                let xf = match cell {
                    Some(cell) => self.styles.xf(cell.style),
                    None => self.styles.xf(sheet.inherited_style(row, col)),
                };
                match (cell, spill) {
                    (Some(cell), spill) => match &cell.content {
                        Content::Formula(expr) => {
                            // Besides formulas that spill, those whose single
                            // result comes from array arithmetic (or could
                            // not spill) are array formulas to Excel.
                            let array = anchors.get(&(row, col)).copied().or_else(|| {
                                let blocked = matches!(cell.value, Value::Error(ErrorKind::Spill));
                                (blocked || needs_array_evaluation(expr, self.wb))
                                    .then(|| Rect::cell(row, col))
                            });
                            self.push_formula(
                                x,
                                (row, col),
                                xf,
                                cell,
                                &to_file_text(expr, self.wb),
                                array,
                            );
                        }
                        Content::Value(value) => self.push_constant(x, row, col, xf, value),
                        // A formatted but empty cell may sit under a spill.
                        Content::Empty => {
                            self.push_constant(x, row, col, xf, spill.unwrap_or(&Value::Empty))
                        }
                    },
                    (None, Some(value)) => self.push_constant(x, row, col, xf, value),
                    (None, None) => {}
                }
            }
            x.push_str("</row>");
        }
        x.push_str("</sheetData>");
    }

    fn sheet_xml(&mut self, sheet: &Sheet, index: usize) -> String {
        // The first sheet is the one shown when the file is opened.
        let selected = index == 0;
        let mut x = String::with_capacity(512 + sheet.cells.len() * 40);
        x.push_str(XML_DECL);
        w!(
            x,
            "<worksheet xmlns=\"{NS_MAIN}\" xmlns:r=\"{NS_DOC_REL}\">"
        );
        // The children of a worksheet come in the order of the schema
        // (CT_Worksheet); Excel refuses a part that departs from it:
        //
        //   sheetPr, dimension, sheetViews, sheetFormatPr, cols, sheetData,
        //   sheetCalcPr, sheetProtection, protectedRanges, scenarios,
        //   autoFilter, sortState, dataConsolidate, customSheetViews,
        //   mergeCells, phoneticPr, conditionalFormatting, dataValidations,
        //   hyperlinks, printOptions, pageMargins, pageSetup, headerFooter,
        //   rowBreaks, colBreaks, customProperties, cellWatches,
        //   ignoredErrors, smartTags, drawing, legacyDrawing,
        //   legacyDrawingHF, drawingHF, picture, oleObjects, controls,
        //   webPublishItems, tableParts, extLst
        //
        // Each element written has its step below, named after it; a new
        // one goes between its neighbours of the list.

        // sheetPr: tabColor, then pageSetUpPr
        if sheet.tab_color.is_some() || page::needs_sheet_pr(&sheet.page) {
            x.push_str("<sheetPr>");
            if let Some(color) = sheet.tab_color {
                push_rgb(&mut x, "tabColor", color);
            }
            page::push_sheet_pr(&mut x, &sheet.page);
            x.push_str("</sheetPr>");
        }
        // dimension
        x.push_str("<dimension ref=\"");
        match sheet.used_rect() {
            Some(rect) => push_range(&mut x, rect),
            None => x.push_str("A1"),
        }
        x.push_str("\"/>");
        // sheetViews
        push_sheet_views(&mut x, sheet, selected);
        // sheetFormatPr
        defaults::write(&mut x, sheet, &self.defaults);
        // cols
        push_cols(&mut x, sheet, self.styles, &self.defaults);
        // sheetData
        self.push_sheet_data(&mut x, sheet);
        // sheetProtection
        if let Some(protection) = &sheet.protection {
            protection::write_sheet(&mut x, protection);
        }
        // autoFilter (the rows it hides are hidden rows of sheetData)
        super::filter::write_sheet(&mut x, self.wb, index, self.dxfs);
        // mergeCells
        let merges: Vec<&Rect> = sheet.merges.iter().filter(|m| !m.is_single()).collect();
        if !merges.is_empty() {
            w!(x, "<mergeCells count=\"{}\">", merges.len());
            for m in merges {
                x.push_str("<mergeCell ref=\"");
                push_range(&mut x, *m);
                x.push_str("\"/>");
            }
            x.push_str("</mergeCells>");
        }
        // conditionalFormatting
        condfmt::write(&mut x, sheet, index, self.wb, self.dxfs);
        // dataValidations
        validation::write(&mut x, sheet, self.wb);
        // printOptions, pageMargins, pageSetup, headerFooter, rowBreaks,
        // colBreaks
        page::write(&mut x, &sheet.page);
        // drawing, legacyDrawing, tableParts: what lives in parts of its
        // own (charts, pictures and shapes; the boxes of notes; tables).
        SheetLinks::of(sheet).push_elements(&mut x);
        // extLst: one `<ext>` per feature, in the order Excel writes them
        // (conditional formats, then data validations).
        let mut extensions = String::new();
        condfmt::write_ext(&mut extensions, sheet, index, self.wb, self.dxfs);
        validation::write_ext(&mut extensions, sheet, self.wb);
        if !extensions.is_empty() {
            w!(x, "<extLst>{extensions}</extLst>");
        }
        x.push_str("</worksheet>");
        x
    }
}

// ---- package -------------------------------------------------------------------

/// `extra` is what comes after the calculation properties: the elements
/// naming parts the workbook part relates to.
fn workbook_xml(wb: &Workbook, extra: &str) -> String {
    let mut x = String::from(XML_DECL);
    w!(x, "<workbook xmlns=\"{NS_MAIN}\" xmlns:r=\"{NS_DOC_REL}\">");
    // The children of the workbook, in the order of the schema
    // (CT_Workbook): fileVersion, fileSharing, workbookPr,
    // workbookProtection, bookViews, sheets, functionGroups,
    // externalReferences, definedNames, calcPr, oleSize,
    // customWorkbookViews, pivotCaches, ..., extLst.
    x.push_str("<workbookPr/>");
    protection::write_workbook(&mut x, &wb.protection);
    x.push_str("<bookViews><workbookView xWindow=\"0\" yWindow=\"0\" windowWidth=\"28800\" windowHeight=\"17600\"/></bookViews>");
    x.push_str("<sheets>");
    for (i, sheet) in wb.sheets.iter().enumerate() {
        x.push_str("<sheet name=\"");
        push_attr(&mut x, &sheet.name);
        w!(x, "\" sheetId=\"{0}\" r:id=\"rId{0}\"/>", i + 1);
    }
    x.push_str("</sheets>");
    // Names Excel would reject are left out rather than spoiling the file.
    // Sheet-scoped names say which sheet they belong to.
    let names: Vec<_> = wb
        .all_names()
        .into_iter()
        .filter(|(_, def)| is_valid_name(&def.name))
        .collect();
    if !names.is_empty() || page::has_names(wb) || super::filter::has_names(wb) {
        x.push_str("<definedNames>");
        // The ranges of the sheets' filters, which Excel keeps as hidden
        // names beside the filters themselves.
        super::filter::push_names(&mut x, wb);
        // The print areas and print titles of the sheets, which are names
        // only in files.
        page::push_names(&mut x, wb);
        for (scope, def) in names {
            x.push_str("<definedName name=\"");
            push_attr(&mut x, &def.name);
            x.push('"');
            if !def.comment.is_empty() {
                x.push_str(" comment=\"");
                push_attr(&mut x, &def.comment);
                x.push('"');
            }
            if let NameScope::Sheet(sheet) = scope {
                w!(x, " localSheetId=\"{sheet}\"");
            }
            x.push('>');
            push_text(&mut x, &to_file_text(&def.expr, wb));
            x.push_str("</definedName>");
        }
        x.push_str("</definedNames>");
    }
    // Cached results are written, but the reader is asked to recalculate:
    // its functions are the reference. The calculation options are written
    // where they are not Excel's defaults.
    x.push_str("<calcPr calcId=\"191029\"");
    match wb.calc_mode() {
        CalcMode::Automatic => {}
        CalcMode::AutomaticExceptTables => x.push_str(" calcMode=\"autoNoTable\""),
        CalcMode::Manual => x.push_str(" calcMode=\"manual\""),
    }
    let iterative = wb.iterative_calc();
    let usual = IterativeCalc::default();
    if iterative.enabled {
        x.push_str(" iterate=\"1\"");
    }
    if iterative.max_iterations != usual.max_iterations {
        w!(x, " iterateCount=\"{}\"", iterative.max_iterations);
    }
    if iterative.max_change != usual.max_change {
        x.push_str(" iterateDelta=\"");
        push_number(&mut x, iterative.max_change);
        x.push('"');
    }
    x.push_str(" fullCalcOnLoad=\"1\"/>");
    x.push_str(extra);
    x.push_str("</workbook>");
    x
}

/// Marks formulas flagged with `cm="1"` as dynamic arrays.
const METADATA_XML: &str = concat!(
    "<metadata xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" ",
    "xmlns:xda=\"http://schemas.microsoft.com/office/spreadsheetml/2017/dynamicarray\">",
    "<metadataTypes count=\"1\"><metadataType name=\"XLDAPR\" minSupportedVersion=\"120000\" copy=\"1\" ",
    "pasteAll=\"1\" pasteValues=\"1\" merge=\"1\" splitFirst=\"1\" rowColShift=\"1\" clearFormats=\"1\" ",
    "clearComments=\"1\" assign=\"1\" coerce=\"1\" cellMeta=\"1\"/></metadataTypes>",
    "<futureMetadata name=\"XLDAPR\" count=\"1\"><bk><extLst>",
    "<ext uri=\"{bdbb8cdc-fa1e-496e-a857-3c3f30c029c3}\">",
    "<xda:dynamicArrayProperties fDynamic=\"1\" fCollapsed=\"0\"/></ext></extLst></bk></futureMetadata>",
    "<cellMetadata count=\"1\"><bk><rc t=\"1\" v=\"0\"/></bk></cellMetadata></metadata>",
);

pub(crate) fn write_workbook<W: Write + Seek>(
    wb: &Workbook,
    out: W,
    macro_enabled: bool,
) -> Result<W> {
    let styles = StyleBook::build(wb);
    let mut strings = SharedStrings::default();
    let mut dxfs = styles.dxfs(wb);
    let mut writer = SheetWriter {
        wb,
        styles: &styles,
        defaults: Defaults::of(&wb.styles.default_style().font),
        strings: &mut strings,
        dxfs: &mut dxfs,
        dynamic_arrays: false,
    };
    let sheets: Vec<String> = wb
        .sheets
        .iter()
        .enumerate()
        .map(|(i, sheet)| writer.sheet_xml(sheet, i))
        .collect();
    let dynamic_arrays = writer.dynamic_arrays;
    let sheet_count = sheets.len();
    let mut drawings = drawings(wb);
    let extras = Extras::of(wb, &mut drawings, &mut dxfs);

    let mut content_types = String::from(XML_DECL);
    content_types
        .push_str("<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">");
    content_types.push_str("<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>");
    content_types.push_str("<Default Extension=\"xml\" ContentType=\"application/xml\"/>");
    for (extension, content_type) in &extras.defaults {
        w!(
            content_types,
            "<Default Extension=\"{extension}\" ContentType=\"{content_type}\"/>"
        );
    }
    let mut override_part = |part: &str, content_type: &str| {
        w!(
            content_types,
            "<Override PartName=\"{part}\" ContentType=\"{content_type}\"/>"
        );
    };
    if macro_enabled {
        override_part(
            "/xl/workbook.xml",
            "application/vnd.ms-excel.sheet.macroEnabled.main+xml",
        );
    } else {
        override_part("/xl/workbook.xml", &format!("{CT_SHEET}.sheet.main+xml"));
    }
    for i in 1..=sheet_count {
        override_part(
            &format!("/xl/worksheets/sheet{i}.xml"),
            &format!("{CT_SHEET}.worksheet+xml"),
        );
    }
    override_part("/xl/styles.xml", &format!("{CT_SHEET}.styles+xml"));
    override_part(
        "/xl/sharedStrings.xml",
        &format!("{CT_SHEET}.sharedStrings+xml"),
    );
    if dynamic_arrays {
        override_part("/xl/metadata.xml", &format!("{CT_SHEET}.sheetMetadata+xml"));
    }
    for drawing in &drawings {
        override_part(
            &format!("/xl/drawings/drawing{}.xml", drawing.number),
            CT_DRAWING,
        );
        for (number, _) in &drawing.charts {
            override_part(&format!("/xl/charts/chart{number}.xml"), CT_CHART);
        }
    }
    for part in &extras.parts {
        if let Some(content_type) = part.content_type {
            override_part(&format!("/{}", part.name), content_type);
        }
    }
    override_part(
        "/docProps/core.xml",
        "application/vnd.openxmlformats-package.core-properties+xml",
    );
    override_part(
        "/docProps/app.xml",
        "application/vnd.openxmlformats-officedocument.extended-properties+xml",
    );
    content_types.push_str("</Types>");

    let mut root_rels = String::from(XML_DECL);
    w!(root_rels, "<Relationships xmlns=\"{NS_PKG_REL}\">");
    w!(
        root_rels,
        "<Relationship Id=\"rId1\" Type=\"{NS_DOC_REL}/officeDocument\" Target=\"xl/workbook.xml\"/>"
    );
    w!(
        root_rels,
        "<Relationship Id=\"rId2\" Type=\"{NS_PKG_REL}/metadata/core-properties\" Target=\"docProps/core.xml\"/>"
    );
    w!(
        root_rels,
        "<Relationship Id=\"rId3\" Type=\"{NS_DOC_REL}/extended-properties\" Target=\"docProps/app.xml\"/>"
    );
    root_rels.push_str("</Relationships>");

    let mut workbook_rels = String::from(XML_DECL);
    w!(workbook_rels, "<Relationships xmlns=\"{NS_PKG_REL}\">");
    let mut next_id = 1;
    let mut relate = |kind: &str, target: &str| {
        let kind = format!("{NS_DOC_REL}/{kind}");
        push_relationship(&mut workbook_rels, next_id, &kind, target);
        next_id += 1;
    };
    // Sheets first: `workbook.xml` refers to sheet n as rId n.
    for i in 1..=sheet_count {
        relate("worksheet", &format!("worksheets/sheet{i}.xml"));
    }
    relate("styles", "styles.xml");
    relate("sharedStrings", "sharedStrings.xml");
    if dynamic_arrays {
        relate("sheetMetadata", "metadata.xml");
    }
    // The parts of the features are numbered after these; the workbook
    // part names some of them by those ids.
    let first_extra = next_id;
    workbook_rels.push_str(&extras.workbook_relationships(first_extra));
    workbook_rels.push_str("</Relationships>");

    let app = format!(
        "{XML_DECL}<Properties xmlns=\"http://schemas.openxmlformats.org/officeDocument/2006/extended-properties\"><Application>OpenSheet</Application></Properties>"
    );
    let core = format!(
        "{XML_DECL}<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\"><dc:creator>OpenSheet</dc:creator></cp:coreProperties>"
    );

    let mut zip = zip::ZipWriter::new(out);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    let mut put = |name: &str, data: &str| -> Result<()> {
        zip.start_file(name, options)?;
        zip.write_all(data.as_bytes())?;
        Ok(())
    };
    put("[Content_Types].xml", &content_types)?;
    put("_rels/.rels", &root_rels)?;
    put("docProps/app.xml", &app)?;
    put("docProps/core.xml", &core)?;
    put(
        "xl/workbook.xml",
        &workbook_xml(wb, &extras.workbook_elements(first_extra)),
    )?;
    put("xl/_rels/workbook.xml.rels", &workbook_rels)?;
    for (i, xml) in sheets.iter().enumerate() {
        put(&format!("xl/worksheets/sheet{}.xml", i + 1), xml)?;
    }
    put("xl/styles.xml", &styles.xml(&mut dxfs))?;
    put("xl/sharedStrings.xml", &strings.xml())?;
    if dynamic_arrays {
        put("xl/metadata.xml", &format!("{XML_DECL}{METADATA_XML}"))?;
    }
    for drawing in &drawings {
        let n = drawing.number;
        put(&format!("xl/drawings/drawing{n}.xml"), &drawing.xml)?;
        put(
            &format!("xl/drawings/_rels/drawing{n}.xml.rels"),
            &drawing.rels,
        )?;
        for (number, xml) in &drawing.charts {
            put(&format!("xl/charts/chart{number}.xml"), xml)?;
        }
    }
    // The relationships of the worksheets are among them.
    for part in &extras.parts {
        zip.start_file(part.name.as_str(), options)?;
        zip.write_all(&part.data)?;
    }
    Ok(zip.finish()?)
}
