//! Computing a PivotTable: from the cache and the layout to the grid of
//! values written into the sheet.
//!
//! The records that pass the filters are sorted into two trees, one per
//! axis: a node is an item of a field under the items of the fields before
//! it. Values are accumulated per (row node, column node) pair at the
//! leaves and rolled up to every ancestor pair, so that any subtotal or
//! grand total is one lookup away. The trees are then walked into *lines*
//! (the rows and columns of the report, with their subtotals, totals and
//! blank lines), and the grid is the product of the two lists of lines.

use super::text::{self, Caption};
use super::calc::CalcFormula;
use super::{
    Aggregation, AxisField, BaseItem, FieldLayout, FilterKind, LabelFilter, PivotTable, ShowAs, SortOrder, Subtotals,
    Top10By, ValueFilter,
};
use crate::eval::compare_text;
use crate::format;
use crate::functions::wildcard_match;
use crate::locale::Locale;
use crate::value::{ErrorKind, Value};
use crate::workbook::Workbook;
use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::Arc;

pub(crate) const NONE: u32 = u32::MAX;
const NO_DATA: u16 = u16::MAX;

/// Size of the frame Excel reserves for a PivotTable without any field.
pub(crate) const PLACEHOLDER: (u32, u32) = (18, 3);

/// Hasher for the integer keys of the trees and of the cube: the default
/// one costs more than the rest of the aggregation.
#[derive(Clone, Copy, Default)]
pub(crate) struct FastHasher(u64);

impl Hasher for FastHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.write_u64(*b as u64);
        }
    }

    fn write_u64(&mut self, n: u64) {
        let x = (self.0 ^ n).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        self.0 = x ^ (x >> 29);
    }
}

type FastMap<V> = HashMap<u64, V, BuildHasherDefault<FastHasher>>;

fn pair(a: u32, b: u32) -> u64 {
    ((a as u64) << 32) | b as u64
}

// ---- trees -----------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Node {
    pub parent: u32,
    /// Item of the node's field (index into the cache items).
    pub item: u32,
    /// 0 for the root, 1 for the items of the first field of the axis...
    pub depth: u8,
    /// In display order once the tree is sorted.
    pub children: Vec<u32>,
}

/// The items of the fields of one axis, nested as they are in the report.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Tree {
    pub nodes: Vec<Node>,
}

impl Default for Tree {
    fn default() -> Tree {
        Tree {
            nodes: vec![Node {
                parent: NONE,
                item: NONE,
                depth: 0,
                children: Vec::new(),
            }],
        }
    }
}

impl Tree {
    fn child(&mut self, index: &mut FastMap<u32>, parent: u32, item: u32) -> u32 {
        let next = self.nodes.len() as u32;
        let id = *index.entry(pair(parent, item)).or_insert(next);
        if id == next {
            let depth = self.nodes[parent as usize].depth + 1;
            self.nodes.push(Node {
                parent,
                item,
                depth,
                children: Vec::new(),
            });
            self.nodes[parent as usize].children.push(id);
        }
        id
    }

    /// The ancestor of `node` at `depth` (the node itself at its own depth);
    /// `NONE` when the node is above that depth.
    pub(crate) fn ancestor(&self, node: u32, depth: u8) -> u32 {
        let mut n = node;
        loop {
            let Some(x) = self.nodes.get(n as usize) else {
                return NONE;
            };
            match x.depth.cmp(&depth) {
                Ordering::Less => return NONE,
                Ordering::Equal => return n,
                Ordering::Greater => n = x.parent,
            }
        }
    }

    /// The child of `parent` standing for `item`.
    pub(crate) fn find_child(&self, parent: u32, item: u32) -> Option<u32> {
        let p = self.nodes.get(parent as usize)?;
        p.children
            .iter()
            .copied()
            .find(|c| self.nodes[*c as usize].item == item)
    }
}

// ---- accumulators ----------------------------------------------------------

/// Everything the eleven summary functions need about a set of values.
#[derive(Clone, Copy, Debug)]
struct Acc {
    /// Records.
    rows: u32,
    /// Non-empty values.
    count: u32,
    /// Numbers.
    n: u32,
    sum: f64,
    /// Running mean and sum of squared deviations (for the variances).
    mean: f64,
    m2: f64,
    min: f64,
    max: f64,
    product: f64,
    /// The first error value met.
    err: Option<ErrorKind>,
}

impl Acc {
    const EMPTY: Acc = Acc {
        rows: 0,
        count: 0,
        n: 0,
        sum: 0.0,
        mean: 0.0,
        m2: 0.0,
        min: f64::INFINITY,
        max: f64::NEG_INFINITY,
        product: 1.0,
        err: None,
    };

    fn add(&mut self, v: &Value) {
        self.rows += 1;
        match v {
            Value::Empty => {}
            Value::Number(x) => {
                self.count += 1;
                self.n += 1;
                self.sum += x;
                let delta = x - self.mean;
                self.mean += delta / self.n as f64;
                self.m2 += delta * (x - self.mean);
                self.min = self.min.min(*x);
                self.max = self.max.max(*x);
                self.product *= x;
            }
            Value::Error(e) => {
                self.count += 1;
                self.err.get_or_insert(*e);
            }
            _ => self.count += 1,
        }
    }

    fn merge(&mut self, o: &Acc) {
        self.rows += o.rows;
        self.count += o.count;
        if o.n > 0 {
            if self.n == 0 {
                self.mean = o.mean;
                self.m2 = o.m2;
            } else {
                let (a, b) = (self.n as f64, o.n as f64);
                let delta = o.mean - self.mean;
                self.mean = (a * self.mean + b * o.mean) / (a + b);
                self.m2 += o.m2 + delta * delta * a * b / (a + b);
            }
            self.n += o.n;
            self.sum += o.sum;
            self.min = self.min.min(o.min);
            self.max = self.max.max(o.max);
            self.product *= o.product;
        }
        if self.err.is_none() {
            self.err = o.err;
        }
    }

    /// The summary of the values; `None` when there is no record at all
    /// (the cell stays empty).
    fn result(&self, f: Aggregation) -> Option<Value> {
        if self.rows == 0 {
            return None;
        }
        let n = self.n as f64;
        let div0 = Value::Error(ErrorKind::Div0);
        Some(match f {
            Aggregation::Count => Value::Number(self.count as f64),
            Aggregation::CountNums => Value::Number(n),
            _ if self.err.is_some() => Value::Error(self.err.unwrap_or(ErrorKind::Value)),
            Aggregation::Sum => Value::Number(self.sum),
            Aggregation::Average if self.n == 0 => div0,
            Aggregation::Average => Value::Number(self.sum / n),
            Aggregation::Max => Value::Number(if self.n == 0 { 0.0 } else { self.max }),
            Aggregation::Min => Value::Number(if self.n == 0 { 0.0 } else { self.min }),
            Aggregation::Product => Value::Number(if self.n == 0 { 0.0 } else { self.product }),
            Aggregation::StdDev | Aggregation::Var if self.n < 2 => div0,
            Aggregation::StdDevp | Aggregation::Varp if self.n < 1 => div0,
            Aggregation::StdDev => Value::Number((self.m2 / (n - 1.0)).max(0.0).sqrt()),
            Aggregation::StdDevp => Value::Number((self.m2 / n).max(0.0).sqrt()),
            Aggregation::Var => Value::Number((self.m2 / (n - 1.0)).max(0.0)),
            Aggregation::Varp => Value::Number((self.m2 / n).max(0.0)),
        })
    }
}

// ---- report ----------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineKind {
    /// A header row or a label column: it carries no values.
    Header,
    /// An item at the innermost level shown (a detail row or column).
    Item,
    /// An item with detail below it, on a line of its own (outline and
    /// compact form).
    Group,
    Subtotal,
    Grand,
    /// The blank line after an item.
    Blank,
}

/// One row or one column of the report.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Line {
    pub kind: LineKind,
    /// The node of the axis tree the line stands for (0 = everything).
    pub node: u32,
    /// The value field, when "Σ Values" is on this axis.
    data: u16,
    /// Position in the axis of the field the line belongs to.
    pub level: u8,
    /// For a custom subtotal, 1 + the position of its function in the
    /// field's list.
    func: u8,
    /// Whether the line shows values.
    pub values: bool,
}

impl Line {
    pub(crate) fn data(&self) -> Option<usize> {
        (self.data != NO_DATA).then_some(self.data as usize)
    }

    /// True for the line of a custom subtotal function.
    pub(crate) fn is_custom(&self) -> bool {
        self.func > 0
    }

    const HEADER: Line = Line {
        kind: LineKind::Header,
        node: 0,
        data: NO_DATA,
        level: 0,
        func: 0,
        values: false,
    };

    /// A line of the given kind at an axis position, standing for nothing:
    /// what the sample report of a style thumbnail is made of.
    pub(crate) fn sample(kind: LineKind, level: u8) -> Line {
        Line {
            kind,
            level,
            values: kind != LineKind::Header,
            ..Line::HEADER
        }
    }
}

/// One position of an axis: a field, or "Σ Values".
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Level {
    /// `None` for "Σ Values".
    pub field: Option<usize>,
    /// Depth of the field's nodes in the axis tree.
    pub depth: u8,
    /// Row axis: the column of the labels. Column axis: the header row.
    pub slot: u32,
    /// Compact form: how deep the labels are indented in their column.
    pub indent: u8,
    /// Items with detail below them get a line of their own.
    own_line: bool,
    subtotal: Sub,
    blank: bool,
    repeat: bool,
    pub compact: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum Sub {
    None,
    /// On the item's own line, above its detail.
    Top,
    Bottom,
    Custom(Vec<Aggregation>),
}

/// What a cell of the report is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tag {
    Blank,
    /// The cell naming the value field (or blank when there are several).
    DataCaption,
    /// "Row Labels".
    RowCaption,
    /// "Column Labels".
    ColCaption,
    /// Name of the row field at this axis position (outline, tabular).
    RowHeader(u8),
    /// Name of the column field at this axis position.
    ColHeader(u8),
    /// Label of the item of the line at this axis position.
    RowLabel(u8),
    ColLabel(u8),
    /// "East Total".
    RowSubtotal(u8),
    ColSubtotal(u8),
    /// "Grand Total".
    RowGrand,
    ColGrand,
    /// Label of the only row of a report without row fields.
    TotalLabel,
    Value,
}

