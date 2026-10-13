//! Whether two workbooks are the same workbook, as far as a user can
//! tell: what the recorder checks a recorded action against, and what the
//! tests of the recorder compare a replay with.
//!
//! Two things are left out on purpose. The results of formulas, which a
//! volatile function changes from one calculation to the next: the
//! formulas themselves are compared. And what a script cannot say about a
//! format while it shows the same: a colour of the theme against the same
//! colour, the automatic colour against black, the name of a cell style.

use opensheet_engine::chart::Chart;
use opensheet_engine::refs::cell_name;
use opensheet_engine::style::{Border, Color, Style, StyleId};
use opensheet_engine::table::Table;
use opensheet_engine::{Content, Sheet, Value, Workbook};
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// Sizes are kept in pixels and scripts give them in points.
const SIZE_TOLERANCE: f32 = 0.02;
/// The corner of a chart is a cell and an offset in it, worked out again
/// from points by a script.
const PLACE_TOLERANCE: f64 = 0.5;

/// What was found different, counted in full and described up to a limit.
struct Found {
    count: usize,
    limit: usize,
    texts: Vec<String>,
}

impl Found {
    fn add(&mut self, text: impl FnOnce() -> String) {
        self.count += 1;
        if self.texts.len() < self.limit {
            self.texts.push(text());
        }
    }
}

/// A format as it shows: without what only says where it came from.
pub(crate) fn shown(style: &Style) -> Style {
    let flat = |color: Color| Color(color.value());
    let line = |border: &mut Border| {
        border.color = if border.is_none() { None } else { Some(flat(border.color.unwrap_or(Color::BLACK))) };
    };
    let mut out = style.clone();
    out.font.theme = None;
    out.font.color = Some(flat(out.font.color.unwrap_or(Color::BLACK)));
    out.fill = out.fill.map(flat);
    line(&mut out.borders.left);
    line(&mut out.borders.right);
    line(&mut out.borders.top);
    line(&mut out.borders.bottom);
    out.cell_style = None;
    out
}

/// Compares the formats of two workbooks by their numbers, remembering
/// the pairs already compared: sheets hold few formats and many cells.
pub(crate) struct Formats<'a> {
    a: &'a Workbook,
    b: &'a Workbook,
    known: HashMap<(StyleId, StyleId), bool>,
}

impl<'a> Formats<'a> {
    pub fn new(a: &'a Workbook, b: &'a Workbook) -> Formats<'a> {
        Formats {
            a,
            b,
            known: HashMap::new(),
        }
    }

    pub fn same(&mut self, a: StyleId, b: StyleId) -> bool {
        let (wa, wb) = (self.a, self.b);
        *self.known.entry((a, b)).or_insert_with(|| {
            let (sa, sb) = (wa.styles.get(a), wb.styles.get(b));
            sa == sb || shown(sa) == shown(sb)
        })
    }
}

/// What a cell holds: nothing for a cell that is not there.
pub(crate) fn content_at(sheet: &Sheet, pos: (u32, u32)) -> &Content {
    const EMPTY: &Content = &Content::Empty;
    match sheet.cells.get(&pos).map(|cell| &cell.content) {
        Some(Content::Value(Value::Empty)) | None => EMPTY,
        Some(content) => content,
    }
}

fn same_size(a: Option<&f32>, b: Option<&f32>, standard_a: f32, standard_b: f32) -> bool {
    (a.copied().unwrap_or(standard_a) - b.copied().unwrap_or(standard_b)).abs() <= SIZE_TOLERANCE
}

/// The keys of two maps, each once.
fn keys<'a, K: Ord + Copy + 'a, V: 'a>(a: &'a BTreeMap<K, V>, b: &'a BTreeMap<K, V>) -> BTreeSet<K> {
    a.keys().chain(b.keys()).copied().collect()
}

/// Where the corner of a chart is, in pixels from the corner of its sheet.
fn corner(sheet: &Sheet, chart: &Chart) -> (f64, f64) {
    let x: f64 = (0..chart.anchor.1).map(|col| sheet.col_width(col) as f64).sum();
    let y: f64 = (0..chart.anchor.0).map(|row| sheet.row_height(row) as f64).sum();
    (x + chart.offset.0 as f64, y + chart.offset.1 as f64)
}

/// Two charts are the same when all but their number is, their place and
/// size within what the conversion from points loses.
pub(crate) fn same_chart(sheet_a: &Sheet, a: &Chart, sheet_b: &Sheet, b: &Chart) -> bool {
    let (place_a, place_b) = (corner(sheet_a, a), corner(sheet_b, b));
    let near = |x: f64, y: f64| (x - y).abs() <= PLACE_TOLERANCE;
    if !near(place_a.0, place_b.0) || !near(place_a.1, place_b.1) || !near(a.size.0 as f64, b.size.0 as f64) || !near(a.size.1 as f64, b.size.1 as f64) {
        return false;
    }
    let mut a = a.clone();
    a.id = b.id;
    a.anchor = b.anchor;
    a.offset = b.offset;
    a.size = b.size;
    a == *b
}

/// A table without its number, which only says in which order tables
/// were made.
pub(crate) fn same_table(a: &Table, b: &Table) -> bool {
    let mut a = a.clone();
    a.id = b.id;
    a == *b
}

fn sheet_differences(formats: &mut Formats, a: &Sheet, b: &Sheet, found: &mut Found) {
    let name = &b.name;
    // Cells: what they hold and how they are formatted.
    let positions: BTreeSet<(u32, u32)> = a.cells.keys().chain(b.cells.keys()).copied().collect();
    for pos in positions {
        if content_at(a, pos) != content_at(b, pos) {
            found.add(|| format!("{name}!{}: {:?} / {:?}", cell_name(pos.0, pos.1), content_at(a, pos), content_at(b, pos)));
        }
        let (style_a, style_b) = (a.style_id_at(pos.0, pos.1), b.style_id_at(pos.0, pos.1));
        if !formats.same(style_a, style_b) {
            let (wa, wb) = (formats.a, formats.b);
            found.add(|| format!("{name}!{} format: {:?} / {:?}", cell_name(pos.0, pos.1), shown(wa.styles.get(style_a)), shown(wb.styles.get(style_b))));
        }
    }
    for col in keys(&a.col_styles, &b.col_styles) {
        let get = |sheet: &Sheet| sheet.col_styles.get(&col).copied().unwrap_or_default();
        if !formats.same(get(a), get(b)) {
            found.add(|| format!("{name}: format of column {col}"));
        }
    }
    for row in keys(&a.row_styles, &b.row_styles) {
        let get = |sheet: &Sheet| sheet.row_styles.get(&row).copied().unwrap_or_default();
        if !formats.same(get(a), get(b)) {
            found.add(|| format!("{name}: format of row {row}"));
        }
    }
    // Sizes.
    for col in keys(&a.col_widths, &b.col_widths) {
        if !same_size(a.col_widths.get(&col), b.col_widths.get(&col), a.default_col_width, b.default_col_width) {
            found.add(|| format!("{name}: width of column {col}: {:?} / {:?}", a.col_widths.get(&col), b.col_widths.get(&col)));
        }
    }
    for row in keys(&a.row_heights, &b.row_heights) {
        if !same_size(a.row_heights.get(&row), b.row_heights.get(&row), a.default_row_height, b.default_row_height) {
            found.add(|| format!("{name}: height of row {row}: {:?} / {:?}", a.row_heights.get(&row), b.row_heights.get(&row)));
        }
    }
    let mut plain = |what: &str, same: bool, a: &dyn std::fmt::Debug, b: &dyn std::fmt::Debug| {
        if !same {
            found.add(|| {
                let (mut a, mut b) = (format!("{a:?}"), format!("{b:?}"));
                a.truncate(300);
                b.truncate(300);
                format!("{name}: {what}: {a} / {b}")
            });
        }
    };
    plain("standard width", (a.default_col_width - b.default_col_width).abs() <= SIZE_TOLERANCE, &a.default_col_width, &b.default_col_width);
    plain("standard height", (a.default_row_height - b.default_row_height).abs() <= SIZE_TOLERANCE, &a.default_row_height, &b.default_row_height);
    plain("hidden columns", a.hidden_cols == b.hidden_cols, &a.hidden_cols, &b.hidden_cols);
    plain("hidden rows", a.hidden_rows == b.hidden_rows, &a.hidden_rows, &b.hidden_rows);
    plain("filtered rows", a.filtered_rows == b.filtered_rows, &a.filtered_rows, &b.filtered_rows);
    let merges = |sheet: &Sheet| sheet.merges.iter().map(|m| (m.r0, m.c0, m.r1, m.c1)).collect::<BTreeSet<_>>();
    plain("merged cells", merges(a) == merges(b), &a.merges, &b.merges);
    plain("frozen panes", (a.frozen_rows, a.frozen_cols) == (b.frozen_rows, b.frozen_cols), &(a.frozen_rows, a.frozen_cols), &(b.frozen_rows, b.frozen_cols));
    plain("gridlines", a.show_gridlines == b.show_gridlines, &a.show_gridlines, &b.show_gridlines);
    plain("headings", a.show_headers == b.show_headers, &a.show_headers, &b.show_headers);
    let tab = |sheet: &Sheet| sheet.tab_color.map(|color| color.value());
    plain("tab colour", tab(a) == tab(b), &tab(a), &tab(b));
    // The arrow a sort leaves on a button of the sheet's filter is all a
    // script cannot draw: sorting the range sorts the same rows.
    let filter = |sheet: &Sheet| {
        sheet.auto_filter.clone().map(|mut filter| {
            filter.sort = None;
            filter
        })
    };
    plain("filter", filter(a) == filter(b), &a.auto_filter, &b.auto_filter);
    plain("names", a.names == b.names, &a.names, &b.names);
    // Tables, whatever the order they were made in.
    let tables = |sheet: &Sheet| {
        let mut tables: Vec<Table> = sheet.tables.clone();
        tables.sort_by_key(|table| (table.range.r0, table.range.c0));
        tables
    };
    let (tables_a, tables_b) = (tables(a), tables(b));
    let same_tables = tables_a.len() == tables_b.len() && tables_a.iter().zip(&tables_b).all(|(a, b)| same_table(a, b));
    plain("tables", same_tables, &tables_a, &tables_b);
    let same_charts = a.charts.len() == b.charts.len() && a.charts.iter().zip(&b.charts).all(|(ca, cb)| same_chart(a, ca, b, cb));
    plain("charts", same_charts, &a.charts, &b.charts);
    // What no script changes: it must simply not have moved.
    macro_rules! untouched {
        ($($field:ident),*) => {
            $(
                if format!("{:?}", a.$field) != format!("{:?}", b.$field) {
                    found.add(|| format!("{name}: {}", stringify!($field)));
                }
            )*
        };
    }
    untouched!(
        advanced_filter,
        consolidation,
        scenarios,
        data_tables,
        text_imports,
        cond_formats,
        validations,
        comments,
        protection,
        page,
        pivots,
        drawings,
        slicers,
        timelines,
        inks,
        custom_views,
        hyperlinks,
        outline,
        sparklines,
        background,
        code_name
    );
}