/// A filter field above the report.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PageCell {
    pub field: usize,
    /// Position of the label in the filter block; the value is to its right.
    pub row: u32,
    pub col: u32,
    pub value: Value,
}

/// The computed layout and values of a PivotTable, as last written into
/// the sheet.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Report {
    pub(crate) rows: u32,
    pub(crate) cols: u32,
    pub(crate) header_rows: u32,
    pub(crate) label_cols: u32,
    pub(crate) caption_rows: u32,
    pub(crate) values: Vec<Value>,
    pub(crate) tags: Vec<Tag>,
    pub(crate) row_lines: Vec<Line>,
    pub(crate) col_lines: Vec<Line>,
    pub(crate) row_levels: Vec<Level>,
    pub(crate) col_levels: Vec<Level>,
    pub(crate) row_tree: Tree,
    pub(crate) col_tree: Tree,
    /// Leaf of each record in the two trees; `NONE` for filtered records.
    pub(crate) rec_row: Vec<u32>,
    pub(crate) rec_col: Vec<u32>,
    pub(crate) pages: Vec<PageCell>,
    pub(crate) page_rows: u32,
    pub(crate) page_cols: u32,
    /// The PivotTable has no field: only its frame is reserved.
    pub(crate) placeholder: bool,
}

/// The owner of the number format of a cell of the report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum FormatOf {
    Nothing,
    /// The labels of a field.
    Field(usize),
    /// A value field.
    Data(usize),
    /// A value field summarised by a custom subtotal function: its "Show
    /// Values As" calculation, and the percent format, do not apply.
    Subtotal(usize),
}

/// What a row or a column of the report stands for (`rowItems`,
/// `colItems`).
#[derive(Clone, Debug, PartialEq)]
pub struct ReportLine {
    pub kind: ReportLineKind,
    /// (field, item) of the line, outermost first.
    pub items: Vec<(usize, u32)>,
    /// The value field of the line when "Σ Values" is on its axis.
    pub data_field: Option<usize>,
    /// The function of a custom subtotal.
    pub function: Option<Aggregation>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReportLineKind {
    Item,
    /// An item above its detail (outline and compact form).
    Group,
    Subtotal,
    GrandTotal,
    Blank,
}

impl Report {
    /// Size of the report without its filter block: (rows, columns).
    pub fn size(&self) -> (u32, u32) {
        (self.rows, self.cols)
    }

    /// Size of the filter block: (rows, columns).
    pub fn page_size(&self) -> (u32, u32) {
        (self.page_rows, self.page_cols)
    }

    /// Row of the first header row relative to the top of the report (1
    /// when the first row holds the captions of the column fields).
    pub fn first_header_row(&self) -> u32 {
        self.caption_rows
    }

    /// Number of rows above the first row of values.
    pub fn first_data_row(&self) -> u32 {
        self.header_rows
    }

    /// Number of columns left of the first column of values.
    pub fn first_data_col(&self) -> u32 {
        self.label_cols
    }

    pub(crate) fn at(&self, r: u32, c: u32) -> usize {
        (r * self.cols + c) as usize
    }

    /// The value shown at a cell of the report (relative coordinates).
    pub fn value(&self, r: u32, c: u32) -> &Value {
        static EMPTY: Value = Value::Empty;
        if r < self.rows && c < self.cols {
            self.values.get(self.at(r, c)).unwrap_or(&EMPTY)
        } else {
            &EMPTY
        }
    }

    pub(crate) fn tag(&self, r: u32, c: u32) -> Tag {
        if r < self.rows && c < self.cols {
            self.tags.get(self.at(r, c)).copied().unwrap_or(Tag::Blank)
        } else {
            Tag::Blank
        }
    }

    /// (field, item) of a node of an axis tree, outermost first.
    pub(crate) fn path(&self, rows: bool, node: u32, p: &PivotTable) -> Vec<(usize, u32)> {
        let (tree, axis) = if rows {
            (&self.row_tree, &p.row_fields)
        } else {
            (&self.col_tree, &p.col_fields)
        };
        let fields: Vec<usize> = axis.iter().filter_map(|a| a.field()).collect();
        let mut out = Vec::new();
        let mut n = node;
        while let Some(x) = tree.nodes.get(n as usize) {
            if x.depth == 0 {
                break;
            }
            if let Some(f) = fields.get(x.depth as usize - 1) {
                out.push((*f, x.item));
            }
            n = x.parent;
        }
        out.reverse();
        out
    }

    /// The rows (or the columns) of values, in order.
    pub fn lines(&self, rows: bool, p: &PivotTable) -> Vec<ReportLine> {
        let (lines, levels) = if rows {
            (&self.row_lines, &self.row_levels)
        } else {
            (&self.col_lines, &self.col_levels)
        };
        lines
            .iter()
            .filter(|l| l.kind != LineKind::Header)
            .map(|l| ReportLine {
                kind: match l.kind {
                    LineKind::Group => ReportLineKind::Group,
                    LineKind::Subtotal => ReportLineKind::Subtotal,
                    LineKind::Grand => ReportLineKind::GrandTotal,
                    LineKind::Blank => ReportLineKind::Blank,
                    _ => ReportLineKind::Item,
                },
                items: self.path(rows, l.node, p),
                data_field: l.data(),
                function: match (l.func, levels.get(l.level as usize).map(|v| &v.subtotal)) {
                    (k, Some(Sub::Custom(fs))) if k > 0 => fs.get(k as usize - 1).copied(),
                    _ => None,
                },
            })
            .collect()
    }

    /// The value field of a cell of values.
    pub(crate) fn data_at(&self, r: u32, c: u32, p: &PivotTable) -> Option<usize> {
        let row = self.row_lines.get(r as usize)?;
        let col = self.col_lines.get(c as usize)?;
        row.data().or(col.data()).or((p.data_fields.len() == 1).then_some(0))
    }

    /// Whose number format a cell of the report takes.
    pub(crate) fn format_of(&self, r: u32, c: u32, p: &PivotTable) -> FormatOf {
        let label = |rows: bool, level: u8| {
            let levels = if rows { &self.row_levels } else { &self.col_levels };
            match levels.get(level as usize).and_then(|l| l.field) {
                Some(f) => FormatOf::Field(f),
                None => FormatOf::Nothing,
            }
        };
        match self.tag(r, c) {
            Tag::Value => match self.data_at(r, c, p) {
                Some(d) if self.row_lines[r as usize].func > 0 || self.col_lines[c as usize].func > 0 => {
                    FormatOf::Subtotal(d)
                }
                Some(d) => FormatOf::Data(d),
                None => FormatOf::Nothing,
            },
            Tag::RowLabel(l) => label(true, l),
            Tag::ColLabel(l) => label(false, l),
            _ => FormatOf::Nothing,
        }
    }

    /// How deep the label of a cell is indented, for the labels of fields
    /// in compact form: Excel aligns those to the left, numbers included.
    pub(crate) fn indent(&self, r: u32, c: u32) -> Option<u8> {
        match self.tag(r, c) {
            Tag::RowLabel(l) | Tag::RowSubtotal(l) => {
                self.row_levels.get(l as usize).filter(|v| v.compact).map(|v| v.indent)
            }
            _ => None,
        }
    }

    /// Whether the label of a cell leaves room on its left for the expand /
    /// collapse button Excel draws there. In a compact column every label
    /// moves over as soon as one of its fields has detail below it, so that
    /// they stay lined up; elsewhere only the text labels of a field with
    /// another field below it do (numbers are aligned to the right).
    pub(crate) fn drill_margin(&self, r: u32, c: u32, p: &PivotTable) -> bool {
        if !p.options.show_drill {
            return false;
        }
        let (rows, level, subtotal) = match self.tag(r, c) {
            Tag::RowLabel(l) => (true, l as usize, false),
            Tag::RowSubtotal(l) => (true, l as usize, true),
            Tag::ColLabel(l) => (false, l as usize, false),
            _ => return false,
        };
        let (levels, axis) = if rows {
            (&self.row_levels, &p.row_fields)
        } else {
            (&self.col_levels, &p.col_fields)
        };
        let Some(lvl) = levels.get(level) else {
            return false;
        };
        let has_button =
            |k: usize, l: &Level| l.field.is_some() && axis.iter().skip(k + 1).any(|a| a.field().is_some());
        if lvl.compact {
            levels
                .iter()
                .enumerate()
                .any(|(k, l)| l.compact && l.slot == lvl.slot && has_button(k, l))
        } else {
            !subtotal && has_button(level, lvl) && !self.value(r, c).is_number()
        }
    }
}

// ---- engine ----------------------------------------------------------------

struct Axis {
    levels: Vec<Level>,
    /// Position of "Σ Values" in the axis.
    values_pos: Option<usize>,
    /// The fields of the axis, without "Σ Values".
    fields: Vec<usize>,
}

struct Engine<'a> {
    p: &'a PivotTable,
    locale: &'a Locale,
    nd: usize,
    /// Per field in use: the item of every record.
    columns: Vec<Option<Cow<'a, [u32]>>>,
    /// Per field in use: which items pass the item and label filters.
    visible: Vec<Option<Vec<bool>>>,
    /// Per field on an axis or in the filters: what its items carry.
    flags: Vec<Option<ItemFlags<'a>>>,
    keep: Vec<bool>,
    rows: Axis,
    cols: Axis,
    row_tree: Tree,
    col_tree: Tree,
    row_index: FastMap<u32>,
    col_index: FastMap<u32>,
    rec_row: Vec<u32>,
    rec_col: Vec<u32>,
    cube: FastMap<u32>,
    accs: Vec<Acc>,
    /// Accumulators per cell: one per value field, then one per field the
    /// calculated fields compute from.
    stride: usize,
    calc: Calculated,
}

/// The calculated fields and items of the cache, ready to be computed.
#[derive(Default)]
struct Calculated {
    /// Per field of the cache: the formula of a calculated field.
    fields: Vec<Option<CalcFormula>>,
    /// The fields whose sums the calculated fields need, each with an
    /// accumulator of its own after those of the value fields.
    summed: Vec<usize>,
    /// The calculated items in solve order: field, item, formula over the
    /// items of the field.
    items: Vec<(usize, u32, CalcFormula)>,
    /// Any workbook: the functions of a formula are computed in one.
    scratch: Option<Workbook>,
}