fn find(a: &Workbook, b: &Workbook, limit: usize) -> Found {
    let mut found = Found {
        count: 0,
        limit,
        texts: Vec::new(),
    };
    let names = |wb: &Workbook| wb.sheets.iter().map(|sheet| sheet.name.clone()).collect::<Vec<_>>();
    if names(a) != names(b) {
        // Sheets that do not pair up: each counts for all it holds.
        found.count += 1 + a.sheets.iter().chain(&b.sheets).map(|sheet| sheet.cells.len()).sum::<usize>();
        found.texts.push(format!("sheets: {:?} / {:?}", names(a), names(b)));
        return found;
    }
    let mut formats = Formats::new(a, b);
    for (sheet_a, sheet_b) in a.sheets.iter().zip(&b.sheets) {
        sheet_differences(&mut formats, sheet_a, sheet_b, &mut found);
    }
    if a.names != b.names {
        found.add(|| format!("names: {:?} / {:?}", a.names.keys(), b.names.keys()));
    }
    if format!("{:?}", a.cell_styles) != format!("{:?}", b.cell_styles) {
        found.add(|| "cell styles".to_string());
    }
    if format!("{:?}", a.protection) != format!("{:?}", b.protection) {
        found.add(|| "workbook protection".to_string());
    }
    found
}

/// What differs between two workbooks, in words (the first few): empty
/// when they are the same.
pub(crate) fn differences(a: &Workbook, b: &Workbook) -> Vec<String> {
    find(a, b, 12).texts
}

/// How much differs between two workbooks: 0 when they are the same.
pub(crate) fn distance(a: &Workbook, b: &Workbook) -> usize {
    find(a, b, 0).count
}