impl Calculated {
    fn of(p: &PivotTable) -> Calculated {
        let cache = &p.cache;
        let names = cache.field_names();
        let fields: Vec<Option<CalcFormula>> = cache
            .fields
            .iter()
            .map(|f| CalcFormula::parse(f.formula.as_ref()?, &names, Locale::en()).ok())
            .collect();
        let mut summed: Vec<usize> = Vec::new();
        for formula in fields.iter().flatten() {
            for used in &formula.uses {
                if fields[*used].is_none() && !summed.contains(used) {
                    summed.push(*used);
                }
            }
        }
        let items: Vec<(usize, u32, CalcFormula)> = cache
            .calc_items
            .iter()
            .filter_map(|item| {
                let x = cache.calc_item_index(item)?;
                let formula = CalcFormula::parse(&item.formula, &cache.item_names(item.field), Locale::en()).ok()?;
                Some((item.field, x, formula))
            })
            .collect();
        let any = fields.iter().any(Option::is_some) || !items.is_empty();
        Calculated {
            fields,
            summed,
            items,
            scratch: any.then(|| Workbook::new(Locale::en())),
        }
    }
}

/// Collapsed state and custom captions of the items of a field, by item.
struct ItemFlags<'a> {
    collapsed: Vec<bool>,
    captions: Vec<Option<&'a str>>,
}

/// The text of a label as a report shows it.
pub(crate) fn label_text(p: &PivotTable, field: usize, label: &Value, locale: &Locale) -> String {
    match label {
        Value::Number(n) => {
            let code = p.label_format(field).unwrap_or_else(|| Arc::from("General"));
            format::format_number(*n, &code, locale).text
        }
        other => format::format_value(other, "General", locale).text,
    }
}

impl LabelFilter {
    /// Whether an item with this label passes the filter. Comparisons are
    /// numeric when both sides are numbers, alphabetical otherwise.
    pub fn matches(&self, label: &str, locale: &Locale) -> bool {
        let lower = label.to_lowercase();
        let cmp = |other: &str| match (locale.parse_number(label), locale.parse_number(other)) {
            (Some(a), Some(b)) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
            _ => compare_text(label, other),
        };
        match self {
            LabelFilter::Equals(s) => wildcard_match(s, label),
            LabelFilter::NotEquals(s) => !wildcard_match(s, label),
            LabelFilter::BeginsWith(s) => lower.starts_with(&s.to_lowercase()),
            LabelFilter::NotBeginsWith(s) => !lower.starts_with(&s.to_lowercase()),
            LabelFilter::EndsWith(s) => lower.ends_with(&s.to_lowercase()),
            LabelFilter::NotEndsWith(s) => !lower.ends_with(&s.to_lowercase()),
            LabelFilter::Contains(s) => lower.contains(&s.to_lowercase()),
            LabelFilter::NotContains(s) => !lower.contains(&s.to_lowercase()),
            LabelFilter::GreaterThan(s) => cmp(s) == Ordering::Greater,
            LabelFilter::GreaterOrEqual(s) => cmp(s) != Ordering::Less,
            LabelFilter::LessThan(s) => cmp(s) == Ordering::Less,
            LabelFilter::LessOrEqual(s) => cmp(s) != Ordering::Greater,
            LabelFilter::Between(a, b) => cmp(a) != Ordering::Less && cmp(b) != Ordering::Greater,
            LabelFilter::NotBetween(a, b) => cmp(a) == Ordering::Less || cmp(b) == Ordering::Greater,
        }
    }
}

impl ValueFilter {
    pub fn matches(&self, v: f64) -> bool {
        match *self {
            ValueFilter::Equals(x) => v == x,
            ValueFilter::NotEquals(x) => v != x,
            ValueFilter::GreaterThan(x) => v > x,
            ValueFilter::GreaterOrEqual(x) => v >= x,
            ValueFilter::LessThan(x) => v < x,
            ValueFilter::LessOrEqual(x) => v <= x,
            ValueFilter::Between(a, b) => v >= a && v <= b,
            ValueFilter::NotBetween(a, b) => v < a || v > b,
        }
    }
}

/// Order of two items sorted by label: numbers first, then text without
/// regard to case, then booleans, errors and the blank item.
fn compare_items(a: &Value, b: &Value) -> Ordering {
    let rank = |v: &Value| match v {
        Value::Number(_) => 0,
        Value::Text(_) => 1,
        Value::Bool(_) => 2,
        Value::Error(_) => 3,
        _ => 4,
    };
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.partial_cmp(y).unwrap_or(Ordering::Equal),
        (Value::Text(x), Value::Text(y)) => compare_text(x, y),
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        _ => rank(a).cmp(&rank(b)),
    }
}

/// The items of a field in ascending label order. A field grouped by range
/// keeps the order of its groups ("Jan" before "Feb").
pub(crate) fn ascending_items(p: &PivotTable, field: usize) -> Vec<u32> {
    let items = p.cache.items(field);
    let mut order: Vec<u32> = (0..items.len() as u32).collect();
    let by_range = p
        .cache
        .fields
        .get(field)
        .and_then(|f| f.group.as_ref())
        .is_some_and(|g| g.kind != super::GroupKind::Discrete);
    if !by_range {
        order.sort_by(|a, b| compare_items(&items[*a as usize], &items[*b as usize]));
    }
    order
}

fn number(v: &Option<Value>) -> Result<Option<f64>, ErrorKind> {
    match v {
        Some(Value::Number(n)) => Ok(Some(*n)),
        Some(Value::Error(e)) => Err(*e),
        _ => Ok(None),
    }
}

fn ratio(own: &Option<Value>, base: &Option<Value>) -> Option<Value> {
    let own = match number(own) {
        Ok(Some(n)) => n,
        Ok(None) => return None,
        Err(e) => return Some(Value::Error(e)),
    };
    Some(match number(base) {
        Ok(Some(b)) if b != 0.0 => Value::Number(own / b),
        Ok(Some(_)) => Value::Error(ErrorKind::Div0),
        Ok(None) => Value::Error(ErrorKind::Null),
        Err(e) => Value::Error(e),
    })
}

/// Where the base item of a "Show Values As" calculation lies for a cell.
enum Rel {
    /// The cell is a total above the base field: nothing to compare.
    Above,
    /// The cell is the base item itself.
    Same,
    /// The base item has no such cell.
    Missing,
    Node(u32),
}

impl<'a> Engine<'a> {
    fn new(p: &'a PivotTable, locale: &'a Locale) -> Engine<'a> {
        let n = p.cache.rows;
        let nf = p.cache.fields.len();
        let mut e = Engine {
            p,
            locale,
            nd: p.data_fields.len(),
            columns: (0..nf).map(|_| None).collect(),
            visible: vec![None; nf],
            flags: (0..nf).map(|_| None).collect(),
            keep: vec![true; n],
            rows: Axis {
                levels: Vec::new(),
                values_pos: None,
                fields: Vec::new(),
            },
            cols: Axis {
                levels: Vec::new(),
                values_pos: None,
                fields: Vec::new(),
            },
            row_tree: Tree::default(),
            col_tree: Tree::default(),
            row_index: FastMap::default(),
            col_index: FastMap::default(),
            rec_row: vec![NONE; n],
            rec_col: vec![NONE; n],
            cube: FastMap::default(),
            accs: Vec::new(),
            stride: p.data_fields.len(),
            calc: Calculated::of(p),
        };
        e.stride += e.calc.summed.len();
        e.rows = e.axis(true);
        e.cols = e.axis(false);
        let used = e
            .rows
            .fields
            .iter()
            .chain(&e.cols.fields)
            .copied()
            .chain(p.page_fields.iter().map(|pf| pf.field));
        for f in used.collect::<Vec<_>>() {
            let Some(pf) = p.fields.get(f) else {
                continue;
            };
            let count = p.cache.items(f).len();
            let mut flags = ItemFlags {
                collapsed: vec![false; count],
                captions: vec![None; count],
            };
            for it in pf.items.iter().filter(|it| (it.x as usize) < count) {
                flags.collapsed[it.x as usize] = it.collapsed;
                flags.captions[it.x as usize] = it.caption.as_deref();
            }
            e.flags[f] = Some(flags);
        }
        e
    }

    /// The label of an item: its custom caption, its value, or "(blank)".
    fn item_value(&self, field: usize, item: u32) -> Value {
        let caption = self
            .flags
            .get(field)
            .and_then(|f| f.as_ref())
            .and_then(|f| f.captions.get(item as usize).copied().flatten());
        match (caption, self.p.cache.items(field).get(item as usize)) {
            (Some(c), _) => Value::text(c),
            (None, Some(Value::Empty) | None) => Value::text(text::caption(Caption::Blank, self.locale)),
            (None, Some(v)) => v.clone(),
        }
    }

    /// Describe one axis: where each field puts its labels, which fields
    /// show subtotals and where.
    fn axis(&self, rows: bool) -> Axis {
        let p = self.p;
        let list = if rows { &p.row_fields } else { &p.col_fields };
        let values_pos = list.iter().position(|a| *a == AxisField::Values);
        let fields: Vec<usize> = list.iter().filter_map(|a| a.field()).collect();
        let n = list.len();
        let mut levels: Vec<Level> = Vec::with_capacity(n);
        let mut depth = 0u8;
        for (pos, a) in list.iter().enumerate() {
            let deeper = pos + 1 < n;
            let real_below = list[pos + 1..].iter().any(|x| x.field().is_some());
            let values_below = values_pos.is_some_and(|v| v > pos);
            let (slot, indent) = match levels.last() {
                None => (0, 0),
                Some(prev) if rows && prev.compact => (prev.slot, prev.indent + 1),
                Some(prev) => (prev.slot + 1, 0),
            };
            let level = match a.field().and_then(|f| p.fields.get(f).map(|pf| (f, pf))) {
                Some((f, pf)) => {
                    depth += 1;
                    let own_line = rows && pf.layout != FieldLayout::Tabular && deeper;
                    let subtotal = match &pf.subtotals {
                        _ if !real_below => Sub::None,
                        Subtotals::None => Sub::None,
                        Subtotals::Automatic if own_line && pf.subtotal_top && !values_below => Sub::Top,
                        Subtotals::Automatic => Sub::Bottom,
                        Subtotals::Custom(fs) if fs.is_empty() => Sub::None,
                        Subtotals::Custom(fs) => Sub::Custom(fs.clone()),
                    };
                    Level {
                        field: Some(f),
                        depth,
                        slot,
                        indent,
                        own_line,
                        subtotal,
                        blank: rows && pf.blank_line && deeper,
                        repeat: rows && pf.repeat_labels,
                        compact: rows && pf.layout == FieldLayout::Compact,
                    }
                }
                None => {
                    // "Σ Values" takes the form of the fields around it.
                    let neighbour = list
                        .iter()
                        .filter_map(|x| x.field())
                        .next()
                        .and_then(|f| p.fields.get(f));
                    let layout = neighbour.map(|pf| pf.layout).unwrap_or(FieldLayout::Compact);
                    Level {
                        field: None,
                        depth,
                        slot,
                        indent,
                        own_line: rows && layout != FieldLayout::Tabular && deeper,
                        subtotal: Sub::None,
                        blank: false,
                        repeat: false,
                        compact: rows && layout == FieldLayout::Compact,
                    }
                }
            };
            levels.push(level);
        }
        Axis {
            levels,
            values_pos,
            fields,
        }
    }

    fn column(&mut self, field: usize) -> &[u32] {
        if self.columns[field].is_none() {
            self.columns[field] = Some(self.p.cache.item_column(field));
        }
        self.columns[field].as_deref().unwrap_or(&[])
    }

    /// Drop the records of hidden items, of items a label filter rejects,
    /// and of everything but the selection of the filter fields.
    fn filter_items(&mut self) {
        let p = self.p;
        let mut used: Vec<(usize, Option<Option<u32>>)> =
            p.page_fields.iter().map(|pf| (pf.field, Some(pf.item))).collect();
        used.extend(self.rows.fields.iter().chain(&self.cols.fields).map(|f| (*f, None)));
        // A slicer or a timeline filters its field wherever it is.
        let dated = p.filters.iter().filter(|x| matches!(x.kind, FilterKind::Date { .. })).map(|x| x.field);
        for f in p.sliced.iter().copied().chain(dated) {
            if !used.iter().any(|(u, _)| *u == f) {
                used.push((f, None));
            }
        }
        for (f, page) in used {
            let Some(pf) = p.fields.get(f) else {
                continue;
            };
            let count = p.cache.items(f).len();
            let mut vis = vec![true; count];
            match page {
                Some(Some(x)) => {
                    vis.fill(false);
                    if let Some(v) = vis.get_mut(x as usize) {
                        *v = true;
                    }
                }
                _ => {
                    for it in &pf.items {
                        if it.hidden
                            && let Some(v) = vis.get_mut(it.x as usize)
                        {
                            *v = false;
                        }
                    }
                }
            }
            if page.is_none() {
                for filter in p.filters.iter().filter(|x| x.field == f) {
                    if let FilterKind::Label(test) = &filter.kind {
                        for (x, v) in vis.iter_mut().enumerate() {
                            if *v
                                && !test.matches(
                                    &label_text(p, f, &self.item_value(f, x as u32), self.locale),
                                    self.locale,
                                )
                            {
                                *v = false;
                            }
                        }
                    }
                }
            }
            for filter in p.filters.iter().filter(|x| x.field == f) {
                if let FilterKind::Date { start, end } = filter.kind {
                    for (v, item) in vis.iter_mut().zip(p.cache.items(f)) {
                        // Whole days: a time of day does not put a record
                        // outside its day.
                        *v &= matches!(item, Value::Number(n) if n.floor() >= start && n.floor() <= end);
                    }
                }
            }
            if vis.iter().any(|v| !v) {
                let column = self.p.cache.item_column(f);
                for (k, x) in self.keep.iter_mut().zip(column.iter()) {
                    if !vis.get(*x as usize).copied().unwrap_or(false) {
                        *k = false;
                    }
                }
                self.columns[f] = Some(column);
            }
            self.visible[f] = Some(vis);
        }
    }

    /// Sort the kept records into a tree over `fields`; returns the leaf of
    /// each record.
    fn build_tree(&mut self, fields: &[usize]) -> (Tree, FastMap<u32>, Vec<u32>) {
        for f in fields {
            self.column(*f);
        }
        let columns: Vec<&[u32]> = fields
            .iter()
            .map(|f| self.columns[*f].as_deref().unwrap_or(&[]))
            .collect();
        let mut tree = Tree::default();
        let mut index = FastMap::default();
        let mut leaves = vec![NONE; self.keep.len()];
        for (r, leaf) in leaves.iter_mut().enumerate() {
            if !self.keep[r] {
                continue;
            }
            let mut node = 0;
            for c in &columns {
                node = tree.child(&mut index, node, c[r]);
            }
            *leaf = node;
        }
        (tree, index, leaves)
    }

    /// Value filters and Top 10 filters, in order: each one looks at the
    /// totals of the items of its field once the filters before it have
    /// been applied, and drops the records of the items it rejects.
    fn filter_values(&mut self) {
        let p = self.p;
        for filter in &p.filters {
            let (data, test): (usize, Rejects) = match &filter.kind {
                FilterKind::Label(_) | FilterKind::Date { .. } => continue,
                FilterKind::Value { data_field, test } => {
                    let test = *test;
                    (
                        *data_field,
                        Box::new(move |sibs| {
                            sibs.iter()
                                .filter(|(_, v)| !test.matches(*v))
                                .map(|(n, _)| *n)
                                .collect()
                        }),
                    )
                }
                FilterKind::Top10 {
                    data_field,
                    top,
                    count,
                    by,
                } => {
                    let (top, count, by) = (*top, *count, *by);
                    (*data_field, Box::new(move |sibs| top10_rejects(sibs, top, count, by)))
                }
            };
            let Some(df) = p.data_fields.get(data) else {
                continue;
            };
            let fields: Vec<usize> = match (
                self.rows.fields.iter().position(|f| *f == filter.field),
                self.cols.fields.iter().position(|f| *f == filter.field),
            ) {
                (Some(k), _) => self.rows.fields[..=k].to_vec(),
                (None, Some(k)) => self.cols.fields[..=k].to_vec(),
                _ => continue,
            };
            let (tree, _, leaves) = self.build_tree(&fields);
            let source = p.cache.database_field(df.field);
            let Some(cf) = p.cache.fields.get(source) else {
                continue;
            };
            // A calculated field has no values of its own to filter by.
            if cf.formula.is_some() {
                continue;
            }
            let mut accs = vec![Acc::EMPTY; tree.nodes.len()];
            for (r, leaf) in leaves.iter().enumerate() {
                if *leaf != NONE {
                    accs[*leaf as usize].add(&cf.shared[cf.records[r] as usize]);
                }
            }
            let depth = fields.len() as u8;
            let mut rejected = vec![false; tree.nodes.len()];
            for parent in tree.nodes.iter().filter(|n| n.depth + 1 == depth) {
                let sibs: Vec<(u32, f64)> = parent
                    .children
                    .iter()
                    .map(|c| {
                        let v = match accs[*c as usize].result(df.function) {
                            Some(Value::Number(n)) => n,
                            _ => 0.0,
                        };
                        (*c, v)
                    })
                    .collect();
                for node in test(&sibs) {
                    rejected[node as usize] = true;
                }
            }
            for (k, leaf) in self.keep.iter_mut().zip(&leaves) {
                if *leaf != NONE && rejected[*leaf as usize] {
                    *k = false;
                }
            }
        }
    }

    /// Items without any record still get their line when their field
    /// shows items with no data.
    fn add_empty_items(&mut self, rows: bool) {
        let fields = if rows {
            self.rows.fields.clone()
        } else {
            self.cols.fields.clone()
        };
        for (d, f) in fields.iter().enumerate() {
            if !self.p.fields.get(*f).is_some_and(|pf| pf.show_all) {
                continue;
            }
            let Some(vis) = self.visible[*f].clone() else {
                continue;
            };
            let (tree, index) = if rows {
                (&mut self.row_tree, &mut self.row_index)
            } else {
                (&mut self.col_tree, &mut self.col_index)
            };
            let parents: Vec<u32> = (0..tree.nodes.len() as u32)
                .filter(|n| tree.nodes[*n as usize].depth as usize == d)
                .collect();
            for parent in parents {
                for (x, v) in vis.iter().enumerate() {
                    if *v {
                        tree.child(index, parent, x as u32);
                    }
                }
            }
        }
    }

    /// Accumulate the value fields per pair of leaves, then roll the pairs
    /// up to every pair of ancestors.
    fn aggregate(&mut self) {
        // `nd` accumulators per cell: those of the value fields, then
        // those of the fields calculated fields compute from.
        let nd = self.stride;
        if self.nd == 0 {
            return;
        }
        let p = self.p;
        let sources: Vec<(&[Value], &[u32])> = p
            .data_fields
            .iter()
            .map(|d| p.cache.database_field(d.field))
            .chain(self.calc.summed.iter().copied())
            .map(|f| match p.cache.fields.get(f) {
                Some(cf) => (&cf.shared[..], &cf.records[..]),
                None => (&[][..], &[][..]),
            })
            .collect();
        for r in 0..self.keep.len() {
            if !self.keep[r] {
                continue;
            }
            let next = (self.accs.len() / nd) as u32;
            let cell = *self.cube.entry(pair(self.rec_row[r], self.rec_col[r])).or_insert(next);
            if cell == next {
                self.accs.extend(std::iter::repeat_n(Acc::EMPTY, nd));
            }
            let base = cell as usize * nd;
            for (d, (shared, records)) in sources.iter().enumerate() {
                if let Some(v) = records.get(r).and_then(|i| shared.get(*i as usize)) {
                    self.accs[base + d].add(v);
                }
            }
        }
        let leaves: Vec<(u64, u32)> = self.cube.iter().map(|(k, v)| (*k, *v)).collect();
        let mut buffer = vec![Acc::EMPTY; nd];
        let ancestors = |tree: &Tree, mut node: u32| {
            let mut out = Vec::with_capacity(4);
            while node != NONE {
                out.push(node);
                node = tree.nodes[node as usize].parent;
            }
            out
        };
        for (key, cell) in leaves {
            let (rn, cn) = ((key >> 32) as u32, key as u32);
            buffer.copy_from_slice(&self.accs[cell as usize * nd..cell as usize * nd + nd]);
            let ra = ancestors(&self.row_tree, rn);
            let ca = ancestors(&self.col_tree, cn);
            for a in &ra {
                for b in &ca {
                    if (*a, *b) == (rn, cn) {
                        continue;
                    }
                    let next = (self.accs.len() / nd) as u32;
                    let target = *self.cube.entry(pair(*a, *b)).or_insert(next);
                    if target == next {
                        self.accs.extend(std::iter::repeat_n(Acc::EMPTY, nd));
                    }
                    let base = target as usize * nd;
                    for (d, acc) in buffer.iter().enumerate() {
                        self.accs[base + d].merge(acc);
                    }
                }
            }
        }
    }

    /// The summary of a value field over a pair of nodes.
    fn raw(&self, rn: u32, cn: u32, d: usize, f: Aggregation) -> Option<Value> {
        self.raw_at(rn, cn, d, f, 0)
    }

    /// `raw`, `depth` formulas deep: calculated items compute from the
    /// cells of other items, which may be calculated too.
    fn raw_at(&self, rn: u32, cn: u32, d: usize, f: Aggregation, depth: u8) -> Option<Value> {
        if !self.calc.items.is_empty() && depth < 12 {
            // Where the formulas of two calculated items meet, the last
            // one in solve order gives the value.
            let hit = [self.calc_item_at(true, rn), self.calc_item_at(false, cn)].into_iter().flatten().max_by_key(|h| h.0);
            if let Some((order, rows, level)) = hit {
                return Some(self.calc_item_value(order, rows, level, (rn, cn), d, f, depth));
            }
        }
        let field = self.p.data_fields.get(d)?.field;
        let plain = if self.calc.fields.get(field).is_some_and(Option::is_some) {
            let cell = *self.cube.get(&pair(rn, cn))?;
            Some(self.calc_field_value(cell as usize, field, 0))
        } else {
            let cell = self.cube.get(&pair(rn, cn));
            cell.and_then(|cell| self.accs.get(*cell as usize * self.stride + d)?.result(f))
        };
        if self.calc.items.is_empty() || depth >= 12 || f != Aggregation::Sum {
            return plain;
        }
        // Subtotals and grand totals take the calculated items in.
        let mut extra: Option<f64> = None;
        for (rows, node) in [(true, rn), (false, cn)] {
            for below in self.calc_items_below(rows, node) {
                let v = if rows { self.raw_at(below, cn, d, f, depth + 1) } else { self.raw_at(rn, below, d, f, depth + 1) };
                if let Some(Value::Number(n)) = v {
                    extra = Some(extra.unwrap_or(0.0) + n);
                }
            }
        }
        match (plain, extra) {
            (Some(Value::Number(n)), Some(more)) => Some(Value::Number(n + more)),
            (None, Some(more)) => Some(Value::Number(more)),
            (plain, _) => plain,
        }
    }

    /// The value of a calculated field over the records of a cell: its
    /// formula applied to the sums of the fields it names.
    fn calc_field_value(&self, cell: usize, field: usize, depth: u8) -> Value {
        let (Some(Some(formula)), Some(scratch)) = (self.calc.fields.get(field), &self.calc.scratch) else {
            return Value::Error(ErrorKind::Name);
        };
        let sum = |used: usize| -> Result<f64, ErrorKind> {
            if self.calc.fields.get(used).is_some_and(Option::is_some) {
                // Another calculated field; one that computes from itself
                // stops here.
                if depth >= 12 {
                    return Err(ErrorKind::Ref);
                }
                return match self.calc_field_value(cell, used, depth + 1) {
                    Value::Number(n) => Ok(n),
                    Value::Error(e) => Err(e),
                    _ => Ok(0.0),
                };
            }
            let Some(slot) = self.calc.summed.iter().position(|f| *f == used) else {
                return Err(ErrorKind::Name);
            };
            match self.accs.get(cell * self.stride + self.nd + slot).and_then(|acc| acc.result(Aggregation::Sum)) {
                Some(Value::Number(n)) => Ok(n),
                Some(Value::Error(e)) => Err(e),
                _ => Ok(0.0),
            }
        };
        formula.compute(&sum, scratch)
    }

    /// The fields of an axis that have calculated items, with the depth of
    /// their nodes.
    fn calc_levels(&self, rows: bool) -> impl Iterator<Item = (u8, usize)> + '_ {
        let fields = if rows { &self.rows.fields } else { &self.cols.fields };
        fields
            .iter()
            .enumerate()
            .filter(|(_, f)| self.calc.items.iter().any(|(field, ..)| field == *f))
            .map(|(k, f)| (k as u8 + 1, *f))
    }

    /// The calculated item a node lies under, if any: its place in the
    /// solve order, the axis, and the depth of the item's node.
    fn calc_item_at(&self, rows: bool, node: u32) -> Option<(usize, bool, u8)> {
        let tree = if rows { &self.row_tree } else { &self.col_tree };
        self.calc_levels(rows)
            .filter_map(|(level, field)| {
                let a = tree.ancestor(node, level);
                let item = tree.nodes.get(a as usize)?.item;
                let order = self.calc.items.iter().position(|(f, x, _)| *f == field && *x == item)?;
                Some((order, rows, level))
            })
            .max_by_key(|h| h.0)
    }

    /// The nodes of calculated items under a node that is above them.
    fn calc_items_below(&self, rows: bool, node: u32) -> Vec<u32> {
        let tree = if rows { &self.row_tree } else { &self.col_tree };
        let mut out = Vec::new();
        let Some(start) = tree.nodes.get(node as usize) else {
            return out;
        };
        for (level, field) in self.calc_levels(rows) {
            if start.depth >= level {
                continue;
            }
            // Walk down to the level of the field.
            let mut at = vec![node];
            for _ in start.depth..level {
                at = at.iter().flat_map(|n| tree.nodes[*n as usize].children.iter().copied()).collect();
            }
            out.extend(at.into_iter().filter(|n| {
                let item = tree.nodes[*n as usize].item;
                self.calc.items.iter().any(|(f, x, _)| *f == field && *x == item)
            }));
        }
        out
    }

    /// The value of a cell under a calculated item: its formula applied to
    /// the cells that stand to the other items as this one to its own.
    #[allow(clippy::too_many_arguments)]
    fn calc_item_value(&self, order: usize, rows: bool, level: u8, at: (u32, u32), d: usize, f: Aggregation, depth: u8) -> Value {
        let (rn, cn) = at;
        let (Some((_, _, formula)), Some(scratch)) = (self.calc.items.get(order), &self.calc.scratch) else {
            return Value::Error(ErrorKind::Name);
        };
        let tree = if rows { &self.row_tree } else { &self.col_tree };
        let index = if rows { &self.row_index } else { &self.col_index };
        let Some((own, below)) = self.split(rows, if rows { rn } else { cn }, level) else {
            return Value::Error(ErrorKind::Ref);
        };
        let parent = tree.nodes[own as usize].parent;
        let cell = |item: usize| -> Result<f64, ErrorKind> {
            let Some(node) = index.get(&pair(parent, item as u32)).and_then(|n| self.descend(rows, *n, &below)) else {
                // The item has nothing here.
                return Ok(0.0);
            };
            let v = if rows { self.raw_at(node, cn, d, f, depth + 1) } else { self.raw_at(rn, node, d, f, depth + 1) };
            match v {
                Some(Value::Number(n)) => Ok(n),
                Some(Value::Error(e)) => Err(e),
                _ => Ok(0.0),
            }
        };
        formula.compute(&cell, scratch)
    }

    /// Give every calculated item its nodes: one under each node above its
    /// field, with below it the items its siblings have.
    fn add_calc_items(&mut self, rows: bool) {
        let levels: Vec<(u8, usize)> = self.calc_levels(rows).collect();
        for (level, field) in levels {
            let wanted: Vec<u32> = self
                .calc
                .items
                .iter()
                .filter(|(f, x, _)| *f == field && self.visible[field].as_ref().is_none_or(|vis| vis.get(*x as usize).copied().unwrap_or(false)))
                .map(|(_, x, _)| *x)
                .collect();
            let (tree, index) = if rows {
                (&mut self.row_tree, &mut self.row_index)
            } else {
                (&mut self.col_tree, &mut self.col_index)
            };
            let parents: Vec<u32> = (0..tree.nodes.len() as u32).filter(|n| tree.nodes[*n as usize].depth + 1 == level).collect();
            for parent in parents {
                let siblings = tree.nodes[parent as usize].children.clone();
                for x in &wanted {
                    let node = tree.child(index, parent, *x);
                    // Copy the shape of what the siblings have below them.
                    let mut todo: Vec<(u32, u32)> = siblings.iter().filter(|s| **s != node).map(|s| (*s, node)).collect();
                    while let Some((from, to)) = todo.pop() {
                        for child in tree.nodes[from as usize].children.clone() {
                            let item = tree.nodes[child as usize].item;
                            let copy = tree.child(index, to, item);
                            todo.push((child, copy));
                        }
                    }
                }
            }
        }
    }

    /// Put the children of every node in display order.
    fn sort_tree(&mut self, rows: bool) {
        let p = self.p;
        let fields = if rows {
            self.rows.fields.clone()
        } else {
            self.cols.fields.clone()
        };
        // Per level: rank of every item in manual order and in label order.
        let orders: Vec<(Vec<u32>, Vec<u32>)> = fields
            .iter()
            .map(|f| {
                let count = p.cache.items(*f).len();
                let mut manual = vec![u32::MAX; count];
                if let Some(pf) = p.fields.get(*f) {
                    for (k, it) in pf.items.iter().enumerate() {
                        if let Some(m) = manual.get_mut(it.x as usize) {
                            *m = k as u32;
                        }
                    }
                }
                let mut label = vec![0u32; count];
                for (k, x) in ascending_items(p, *f).into_iter().enumerate() {
                    label[x as usize] = k as u32;
                }
                (manual, label)
            })
            .collect();
        let count = if rows {
            self.row_tree.nodes.len()
        } else {
            self.col_tree.nodes.len()
        };
        for n in 0..count {
            let tree = if rows { &self.row_tree } else { &self.col_tree };
            let node = &tree.nodes[n];
            let Some(f) = fields.get(node.depth as usize) else {
                continue;
            };
            if node.children.len() < 2 {
                continue;
            }
            let Some(pf) = p.fields.get(*f) else {
                continue;
            };
            let (manual, label) = &orders[node.depth as usize];
            let item = |c: u32| tree.nodes[c as usize].item as usize;
            let mut keyed: Vec<(u32, f64, u32, u32)> = node
                .children
                .iter()
                .map(|c| {
                    let value = match pf.sort.by_value.and_then(|d| p.data_fields.get(d).map(|df| (d, df))) {
                        Some((d, df)) => {
                            let v = if rows {
                                self.raw(*c, 0, d, df.function)
                            } else {
                                self.raw(0, *c, d, df.function)
                            };
                            match v {
                                Some(Value::Number(x)) => x,
                                _ => 0.0,
                            }
                        }
                        None => 0.0,
                    };
                    (*c, value, manual[item(*c)], label[item(*c)])
                })
                .collect();
            let by_value = pf.sort.by_value.is_some_and(|d| d < self.nd);
            keyed.sort_by(|a, b| {
                let order = match pf.sort.order {
                    SortOrder::Manual => a.2.cmp(&b.2).then(a.3.cmp(&b.3)),
                    _ if by_value => a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal),
                    _ => a.3.cmp(&b.3),
                };
                let order = if pf.sort.order == SortOrder::Descending {
                    order.reverse()
                } else {
                    order
                };
                // Equal values keep the order of the labels.
                order.then(a.3.cmp(&b.3))
            });
            let sorted: Vec<u32> = keyed.into_iter().map(|k| k.0).collect();
            let tree = if rows { &mut self.row_tree } else { &mut self.col_tree };
            tree.nodes[n].children = sorted;
        }
    }

    // ---- lines -------------------------------------------------------------

    fn collapsed(&self, field: usize, item: u32) -> bool {
        let flags = self.flags.get(field).and_then(|f| f.as_ref());
        flags.is_some_and(|f| f.collapsed.get(item as usize).copied().unwrap_or(false))
    }

    fn lines(&self, rows: bool) -> Vec<Line> {
        let axis = if rows { &self.rows } else { &self.cols };
        let other_has_fields = if rows {
            !self.cols.fields.is_empty()
        } else {
            !self.rows.fields.is_empty()
        };
        let mut out = Vec::new();
        if axis.levels.is_empty() {
            // Everything on one line; without value fields it only exists
            // to hold the totals of the other axis' header.
            if self.nd > 0 || (!rows && !other_has_fields) {
                out.push(Line {
                    kind: LineKind::Item,
                    node: 0,
                    data: NO_DATA,
                    level: 0,
                    func: 0,
                    values: self.nd > 0,
                });
            }
            return out;
        }
        self.emit(rows, &mut out, 0, 0, NO_DATA);
        let wanted = if rows {
            self.p.options.col_grand_totals
        } else {
            self.p.options.row_grand_totals
        };
        if wanted && !axis.fields.is_empty() {
            let grand = |data: u16| Line {
                kind: LineKind::Grand,
                node: 0,
                data,
                level: 0,
                func: 0,
                values: true,
            };
            if axis.values_pos.is_some() {
                out.extend((0..self.nd as u16).map(grand));
            } else {
                out.push(grand(NO_DATA));
            }
        }
        out
    }

    fn emit(&self, rows: bool, out: &mut Vec<Line>, node: u32, pos: usize, data: u16) {
        let (axis, tree) = if rows {
            (&self.rows, &self.row_tree)
        } else {
            (&self.cols, &self.col_tree)
        };
        let Some(lvl) = axis.levels.get(pos) else {
            return;
        };
        let last = pos + 1 == axis.levels.len();
        let level = pos as u8;
        // A line shows values once its value field is known.
        let known = |data: u16| data != NO_DATA || axis.values_pos.is_none();
        let Some(f) = lvl.field else {
            for d in 0..self.nd as u16 {
                if last {
                    out.push(Line {
                        kind: LineKind::Item,
                        node,
                        data: d,
                        level,
                        func: 0,
                        values: true,
                    });
                } else {
                    if lvl.own_line {
                        out.push(Line {
                            kind: LineKind::Group,
                            node,
                            data: d,
                            level,
                            func: 0,
                            values: false,
                        });
                    }
                    self.emit(rows, out, node, pos + 1, d);
                }
            }
            return;
        };
        let values_below = axis.values_pos.is_some_and(|v| v > pos);
        for &child in &tree.nodes[node as usize].children {
            let item = tree.nodes[child as usize].item;
            if last {
                out.push(Line {
                    kind: LineKind::Item,
                    node: child,
                    data,
                    level,
                    func: 0,
                    values: known(data),
                });
            } else if self.collapsed(f, item) {
                if values_below {
                    // The value fields still show under a collapsed item.
                    let at = axis.values_pos.unwrap_or(pos);
                    if lvl.own_line {
                        out.push(Line {
                            kind: LineKind::Group,
                            node: child,
                            data,
                            level,
                            func: 0,
                            values: false,
                        });
                    }
                    for d in 0..self.nd as u16 {
                        out.push(Line {
                            kind: LineKind::Item,
                            node: child,
                            data: d,
                            level: at as u8,
                            func: 0,
                            values: true,
                        });
                    }
                } else {
                    out.push(Line {
                        kind: LineKind::Item,
                        node: child,
                        data,
                        level,
                        func: 0,
                        values: known(data),
                    });
                }
            } else {
                if lvl.own_line {
                    out.push(Line {
                        kind: LineKind::Group,
                        node: child,
                        data,
                        level,
                        func: 0,
                        values: lvl.subtotal == Sub::Top && known(data),
                    });
                }
                self.emit(rows, out, child, pos + 1, data);
                let functions = match &lvl.subtotal {
                    Sub::Bottom => 1,
                    Sub::Custom(fs) => fs.len(),
                    _ => 0,
                };
                for k in 0..functions {
                    let func = if lvl.subtotal == Sub::Bottom { 0 } else { k as u8 + 1 };
                    let datas: Vec<u16> = if values_below {
                        (0..self.nd as u16).collect()
                    } else {
                        vec![data]
                    };
                    for d in datas {
                        out.push(Line {
                            kind: LineKind::Subtotal,
                            node: child,
                            data: d,
                            level,
                            func,
                            values: known(d),
                        });
                    }
                }
            }
            if lvl.blank && !last {
                out.push(Line {
                    kind: LineKind::Blank,
                    node: child,
                    data: NO_DATA,
                    level,
                    func: 0,
                    values: false,
                });
            }
        }
    }

    // ---- values ------------------------------------------------------------

    /// Where a field sits: (on the row axis, depth of its nodes).
    fn locate(&self, field: usize) -> Option<(bool, u8)> {
        if let Some(k) = self.rows.fields.iter().position(|f| *f == field) {
            return Some((true, k as u8 + 1));
        }
        self.cols
            .fields
            .iter()
            .position(|f| *f == field)
            .map(|k| (false, k as u8 + 1))
    }

    /// The node at `depth` above `node`, and the items from there down to
    /// `node` (innermost first).
    fn split(&self, rows: bool, node: u32, depth: u8) -> Option<(u32, Vec<u32>)> {
        let tree = if rows { &self.row_tree } else { &self.col_tree };
        if tree.nodes[node as usize].depth < depth {
            return None;
        }
        let mut below = Vec::new();
        let mut a = node;
        while tree.nodes[a as usize].depth > depth {
            below.push(tree.nodes[a as usize].item);
            a = tree.nodes[a as usize].parent;
        }
        Some((a, below))
    }

    /// Follow `below` (innermost first) down from `from`.
    fn descend(&self, rows: bool, from: u32, below: &[u32]) -> Option<u32> {
        let index = if rows { &self.row_index } else { &self.col_index };
        let mut n = from;
        for item in below.iter().rev() {
            n = *index.get(&pair(n, *item))?;
        }
        Some(n)
    }

    /// The node that stands to `node` as the base item stands to the item
    /// of the base field in `node`.
    fn relative(&self, rows: bool, node: u32, depth: u8, base: BaseItem) -> Rel {
        let tree = if rows { &self.row_tree } else { &self.col_tree };
        let index = if rows { &self.row_index } else { &self.col_index };
        let Some((a, below)) = self.split(rows, node, depth) else {
            return Rel::Above;
        };
        let parent = tree.nodes[a as usize].parent;
        let sibs = &tree.nodes[parent as usize].children;
        let at = sibs.iter().position(|s| *s == a).unwrap_or(0);
        let target = match base {
            BaseItem::Item(x) if tree.nodes[a as usize].item == x => return Rel::Same,
            BaseItem::Item(x) => index.get(&pair(parent, x)).copied(),
            // The first item has nothing before it: it is its own base.
            BaseItem::Previous if at == 0 => return Rel::Same,
            BaseItem::Previous => Some(sibs[at - 1]),
            BaseItem::Next if at + 1 >= sibs.len() => return Rel::Same,
            BaseItem::Next => Some(sibs[at + 1]),
        };
        match target.and_then(|t| self.descend(rows, t, &below)) {
            Some(n) => Rel::Node(n),
            None => Rel::Missing,
        }
    }

    /// The plain values of the cell's counterparts under every sibling of
    /// its base item, in display order, and the position of the cell's own.
    fn along(&self, rn: u32, cn: u32, d: usize, field: usize) -> Option<(Vec<Option<f64>>, usize)> {
        let (rows, depth) = self.locate(field)?;
        let tree = if rows { &self.row_tree } else { &self.col_tree };
        let (a, below) = self.split(rows, if rows { rn } else { cn }, depth)?;
        let parent = tree.nodes[a as usize].parent;
        let sibs = &tree.nodes[parent as usize].children;
        let f = self.p.data_fields[d].function;
        let values = sibs
            .iter()
            .map(|s| {
                let n = self.descend(rows, *s, &below)?;
                let v = if rows {
                    self.raw(n, cn, d, f)
                } else {
                    self.raw(rn, n, d, f)
                };
                number(&v).ok().flatten()
            })
            .collect();
        Some((values, sibs.iter().position(|s| *s == a)?))
    }

    /// The value a cell shows: the summary of its value field, put through
    /// its "Show Values As" calculation.
    fn shown(&self, rn: u32, cn: u32, d: usize, custom: Option<Aggregation>) -> Option<Value> {
        let df = &self.p.data_fields[d];
        if let Some(f) = custom {
            return self.raw(rn, cn, d, f);
        }
        let f = df.function;
        let own = self.raw(rn, cn, d, f);
        let rel = |field: usize, item: BaseItem| -> Option<(Rel, bool)> {
            let (rows, depth) = self.locate(field)?;
            Some((self.relative(rows, if rows { rn } else { cn }, depth, item), rows))
        };
        let other = |rows: bool, n: u32| {
            if rows {
                self.raw(n, cn, d, f)
            } else {
                self.raw(rn, n, d, f)
            }
        };
        let na = Some(Value::Error(ErrorKind::NA));
        match df.show_as {
            ShowAs::Normal => own,
            ShowAs::PercentOfTotal => ratio(&own, &self.raw(0, 0, d, f)),
            ShowAs::PercentOfColumn => ratio(&own, &self.raw(0, cn, d, f)),
            ShowAs::PercentOfRow => ratio(&own, &self.raw(rn, 0, d, f)),
            ShowAs::PercentOfParentRow => {
                let parent = self.row_tree.nodes[rn as usize].parent;
                ratio(&own, &self.raw(if parent == NONE { 0 } else { parent }, cn, d, f))
            }
            ShowAs::PercentOfParentColumn => {
                let parent = self.col_tree.nodes[cn as usize].parent;
                ratio(&own, &self.raw(rn, if parent == NONE { 0 } else { parent }, d, f))
            }
            ShowAs::PercentOfParent { field } => {
                let Some((rows, depth)) = self.locate(field) else {
                    return na;
                };
                let tree = if rows { &self.row_tree } else { &self.col_tree };
                match tree.ancestor(if rows { rn } else { cn }, depth) {
                    NONE => None,
                    a => ratio(&own, &other(rows, a)),
                }
            }
            ShowAs::PercentOf { field, item } => match rel(field, item) {
                None => na,
                Some((Rel::Above, _)) => None,
                Some((Rel::Same, _)) => own.as_ref().map(|_| Value::Number(1.0)),
                Some((Rel::Missing, _)) => own.as_ref().map(|_| Value::Error(ErrorKind::Null)),
                Some((Rel::Node(n), rows)) => ratio(&own, &other(rows, n)),
            },
            ShowAs::Difference { field, item } | ShowAs::PercentDifference { field, item } => {
                let percent = matches!(df.show_as, ShowAs::PercentDifference { .. });
                let base = match rel(field, item) {
                    None => return na,
                    Some((Rel::Above | Rel::Same, _)) => return None,
                    Some((Rel::Missing, _)) => None,
                    Some((Rel::Node(n), rows)) => other(rows, n),
                };
                match (number(&own), number(&base)) {
                    (Err(e), _) | (_, Err(e)) => Some(Value::Error(e)),
                    (Ok(None), Ok(None)) => None,
                    (Ok(a), Ok(b)) if !percent => Some(Value::Number(a.unwrap_or(0.0) - b.unwrap_or(0.0))),
                    (Ok(_), Ok(None)) => Some(Value::Error(ErrorKind::Null)),
                    (Ok(a), Ok(Some(b))) => Some(match b == 0.0 {
                        true => Value::Error(ErrorKind::Div0),
                        false => Value::Number((a.unwrap_or(0.0) - b) / b),
                    }),
                }
            }
            ShowAs::RunningTotal { field } | ShowAs::PercentRunningTotal { field } => {
                if self.locate(field).is_none() {
                    return na;
                }
                let (values, at) = self.along(rn, cn, d, field)?;
                let running: f64 = values[..=at].iter().map(|v| v.unwrap_or(0.0)).sum();
                if matches!(df.show_as, ShowAs::RunningTotal { .. }) {
                    return Some(Value::Number(running));
                }
                let total: f64 = values.iter().map(|v| v.unwrap_or(0.0)).sum();
                Some(if total == 0.0 {
                    Value::Error(ErrorKind::Div0)
                } else {
                    Value::Number(running / total)
                })
            }
            ShowAs::RankAscending { field } | ShowAs::RankDescending { field } => {
                if self.locate(field).is_none() {
                    return na;
                }
                let (values, at) = self.along(rn, cn, d, field)?;
                let mine = values[at]?;
                let ascending = matches!(df.show_as, ShowAs::RankAscending { .. });
                // Equal values share a rank and the next rank follows on.
                let mut ahead: Vec<f64> = values
                    .iter()
                    .flatten()
                    .copied()
                    .filter(|v| if ascending { *v < mine } else { *v > mine })
                    .collect();
                ahead.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
                ahead.dedup();
                Some(Value::Number(ahead.len() as f64 + 1.0))
            }
            ShowAs::Index => {
                let (own, grand, row, col) = match (
                    number(&own),
                    number(&self.raw(0, 0, d, f)),
                    number(&self.raw(rn, 0, d, f)),
                    number(&self.raw(0, cn, d, f)),
                ) {
                    (Ok(Some(a)), Ok(Some(b)), Ok(Some(c)), Ok(Some(e))) => (a, b, c, e),
                    (Ok(None), ..) => return None,
                    (Err(e), ..) | (_, Err(e), ..) | (_, _, Err(e), _) | (.., Err(e)) => return Some(Value::Error(e)),
                    _ => return None,
                };
                Some(if row * col == 0.0 {
                    Value::Error(ErrorKind::Div0)
                } else {
                    Value::Number(own * grand / (row * col))
                })
            }
        }
    }

    // ---- grid --------------------------------------------------------------

    /// The label of a line at one of its levels: the item of the field, or
    /// the name of the value field for "Σ Values".
    fn label(&self, rows: bool, line: &Line, pos: usize) -> Value {
        let (axis, tree) = if rows {
            (&self.rows, &self.row_tree)
        } else {
            (&self.cols, &self.col_tree)
        };
        let lvl = &axis.levels[pos];
        match lvl.field {
            None => match self.p.data_fields.get(line.data as usize) {
                Some(d) => Value::text(&d.name),
                None => Value::Empty,
            },
            Some(f) => match tree.ancestor(line.node, lvl.depth) {
                NONE => Value::Empty,
                n => self.item_value(f, tree.nodes[n as usize].item),
            },
        }
    }

    /// What tells two lines apart at a level: the node of the field's item,
    /// or the value field.
    fn identity(&self, rows: bool, line: &Line, pos: usize) -> u32 {
        let (axis, tree) = if rows {
            (&self.rows, &self.row_tree)
        } else {
            (&self.cols, &self.col_tree)
        };
        match axis.levels[pos].field {
            None => line.data as u32,
            Some(_) => tree.ancestor(line.node, axis.levels[pos].depth),
        }
    }

    /// The labels of every line of an axis: (line, level, tag, value).
    fn labels(&self, rows: bool, lines: &[Line]) -> Vec<(usize, usize, Tag, Value)> {
        let axis = if rows { &self.rows } else { &self.cols };
        let locale = self.locale;
        let p = self.p;
        let mut out = Vec::new();
        let mut previous: Option<&Line> = None;
        for (i, line) in lines.iter().enumerate() {
            let level = line.level as usize;
            match line.kind {
                LineKind::Header | LineKind::Blank => continue,
                LineKind::Grand => {
                    let text = match p.data_fields.get(line.data as usize) {
                        Some(d) => text::grand_total_of(&d.name, locale),
                        None => p
                            .options
                            .grand_total_caption
                            .clone()
                            .unwrap_or_else(|| text::caption(Caption::GrandTotal, locale).to_string()),
                    };
                    out.push((
                        i,
                        0,
                        if rows { Tag::RowGrand } else { Tag::ColGrand },
                        Value::from(text),
                    ));
                    previous = None;
                    continue;
                }
                _ => {}
            }
            if axis.levels.is_empty() {
                continue;
            }
            // Labels of the levels above, where they share the line of
            // their first detail (tabular form, column headers).
            for a in 0..level {
                let lvl = &axis.levels[a];
                // An item on a line of its own is not named again below,
                // unless its labels are repeated (which compact form, with
                // one column for all, cannot do). A collapsed item hides
                // the fields below it.
                if (lvl.own_line && !(lvl.repeat && !lvl.compact)) || self.identity(rows, line, a) == NONE {
                    continue;
                }
                let first = previous
                    .is_none_or(|prev| (0..=a).any(|k| self.identity(rows, prev, k) != self.identity(rows, line, k)));
                if first || lvl.repeat {
                    let tag = if rows {
                        Tag::RowLabel(a as u8)
                    } else {
                        Tag::ColLabel(a as u8)
                    };
                    out.push((i, a, tag, self.label(rows, line, a)));
                }
            }
            let own = self.label(rows, line, level);
            if line.kind == LineKind::Subtotal {
                let item = match axis.levels[level].field {
                    Some(f) => label_text(p, f, &own, locale),
                    None => format::format_value(&own, "General", locale).text,
                };
                let word = match (&axis.levels[level].subtotal, line.func) {
                    (Sub::Custom(fs), k) if k > 0 => fs.get(k as usize - 1).map(|f| f.label(locale).to_string()),
                    _ => None,
                };
                let text = match (word, p.data_fields.get(line.data as usize)) {
                    (Some(w), _) => text::subtotal_label(&item, &w, locale),
                    // Below "Σ Values" the line already belongs to one
                    // value field; above it, the subtotal names its own.
                    (None, Some(d)) if axis.values_pos.is_some_and(|v| v > level) => format!("{item} {}", d.name),
                    _ => text::subtotal_label(&item, text::caption(Caption::Total, locale), locale),
                };
                let tag = if rows {
                    Tag::RowSubtotal(level as u8)
                } else {
                    Tag::ColSubtotal(level as u8)
                };
                out.push((i, level, tag, Value::from(text)));
            } else {
                let tag = if rows {
                    Tag::RowLabel(level as u8)
                } else {
                    Tag::ColLabel(level as u8)
                };
                out.push((i, level, tag, own));
            }
            previous = Some(line);
        }
        out
    }

    fn field_caption(&self, level: &Level) -> Value {
        match level.field {
            Some(f) => Value::text(self.p.field_caption(f)),
            None => Value::text(self.p.data_caption(self.locale)),
        }
    }

    fn report(mut self) -> Report {
        let p = self.p;
        let locale = self.locale;
        let pages = self.pages();
        let (page_rows, page_cols) = pages
            .iter()
            .fold((0, 0), |(r, c), cell| (r.max(cell.row + 1), c.max(cell.col + 2)));
        if p.row_fields.is_empty() && p.col_fields.is_empty() && self.nd == 0 {
            return Report {
                rows: PLACEHOLDER.0,
                cols: PLACEHOLDER.1,
                values: vec![Value::Empty; (PLACEHOLDER.0 * PLACEHOLDER.1) as usize],
                tags: vec![Tag::Blank; (PLACEHOLDER.0 * PLACEHOLDER.1) as usize],
                row_lines: vec![Line::HEADER; PLACEHOLDER.0 as usize],
                col_lines: vec![Line::HEADER; PLACEHOLDER.1 as usize],
                pages,
                page_rows,
                page_cols,
                placeholder: true,
                ..Report::default()
            };
        }
        self.filter_items();
        self.filter_values();
        let row_fields = self.rows.fields.clone();
        let col_fields = self.cols.fields.clone();
        (self.row_tree, self.row_index, self.rec_row) = self.build_tree(&row_fields);
        (self.col_tree, self.col_index, self.rec_col) = self.build_tree(&col_fields);
        self.add_empty_items(true);
        self.add_empty_items(false);
        self.add_calc_items(true);
        self.add_calc_items(false);
        self.aggregate();
        self.sort_tree(true);
        self.sort_tree(false);

        let row_lines = self.lines(true);
        let col_lines = self.lines(false);
        let has_col_fields = !self.cols.fields.is_empty();
        let label_cols = match self.rows.levels.last() {
            Some(l) => l.slot + 1,
            None => has_col_fields as u32,
        };
        let caption_rows = has_col_fields as u32;
        for (k, lvl) in self.cols.levels.iter_mut().enumerate() {
            lvl.slot = caption_rows + k as u32;
        }
        let header_rows = caption_rows + self.cols.levels.len().max(1) as u32;
        let rows = header_rows + row_lines.len() as u32;
        let cols = label_cols + col_lines.len() as u32;
        let mut values = vec![Value::Empty; (rows * cols) as usize];
        let mut tags = vec![Tag::Blank; (rows * cols) as usize];
        // Without "field captions and filter drop downs", the cells of the
        // captions are still there, empty.
        let headers = p.options.show_headers;
        let mut put = |r: u32, c: u32, tag: Tag, v: Value| {
            if r < rows && c < cols {
                let caption = matches!(
                    tag,
                    Tag::RowCaption | Tag::ColCaption | Tag::RowHeader(_) | Tag::ColHeader(_)
                );
                values[(r * cols + c) as usize] = if caption && !headers { Value::Empty } else { v };
                tags[(r * cols + c) as usize] = tag;
            }
        };
        let single = (self.nd == 1).then(|| Value::text(&p.data_fields[0].name));

        // The corner: captions of the value field and of the fields.
        if caption_rows == 1 {
            if label_cols > 0 {
                put(0, 0, Tag::DataCaption, single.clone().unwrap_or_default());
            }
            if self.cols.levels.first().is_some_and(|l| {
                l.field
                    .and_then(|f| p.fields.get(f))
                    .is_none_or(|pf| pf.layout == FieldLayout::Compact)
            }) {
                let text = p
                    .options
                    .col_header_caption
                    .clone()
                    .unwrap_or_else(|| text::caption(Caption::ColumnLabels, locale).to_string());
                put(0, label_cols, Tag::ColCaption, Value::from(text));
            } else {
                for (k, lvl) in self.cols.levels.iter().enumerate() {
                    put(
                        0,
                        label_cols + k as u32,
                        Tag::ColHeader(k as u8),
                        self.field_caption(lvl),
                    );
                }
            }
        }
        let last_header = header_rows - 1;
        for slot in 0..label_cols {
            let Some((k, first)) = self.rows.levels.iter().enumerate().find(|(_, l)| l.slot == slot) else {
                continue;
            };
            if first.compact {
                let text = if self.rows.levels.iter().any(|l| l.slot == slot && l.field.is_some()) {
                    p.options
                        .row_header_caption
                        .clone()
                        .unwrap_or_else(|| text::caption(Caption::RowLabels, locale).to_string())
                } else {
                    p.data_caption(locale)
                };
                put(last_header, slot, Tag::RowCaption, Value::from(text));
            } else {
                put(last_header, slot, Tag::RowHeader(k as u8), self.field_caption(first));
            }
        }

        // Labels of the lines.
        for (i, level, tag, v) in self.labels(false, &col_lines) {
            let row = match tag {
                Tag::ColGrand => caption_rows,
                _ => self.cols.levels[level].slot,
            };
            put(row, label_cols + i as u32, tag, v);
        }
        for (i, level, tag, v) in self.labels(true, &row_lines) {
            let col = match tag {
                Tag::RowGrand => 0,
                _ => self.rows.levels[level].slot,
            };
            put(header_rows + i as u32, col, tag, v);
        }
        if self.cols.levels.is_empty()
            && let Some(name) = &single
            && !col_lines.is_empty()
        {
            put(0, label_cols, Tag::DataCaption, name.clone());
        }
        if self.rows.levels.is_empty() && label_cols == 1 && !row_lines.is_empty() {
            match &single {
                Some(name) => {
                    // The value field names its row instead of the corner.
                    put(0, 0, Tag::Blank, Value::Empty);
                    put(header_rows, 0, Tag::DataCaption, name.clone());
                }
                None => put(
                    header_rows,
                    0,
                    Tag::TotalLabel,
                    Value::text(text::caption(Caption::Total, locale)),
                ),
            }
        }

        // Values.
        let opts = &p.options;
        // "For empty cells show": unchecked, Excel shows zeros.
        let missing = if !opts.show_missing {
            Value::Number(0.0)
        } else if opts.missing_caption.is_empty() {
            Value::Empty
        } else {
            match locale.parse_number(&opts.missing_caption) {
                Some(n) => Value::Number(n),
                None => Value::text(&opts.missing_caption),
            }
        };
        let custom =
            |axis: &Axis, line: &Line| match (axis.levels.get(line.level as usize).map(|l| &l.subtotal), line.func) {
                (Some(Sub::Custom(fs)), k) if k > 0 => fs.get(k as usize - 1).copied(),
                _ => None,
            };
        if self.nd > 0 {
            for (i, row) in row_lines.iter().enumerate() {
                if !row.values {
                    continue;
                }
                for (j, col) in col_lines.iter().enumerate() {
                    if !col.values {
                        continue;
                    }
                    let Some(d) = row.data().or(col.data()).or((self.nd == 1).then_some(0)) else {
                        continue;
                    };
                    let function = custom(&self.rows, row).or(custom(&self.cols, col));
                    let v = match self.shown(row.node, col.node, d, function) {
                        None => missing.clone(),
                        Some(Value::Error(_)) if opts.show_error => match opts.error_caption.is_empty() {
                            true => Value::Empty,
                            false => Value::text(&opts.error_caption),
                        },
                        Some(v) => v,
                    };
                    put(header_rows + i as u32, label_cols + j as u32, Tag::Value, v);
                }
            }
        }

        let header = |n: u32, lines: Vec<Line>| {
            let mut all = vec![Line::HEADER; n as usize];
            all.extend(lines);
            all
        };
        Report {
            rows,
            cols,
            header_rows,
            label_cols,
            caption_rows,
            values,
            tags,
            row_lines: header(header_rows, row_lines),
            col_lines: header(label_cols, col_lines),
            row_levels: self.rows.levels,
            col_levels: self.cols.levels,
            row_tree: self.row_tree,
            col_tree: self.col_tree,
            rec_row: self.rec_row,
            rec_col: self.rec_col,
            pages,
            page_rows,
            page_cols,
            placeholder: false,
        }
    }

    /// The filter fields: label and current selection, laid out down then
    /// over (or over then down), `page_wrap` fields per column (or row).
    fn pages(&self) -> Vec<PageCell> {
        let p = self.p;
        let wrap = p.options.page_wrap as usize;
        p.page_fields
            .iter()
            .enumerate()
            .filter(|(_, pf)| pf.field < p.fields.len())
            .map(|(k, pf)| {
                let (major, minor) = match k.checked_div(wrap) {
                    Some(major) => (major, k % wrap),
                    None => (0, k),
                };
                let (row, block) = if p.options.page_over_then_down {
                    (major, minor)
                } else {
                    (minor, major)
                };
                let hidden = p.fields[pf.field].items.iter().any(|it| it.hidden);
                let value = match pf.item {
                    Some(x) => self.item_value(pf.field, x),
                    None if hidden => Value::text(text::caption(Caption::MultipleItems, self.locale)),
                    None => Value::text(text::caption(Caption::All, self.locale)),
                };
                PageCell {
                    field: pf.field,
                    row: row as u32,
                    // Label, value and a column left empty between blocks.
                    col: block as u32 * 3,
                    value,
                }
            })
            .collect()
    }
}

/// A value filter at work: given the items of one group as (node, total),
/// the nodes to drop.
type Rejects = Box<dyn Fn(&[(u32, f64)]) -> Vec<u32>>;

/// The siblings a Top 10 filter rejects.
fn top10_rejects(sibs: &[(u32, f64)], top: bool, count: f64, by: Top10By) -> Vec<u32> {
    let mut order: Vec<(u32, f64)> = sibs.to_vec();
    order.sort_by(|a, b| {
        let o = a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal);
        if top { o.reverse() } else { o }
    });
    let total: f64 = order.iter().map(|x| x.1).sum();
    let mut kept = 0usize;
    match by {
        Top10By::Items => {
            kept = (count.max(0.0) as usize).min(order.len());
            // Items tied with the last one kept are kept too.
            while kept > 0 && kept < order.len() && order[kept].1 == order[kept - 1].1 {
                kept += 1;
            }
        }
        Top10By::Percent | Top10By::Sum => {
            let target = if by == Top10By::Percent {
                total * count / 100.0
            } else {
                count
            };
            let mut running = 0.0;
            while kept < order.len() && running < target {
                running += order[kept].1;
                kept += 1;
            }
        }
    }
    order[kept..].iter().map(|x| x.0).collect()
}

/// Compute the report of a PivotTable from its cache.
pub(crate) fn compute(p: &PivotTable, locale: &Locale) -> Report {
    Engine::new(p, locale).report()
}
