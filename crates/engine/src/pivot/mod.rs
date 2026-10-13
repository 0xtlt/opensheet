//! Pivot tables (`Sheet::pivots`).
//!
//! A [`PivotTable`] is a definition (source, fields in the four areas,
//! options, style), a [`PivotCache`] holding a copy of the source records,
//! and the [`Report`] computed from the two. As in Excel, the report is
//! written into the cells of the sheet as plain values and does not follow
//! the source until it is refreshed.
//!
//! The model follows the file format: the cache is `pivotCacheDefinition`
//! and `pivotCacheRecords` (shared by every PivotTable built on the same
//! source, as the same `Arc`), and the rest is `pivotTableDefinition`.
//!
//! Every command goes through [`Workbook::pivot_update`]: it changes a copy
//! of the definition, computes the new report, checks that it can be
//! written (no other PivotTable, table or merged cell in the way, and no
//! data unless the caller allows it), then takes the undo snapshot and
//! writes. The caller commits.

mod cache;
mod calc;
mod chart;
mod commands;
mod compute;
mod getdata;
mod slicing;
mod style;
#[cfg(test)]
mod tests;
mod text;

pub use cache::{CacheField, CalcItem, FieldGroup, GroupBy, GroupKind, PivotCache};
pub use calc::{CalcFormula, ListLabels};
pub use chart::{ChartButtonKind, PivotChartFields, PivotChartLink, chart_kind_allowed};
pub use commands::PivotDestination;
pub use compute::{Report, ReportLine, ReportLineKind};
pub use getdata::get_pivot_data;
pub use style::{PivotStyle, style_patch_at};
pub use text::{Caption, caption, data_field_name};

use crate::ast::{Expr, StructuralEdit};
use crate::format;
use crate::locale::Locale;
use crate::refs::{MAX_COLS, MAX_ROWS, Rect};
use crate::value::Value;
use crate::workbook::{Sheet, Workbook};
use compute::{LineKind, Tag};
use std::sync::Arc;

/// How a value field summarises its values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Aggregation {
    Sum,
    /// Number of non-empty values.
    Count,
    Average,
    Max,
    Min,
    Product,
    /// Number of numeric values.
    CountNums,
    StdDev,
    StdDevp,
    Var,
    Varp,
}

impl Aggregation {
    pub const ALL: [Aggregation; 11] = [
        Aggregation::Sum,
        Aggregation::Count,
        Aggregation::Average,
        Aggregation::Max,
        Aggregation::Min,
        Aggregation::Product,
        Aggregation::CountNums,
        Aggregation::StdDev,
        Aggregation::StdDevp,
        Aggregation::Var,
        Aggregation::Varp,
    ];
}

/// Where the records of a PivotTable come from (`worksheetSource`).
#[derive(Clone, Debug, PartialEq)]
pub enum PivotSource {
    /// A range whose first row holds the field names.
    Range { sheet: String, range: Rect },
    /// A table, or a defined name referring to a range.
    Name(String),
    /// The source range was deleted.
    Invalid,
}

impl PivotSource {
    /// The sheet and the range of the source, header row included.
    pub fn resolve(&self, wb: &Workbook) -> Result<(usize, Rect), PivotError> {
        match self {
            PivotSource::Range { sheet, range } => {
                let si = wb.sheet_index(sheet).ok_or(PivotError::InvalidSource)?;
                Ok((si, *range))
            }
            PivotSource::Name(name) => {
                if let Some((si, table)) = wb.table_by_name(name) {
                    if !table.header_row {
                        return Err(PivotError::InvalidSource);
                    }
                    let last = table.range.r1.saturating_sub(table.totals_row as u32);
                    return Ok((si, Rect::new(table.range.r0, table.range.c0, last, table.range.c1)));
                }
                match wb.defined_name(name) {
                    Some(Expr::Ref(r)) => {
                        let sheet = r.sheet.as_deref().ok_or(PivotError::InvalidSource)?;
                        let si = wb.sheet_index(sheet).ok_or(PivotError::InvalidSource)?;
                        Ok((si, r.rect()))
                    }
                    _ => Err(PivotError::InvalidSource),
                }
            }
            PivotSource::Invalid => Err(PivotError::InvalidSource),
        }
    }

    /// The source as the "Table/Range" box shows it: `Sheet1!$A$1:$D$10`
    /// or the name of the table.
    pub fn to_text(&self) -> String {
        match self {
            PivotSource::Range { sheet, range } => {
                let mut out = String::new();
                crate::ast::write_sheet_name(sheet, &mut out);
                let cell = |r: u32, c: u32| format!("${}${}", crate::refs::col_to_letters(c), r + 1);
                out.push('!');
                out.push_str(&cell(range.r0, range.c0));
                if !range.is_single() {
                    out.push(':');
                    out.push_str(&cell(range.r1, range.c1));
                }
                out
            }
            PivotSource::Name(name) => name.clone(),
            PivotSource::Invalid => "#REF!".to_string(),
        }
    }

    /// Read what the user typed in the "Table/Range" box: a range (on
    /// `sheet` unless it names another one), a table or a defined name.
    pub fn parse(text: &str, sheet: usize, wb: &Workbook) -> Option<PivotSource> {
        let text = text.trim().trim_start_matches('=');
        match crate::parser::parse_formula(text, wb.locale).ok()? {
            Expr::Ref(r) => {
                let name = match &r.sheet {
                    Some(s) => wb.sheets.get(wb.sheet_index(s)?)?.name.clone(),
                    None => wb.sheets.get(sheet)?.name.clone(),
                };
                Some(PivotSource::Range {
                    sheet: name,
                    range: r.rect(),
                })
            }
            Expr::Name(n) => {
                let source = PivotSource::Name(n.to_string());
                source.resolve(wb).ok().map(|_| source)
            }
            _ => None,
        }
    }
}

/// An entry of the row or column area: a field, or "Σ Values" (present
/// when there are several value fields).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxisField {
    Field(usize),
    Values,
}

impl AxisField {
    pub fn field(&self) -> Option<usize> {
        match self {
            AxisField::Field(f) => Some(*f),
            AxisField::Values => None,
        }
    }
}

/// The four areas of the PivotTable Fields pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Area {
    Filters,
    Columns,
    Rows,
    Values,
}

/// A field in the filter area (`pageField`).
#[derive(Clone, Debug, PartialEq)]
pub struct PageField {
    pub field: usize,
    /// The one item shown. `None` shows "(All)", or the items left visible
    /// in the field when several are selected.
    pub item: Option<u32>,
}

/// An item of a field, as the PivotTable shows it (`item`).
#[derive(Clone, Debug, PartialEq)]
pub struct PivotItem {
    /// Index of the item in the cache (`PivotCache::items`).
    pub x: u32,
    /// Unchecked in the field's list.
    pub hidden: bool,
    /// Its detail is not shown (`sd="0"`).
    pub collapsed: bool,
    /// A label typed over the item's.
    pub caption: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SortOrder {
    /// The order of `PivotField::items`.
    #[default]
    Manual,
    Ascending,
    Descending,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct FieldSort {
    pub order: SortOrder,
    /// Sort by the totals of this value field (index into `data_fields`)
    /// instead of by label.
    pub by_value: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Subtotals {
    /// One subtotal, with the function of each value field.
    #[default]
    Automatic,
    None,
    /// One subtotal line per function.
    Custom(Vec<Aggregation>),
}

/// The form of a row field ("Report Layout").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FieldLayout {
    /// The labels of the next field go below, in the same column.
    #[default]
    Compact,
    /// The labels of the next field go below, in the next column.
    Outline,
    /// The labels of the next field start on the same row.
    Tabular,
}

/// How a field of the cache is shown in this PivotTable (`pivotField`).
#[derive(Clone, Debug, PartialEq)]
pub struct PivotField {
    /// A name typed over the field's.
    pub caption: Option<String>,
    /// Every item of the field, in manual order.
    pub items: Vec<PivotItem>,
    pub sort: FieldSort,
    pub subtotals: Subtotals,
    /// Automatic subtotals go on the item's own row, above its detail
    /// (outline and compact form only).
    pub subtotal_top: bool,
    pub layout: FieldLayout,
    /// A blank row follows each item.
    pub blank_line: bool,
    /// The item's label is repeated on every row (outline, tabular).
    pub repeat_labels: bool,
    /// Items without data are shown.
    pub show_all: bool,
    /// Number format of the labels; a date field otherwise uses the format
    /// of its source column.
    pub num_fmt: Option<Arc<str>>,
}

impl Default for PivotField {
    fn default() -> PivotField {
        PivotField {
            caption: None,
            items: Vec::new(),
            sort: FieldSort::default(),
            subtotals: Subtotals::Automatic,
            subtotal_top: true,
            layout: FieldLayout::Compact,
            blank_line: false,
            repeat_labels: false,
            show_all: false,
            num_fmt: None,
        }
    }
}

/// The item a "Show Values As" calculation compares with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaseItem {
    Previous,
    Next,
    Item(u32),
}

/// "Show Values As" (`showDataAs`, with its base field and base item).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ShowAs {
    #[default]
    Normal,
    PercentOfTotal,
    PercentOfColumn,
    PercentOfRow,
    PercentOf {
        field: usize,
        item: BaseItem,
    },
    PercentOfParentRow,
    PercentOfParentColumn,
    PercentOfParent {
        field: usize,
    },
    Difference {
        field: usize,
        item: BaseItem,
    },
    PercentDifference {
        field: usize,
        item: BaseItem,
    },
    RunningTotal {
        field: usize,
    },
    PercentRunningTotal {
        field: usize,
    },
    /// 1 for the smallest value.
    RankAscending {
        field: usize,
    },
    /// 1 for the largest value.
    RankDescending {
        field: usize,
    },
    Index,
}

impl ShowAs {
    /// The field the calculation runs along, if it needs one.
    pub fn base_field(&self) -> Option<usize> {
        match *self {
            ShowAs::PercentOf { field, .. }
            | ShowAs::PercentOfParent { field }
            | ShowAs::Difference { field, .. }
            | ShowAs::PercentDifference { field, .. }
            | ShowAs::RunningTotal { field }
            | ShowAs::PercentRunningTotal { field }
            | ShowAs::RankAscending { field }
            | ShowAs::RankDescending { field } => Some(field),
            _ => None,
        }
    }

    /// True when the results are percentages: Excel then formats the values
    /// as `0.00%` unless the field has a number format of its own.
    pub fn is_percentage(&self) -> bool {
        matches!(
            self,
            ShowAs::PercentOfTotal
                | ShowAs::PercentOfColumn
                | ShowAs::PercentOfRow
                | ShowAs::PercentOf { .. }
                | ShowAs::PercentOfParentRow
                | ShowAs::PercentOfParentColumn
                | ShowAs::PercentOfParent { .. }
                | ShowAs::PercentDifference { .. }
                | ShowAs::PercentRunningTotal { .. }
        )
    }
}

/// A field in the values area (`dataField`).
#[derive(Clone, Debug, PartialEq)]
pub struct DataField {
    pub field: usize,
    /// "Sum of Sales", or the custom name.
    pub name: String,
    pub function: Aggregation,
    pub show_as: ShowAs,
    pub num_fmt: Option<Arc<str>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LabelFilter {
    /// `*` and `?` are wildcards.
    Equals(String),
    NotEquals(String),
    BeginsWith(String),
    NotBeginsWith(String),
    EndsWith(String),
    NotEndsWith(String),
    Contains(String),
    NotContains(String),
    GreaterThan(String),
    GreaterOrEqual(String),
    LessThan(String),
    LessOrEqual(String),
    Between(String, String),
    NotBetween(String, String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ValueFilter {
    Equals(f64),
    NotEquals(f64),
    GreaterThan(f64),
    GreaterOrEqual(f64),
    LessThan(f64),
    LessOrEqual(f64),
    Between(f64, f64),
    NotBetween(f64, f64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Top10By {
    Items,
    /// Items making up this percentage of the total.
    Percent,
    /// Items making up this sum.
    Sum,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FilterKind {
    Label(LabelFilter),
    /// Keep the items whose total of a value field passes the test.
    Value {
        data_field: usize,
        test: ValueFilter,
    },
    Top10 {
        data_field: usize,
        /// Top rather than bottom.
        top: bool,
        count: f64,
        by: Top10By,
    },
    /// Keep the records whose date lies between two days, both included
    /// (what a timeline sets). Unlike the others it applies wherever the
    /// field is, in the report or not.
    Date {
        start: f64,
        end: f64,
    },
}

/// A label or value filter on a row or column field (`filter`).
#[derive(Clone, Debug, PartialEq)]
pub struct PivotFilter {
    pub field: usize,
    pub kind: FilterKind,
}

/// PivotTable Options.
#[derive(Clone, Debug, PartialEq)]
pub struct PivotOptions {
    /// "Show grand totals for rows": the column at the right.
    pub row_grand_totals: bool,
    /// "Show grand totals for columns": the row at the bottom.
    pub col_grand_totals: bool,
    /// "For empty cells show" is checked; unchecked, empty cells show 0.
    pub show_missing: bool,
    pub missing_caption: String,
    /// "For error values show" is checked.
    pub show_error: bool,
    pub error_caption: String,
    /// Captions typed over "Grand Total", "Row Labels", "Column Labels"
    /// and "Values"; `None` uses the workbook's language.
    pub grand_total_caption: Option<String>,
    pub row_header_caption: Option<String>,
    pub col_header_caption: Option<String>,
    pub data_caption: Option<String>,
    /// Filter fields are laid out over, then down.
    pub page_over_then_down: bool,
    /// Filter fields per column (or per row); 0 = all in one.
    pub page_wrap: u32,
    /// "Display field captions and filter drop downs".
    pub show_headers: bool,
    /// "Show expand/collapse buttons".
    pub show_drill: bool,
}

impl Default for PivotOptions {
    fn default() -> PivotOptions {
        PivotOptions {
            row_grand_totals: true,
            col_grand_totals: true,
            show_missing: true,
            missing_caption: String::new(),
            show_error: false,
            error_caption: String::new(),
            grand_total_caption: None,
            row_header_caption: None,
            col_header_caption: None,
            data_caption: None,
            page_over_then_down: false,
            page_wrap: 0,
            show_headers: true,
            show_drill: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PivotTable {
    /// Unique in the workbook.
    pub id: u32,
    /// Unique on its sheet, compared without case.
    pub name: String,
    pub source: PivotSource,
    pub cache: Arc<PivotCache>,
    /// Top-left cell of the report. Filter fields go above it, one empty
    /// row in between; the report moves down when they need the room.
    pub anchor: (u32, u32),
    /// One per field of the cache.
    pub fields: Vec<PivotField>,
    pub row_fields: Vec<AxisField>,
    pub col_fields: Vec<AxisField>,
    pub page_fields: Vec<PageField>,
    pub data_fields: Vec<DataField>,
    /// Label and value filters, applied in order.
    pub filters: Vec<PivotFilter>,
    /// The fields a slicer filters: their unchecked items are left out
    /// even while the field is in none of the areas.
    pub sliced: Vec<usize>,
    /// The PivotCharts: charts whose series are made from the report.
    pub charts: Vec<PivotChartLink>,
    pub options: PivotOptions,
    pub style: PivotStyle,
    /// The cells the report occupies since it was last written
    /// (`location`), and those of the filter fields above it.
    pub location: Option<Rect>,
    pub page_location: Option<Rect>,
    /// The report as last written.
    pub report: Arc<Report>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PivotError {
    NoSuchPivot,
    NoSuchField,
    /// The source is not a range with a header row and at least one record
    /// ("Data source reference is not valid").
    InvalidSource,
    /// A column of the source has no header ("The PivotTable field name is
    /// not valid").
    EmptyFieldName,
    /// "A PivotTable report cannot overlap another PivotTable report."
    OverlapsPivot(String),
    /// "A PivotTable report cannot overlap a table."
    OverlapsTable,
    /// The report would cover merged cells.
    OverlapsMerged,
    /// The report would cover cells holding data: ask the user ("There's
    /// already data here. Do you want to replace it?") and call again
    /// allowing it. The range is the one the report needs.
    WouldOverwrite(Rect),
    /// The report does not fit on the sheet.
    TooLarge,
    /// "Cannot group that selection."
    CannotGroup,
    CannotUngroup,
    /// The name is empty or already used on the sheet.
    InvalidName,
    /// The cell is not a value of a PivotTable.
    NotAValue,
    /// The formula of a calculated field or item cannot be read.
    BadFormula,
    /// "References, names, and arrays are not supported in PivotTable
    /// formulas."
    FormulaReference,
    /// The formula uses a function whose result varies (RAND, NOW) or
    /// that reads a PivotTable.
    FormulaFunction,
    /// The formula computes from itself.
    FormulaCircular,
    /// Another field or item has the name.
    FormulaNameTaken,
    /// Calculated items do not go with averages, standard deviations and
    /// variances, with custom subtotals, with a field that is twice in
    /// the values, with a grouped field, nor in the filter area.
    ItemsWithAverages,
    ItemsWithCustomSubtotals,
    ItemsWithRepeatedField,
    ItemsInGroupedField,
    ItemsInPageField,
    /// "PivotChart reports cannot use XY (Scatter), Bubble, or Stock
    /// chart types."
    ChartKind,
}

/// The part of a PivotTable a cell belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PivotPart {
    /// Inside the frame of a PivotTable that has no field yet.
    Placeholder,
    /// The name of a filter field.
    FilterLabel,
    /// The selection of a filter field (it carries the drop-down).
    FilterValue,
    /// An empty cell of the report: corner, blank line, gap.
    Blank,
    /// The cell naming the value field ("Sum of Sales").
    DataCaption,
    /// "Row Labels" (compact form): its drop-down serves every row field
    /// sharing the column.
    RowHeaderCaption,
    /// "Column Labels".
    ColumnHeaderCaption,
    /// The name of a field above its labels (outline and tabular form,
    /// column fields); "Values" for the value fields.
    FieldHeader,
    RowLabel,
    ColumnLabel,
    RowSubtotalLabel,
    ColumnSubtotalLabel,
    RowGrandTotalLabel,
    ColumnGrandTotalLabel,
    Value,
}

/// The button Excel draws in a cell of a PivotTable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PivotButton {
    /// The drop-down of a field header or of a filter field; `filtered`
    /// when one of its fields hides items.
    Filter { filtered: bool },
    /// "+": the item is collapsed.
    Expand,
    /// "-".
    Collapse,
}

/// What a cell of a PivotTable stands for.
#[derive(Clone, Debug, PartialEq)]
pub struct PivotCell {
    /// `PivotTable::id`.
    pub pivot: u32,
    pub part: PivotPart,
    /// The field of a label, a field header or a filter cell.
    pub field: Option<usize>,
    /// The item of a label (index into the cache items of `field`).
    pub item: Option<u32>,
    /// The fields a header's drop-down serves.
    pub fields: Vec<usize>,
    /// The row items the cell lies under, outermost first: (field, item).
    pub row_items: Vec<(usize, u32)>,
    /// The column items the cell lies under.
    pub col_items: Vec<(usize, u32)>,
    /// The value field of a value, or of a label of "Σ Values".
    pub data_field: Option<usize>,
    /// What the cell's row is, below the headers.
    pub row_kind: Option<ReportLineKind>,
    /// What the cell's column is, right of the labels.
    pub col_kind: Option<ReportLineKind>,
    pub button: Option<PivotButton>,
    /// Indentation of a label in compact form.
    pub indent: u8,
}

fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

impl PivotTable {
    /// A PivotTable without any field in its areas.
    pub fn new(id: u32, name: String, source: PivotSource, cache: Arc<PivotCache>, anchor: (u32, u32)) -> PivotTable {
        let mut p = PivotTable {
            id,
            name,
            source,
            cache,
            anchor,
            fields: Vec::new(),
            row_fields: Vec::new(),
            col_fields: Vec::new(),
            page_fields: Vec::new(),
            data_fields: Vec::new(),
            filters: Vec::new(),
            sliced: Vec::new(),
            charts: Vec::new(),
            options: PivotOptions::default(),
            style: PivotStyle::default(),
            location: None,
            page_location: None,
            report: Arc::new(Report::default()),
        };
        p.normalize();
        p
    }

    // ---- reading -----------------------------------------------------------

    /// Index of the field with this name or caption, compared without case.
    pub fn field_index(&self, name: &str) -> Option<usize> {
        (0..self.fields.len())
            .find(|&f| same_name(self.field_caption(f), name))
            .or_else(|| self.cache.field_index(name))
    }

    /// The name the PivotTable shows for a field.
    pub fn field_caption(&self, field: usize) -> &str {
        match (
            self.fields.get(field).and_then(|f| f.caption.as_deref()),
            self.cache.fields.get(field),
        ) {
            (Some(c), _) => c,
            (None, Some(f)) => &f.name,
            (None, None) => "",
        }
    }

    /// The label of an item: its custom caption, its value (a number, a
    /// date serial, text...) or "(blank)".
    pub fn item_label(&self, field: usize, item: u32, locale: &Locale) -> Value {
        let custom = self
            .fields
            .get(field)
            .and_then(|f| f.items.iter().find(|i| i.x == item))
            .and_then(|i| i.caption.as_ref());
        match (custom, self.cache.items(field).get(item as usize)) {
            (Some(c), _) => Value::text(c),
            (None, Some(Value::Empty) | None) => Value::text(caption(Caption::Blank, locale)),
            (None, Some(v)) => v.clone(),
        }
    }

    /// The label of an item as text, formatted as the report shows it.
    pub fn item_text(&self, field: usize, item: u32, locale: &Locale) -> String {
        compute::label_text(self, field, &self.item_label(field, item, locale), locale)
    }

    /// The number format of the labels of a field, when it has one.
    pub fn label_format(&self, field: usize) -> Option<Arc<str>> {
        if let Some(f) = self.fields.get(field).and_then(|f| f.num_fmt.clone()) {
            return Some(f);
        }
        let cf = self.cache.fields.get(field)?;
        (cf.group.is_none() && format::is_date_format(&cf.num_fmt)).then(|| cf.num_fmt.clone())
    }

    /// The number format of the values of a value field: its own, or
    /// `0.00%` for a calculation that yields percentages.
    pub fn data_format(&self, data_field: usize) -> Option<Arc<str>> {
        let d = self.data_fields.get(data_field)?;
        match &d.num_fmt {
            Some(f) => Some(f.clone()),
            None if d.show_as.is_percentage() => Some(Arc::from("0.00%")),
            None => None,
        }
    }

    /// The caption of "Σ Values" ("Values").
    pub fn data_caption(&self, locale: &Locale) -> String {
        self.options
            .data_caption
            .clone()
            .unwrap_or_else(|| caption(Caption::Values, locale).to_string())
    }

    /// The area a field sits in, if it is a filter, row or column field.
    pub fn axis_of(&self, field: usize) -> Option<Area> {
        if self.row_fields.contains(&AxisField::Field(field)) {
            Some(Area::Rows)
        } else if self.col_fields.contains(&AxisField::Field(field)) {
            Some(Area::Columns)
        } else if self.page_fields.iter().any(|p| p.field == field) {
            Some(Area::Filters)
        } else {
            None
        }
    }

    /// True when the field is in one of the four areas (its box is checked
    /// in the field list).
    pub fn is_used(&self, field: usize) -> bool {
        self.axis_of(field).is_some() || self.data_fields.iter().any(|d| d.field == field)
    }

    /// The items of a field in the order of its filter list: by label,
    /// ascending or descending, or the manual order.
    pub fn items_in_order(&self, field: usize) -> Vec<u32> {
        let Some(f) = self.fields.get(field) else {
            return Vec::new();
        };
        match f.sort.order {
            SortOrder::Manual => f.items.iter().map(|it| it.x).collect(),
            SortOrder::Ascending => compute::ascending_items(self, field),
            SortOrder::Descending => compute::ascending_items(self, field).into_iter().rev().collect(),
        }
    }

    /// True when the field hides items or has a label or value filter.
    pub fn is_filtered(&self, field: usize) -> bool {
        self.fields.get(field).is_some_and(|f| f.items.iter().any(|i| i.hidden))
            || self.filters.iter().any(|f| f.field == field)
            || self.page_fields.iter().any(|p| p.field == field && p.item.is_some())
    }

    /// The area Excel puts a field in when its box is checked in the field
    /// list: numbers go to the values, everything else to the rows.
    pub fn default_area(&self, field: usize) -> Area {
        if self.cache.is_numeric(field) && !self.cache.is_date(field) {
            Area::Values
        } else {
            Area::Rows
        }
    }

    /// The cells the PivotTable occupies: the report and the filter block.
    pub fn footprint(&self) -> impl Iterator<Item = Rect> + use<> {
        [self.location, self.page_location].into_iter().flatten()
    }

    pub fn contains(&self, row: u32, col: u32) -> bool {
        self.footprint().any(|r| r.contains(row, col))
    }

    // ---- consistency -------------------------------------------------------

    fn default_field(&self, field: usize) -> PivotField {
        // Groups made by hand start in the order they were made in, the
        // items of any other field in ascending order.
        let by_hand = matches!(
            self.cache.fields[field].group,
            Some(FieldGroup {
                kind: GroupKind::Discrete,
                ..
            })
        );
        let order = if by_hand {
            (0..self.cache.items(field).len() as u32).collect()
        } else {
            compute::ascending_items(self, field)
        };
        PivotField {
            items: order
                .into_iter()
                .map(|x| PivotItem {
                    x,
                    hidden: false,
                    collapsed: false,
                    caption: None,
                })
                .collect(),
            ..PivotField::default()
        }
    }

    /// Bring the definition in line with its cache and with itself: one
    /// field per cache field, every item listed once, no reference to a
    /// field that does not exist, a field in one area only, and "Σ Values"
    /// present exactly when there are several value fields.
    pub fn normalize(&mut self) {
        let nf = self.cache.fields.len();
        self.fields.truncate(nf);
        for f in self.fields.len()..nf {
            let field = self.default_field(f);
            self.fields.push(field);
        }
        for f in 0..nf {
            let count = self.cache.items(f).len();
            let mut seen = vec![false; count];
            self.fields[f].items.retain(|it| match seen.get_mut(it.x as usize) {
                Some(s) if !*s => {
                    *s = true;
                    true
                }
                _ => false,
            });
            if seen.iter().any(|s| !s) {
                // New items go to the end, as Excel does on refresh.
                for x in compute::ascending_items(self, f) {
                    if !seen[x as usize] {
                        self.fields[f].items.push(PivotItem {
                            x,
                            hidden: false,
                            collapsed: false,
                            caption: None,
                        });
                    }
                }
            }
        }
        let mut used = vec![false; nf];
        // Calculated fields are value fields only.
        let calculated: Vec<bool> = self.cache.fields.iter().map(|f| f.formula.is_some()).collect();
        let mut keep_axis = |list: &mut Vec<AxisField>| {
            let mut values = false;
            list.retain(|a| match a {
                AxisField::Field(f) if *f < nf && !used[*f] && !calculated[*f] => {
                    used[*f] = true;
                    true
                }
                AxisField::Values if !values => {
                    values = true;
                    true
                }
                _ => false,
            });
        };
        keep_axis(&mut self.row_fields);
        keep_axis(&mut self.col_fields);
        self.page_fields.retain(|p| {
            let ok = p.field < nf && !used[p.field] && !calculated[p.field];
            if ok {
                used[p.field] = true;
            }
            ok
        });
        for p in &mut self.page_fields {
            if p.item.is_some_and(|x| x as usize >= self.cache.items(p.field).len()) {
                p.item = None;
            }
        }
        self.data_fields.retain(|d| d.field < nf);
        let on_axis: Vec<usize> = self
            .row_fields
            .iter()
            .chain(&self.col_fields)
            .filter_map(|a| a.field())
            .collect();
        for d in &mut self.data_fields {
            if d.show_as.base_field().is_some_and(|f| !on_axis.contains(&f)) {
                d.show_as = ShowAs::Normal;
            }
        }
        let nd = self.data_fields.len();
        self.filters.retain(|f| {
            f.field < nf
                && match f.kind {
                    FilterKind::Label(_) | FilterKind::Date { .. } => true,
                    FilterKind::Value { data_field, .. } | FilterKind::Top10 { data_field, .. } => data_field < nd,
                }
        });
        self.sliced.retain(|f| *f < nf);
        self.sliced.sort_unstable();
        self.sliced.dedup();
        for f in &mut self.fields {
            if f.sort.by_value.is_some_and(|d| d >= nd) {
                f.sort.by_value = None;
            }
        }
        let in_rows = self.row_fields.contains(&AxisField::Values);
        let in_cols = self.col_fields.contains(&AxisField::Values);
        if nd < 2 {
            self.row_fields.retain(|a| *a != AxisField::Values);
            self.col_fields.retain(|a| *a != AxisField::Values);
        } else if in_rows && in_cols {
            self.col_fields.retain(|a| *a != AxisField::Values);
        } else if !in_rows && !in_cols {
            self.col_fields.push(AxisField::Values);
        }
    }

    /// Move the definition onto another cache: fields are matched by name
    /// and items by value, so that hidden and collapsed items, captions and
    /// the manual order survive a refresh. Fields that are gone leave the
    /// areas.
    pub fn rebind(&mut self, cache: Arc<PivotCache>) {
        let old = std::mem::replace(&mut self.cache, cache);
        let new = self.cache.clone();
        let map: Vec<Option<usize>> = old.fields.iter().map(|f| new.field_index(&f.name)).collect();
        let field = |f: usize| map.get(f).copied().flatten();
        let mut fields: Vec<Option<PivotField>> = vec![None; new.fields.len()];
        for (k, pf) in std::mem::take(&mut self.fields).into_iter().enumerate() {
            let Some(n) = field(k) else {
                continue;
            };
            let mut pf = pf;
            let index: std::collections::HashMap<cache::ItemKey, u32> = new
                .items(n)
                .iter()
                .enumerate()
                .map(|(x, v)| (cache::item_key(v), x as u32))
                .collect();
            let old_items = old.items(k);
            let by_range = new.fields[n]
                .group
                .as_ref()
                .is_some_and(|g| g.kind != GroupKind::Discrete);
            let mut items: Vec<PivotItem> = pf
                .items
                .drain(..)
                .filter_map(|mut it| {
                    it.x = *index.get(&cache::item_key(old_items.get(it.x as usize)?))?;
                    Some(it)
                })
                .collect();
            if by_range {
                // Periods and intervals stay in their natural order.
                items.sort_by_key(|it| it.x);
            } else if new.fields[n].group.is_some() {
                // Hand-made groups: an item that appears (a group, or the
                // members of a dissolved one) goes where the cache has it.
                for x in 0..new.items(n).len() as u32 {
                    if items.iter().any(|it| it.x == x) {
                        continue;
                    }
                    let after = x
                        .checked_sub(1)
                        .and_then(|prev| items.iter().position(|it| it.x == prev));
                    let item = PivotItem {
                        x,
                        hidden: false,
                        collapsed: false,
                        caption: None,
                    };
                    items.insert(after.map_or(0, |k| k + 1), item);
                }
            }
            pf.items = items;
            fields[n] = Some(pf);
        }
        self.fields = Vec::with_capacity(fields.len());
        for (k, pf) in fields.into_iter().enumerate() {
            let pf = pf.unwrap_or_else(|| self.default_field(k));
            self.fields.push(pf);
        }
        let axis = |list: &mut Vec<AxisField>| {
            *list = list
                .iter()
                .filter_map(|a| match a {
                    AxisField::Field(f) => field(*f).map(AxisField::Field),
                    AxisField::Values => Some(AxisField::Values),
                })
                .collect();
        };
        axis(&mut self.row_fields);
        axis(&mut self.col_fields);
        // A filter field keeps its selected item when it still exists.
        let old_pages = std::mem::take(&mut self.page_fields);
        for p in old_pages {
            let Some(n) = field(p.field) else {
                continue;
            };
            let item = p.item.and_then(|x| {
                let key = cache::item_key(old.items(p.field).get(x as usize)?);
                new.items(n)
                    .iter()
                    .position(|v| cache::item_key(v) == key)
                    .map(|i| i as u32)
            });
            self.page_fields.push(PageField { field: n, item });
        }
        // Value fields that are gone take their place in the list with
        // them: what refers to the others by position must follow.
        let mut data_map: Vec<Option<usize>> = Vec::new();
        let mut data_fields = Vec::new();
        for mut d in std::mem::take(&mut self.data_fields) {
            match field(d.field) {
                Some(n) => {
                    d.field = n;
                    d.show_as = match d.show_as {
                        ShowAs::PercentOf { field: f, item } => field(f).map(|f| ShowAs::PercentOf { field: f, item }),
                        ShowAs::PercentOfParent { field: f } => field(f).map(|f| ShowAs::PercentOfParent { field: f }),
                        ShowAs::Difference { field: f, item } => {
                            field(f).map(|f| ShowAs::Difference { field: f, item })
                        }
                        ShowAs::PercentDifference { field: f, item } => {
                            field(f).map(|f| ShowAs::PercentDifference { field: f, item })
                        }
                        ShowAs::RunningTotal { field: f } => field(f).map(|f| ShowAs::RunningTotal { field: f }),
                        ShowAs::PercentRunningTotal { field: f } => {
                            field(f).map(|f| ShowAs::PercentRunningTotal { field: f })
                        }
                        ShowAs::RankAscending { field: f } => field(f).map(|f| ShowAs::RankAscending { field: f }),
                        ShowAs::RankDescending { field: f } => field(f).map(|f| ShowAs::RankDescending { field: f }),
                        other => Some(other),
                    }
                    .unwrap_or(ShowAs::Normal);
                    data_map.push(Some(data_fields.len()));
                    data_fields.push(d);
                }
                None => data_map.push(None),
            }
        }
        self.data_fields = data_fields;
        let data = |d: usize| data_map.get(d).copied().flatten();
        self.filters = std::mem::take(&mut self.filters)
            .into_iter()
            .filter_map(|mut f| {
                f.field = field(f.field)?;
                match &mut f.kind {
                    FilterKind::Label(_) | FilterKind::Date { .. } => {}
                    FilterKind::Value { data_field, .. } | FilterKind::Top10 { data_field, .. } => {
                        *data_field = data(*data_field)?
                    }
                }
                Some(f)
            })
            .collect();
        self.sliced = self.sliced.iter().filter_map(|f| field(*f)).collect();
        for f in &mut self.fields {
            f.sort.by_value = f.sort.by_value.and_then(data);
        }
        self.normalize();
    }

    // ---- the areas ---------------------------------------------------------

    /// Take a field out of the filter, row and column areas.
    pub fn remove_field(&mut self, field: usize) {
        self.row_fields.retain(|a| *a != AxisField::Field(field));
        self.col_fields.retain(|a| *a != AxisField::Field(field));
        self.page_fields.retain(|p| p.field != field);
        // What a timeline set stays: it filters the field wherever it is.
        self.filters
            .retain(|f| f.field != field || matches!(f.kind, FilterKind::Date { .. }));
        self.normalize();
    }

    /// Put a field in an area, at `position` (at the end when `None`). In
    /// the values area this adds a value field, summed when the field is
    /// numeric and counted otherwise; a field can be there several times.
    /// In the other areas the field moves there from where it was.
    pub fn add_field(&mut self, field: usize, area: Area, position: Option<usize>, locale: &Locale) {
        if field >= self.fields.len() {
            return;
        }
        // A calculated field has no items: it is only ever a value field.
        if area != Area::Values && self.cache.fields[field].formula.is_some() {
            return;
        }
        if area == Area::Values {
            let function = if self.cache.is_numeric(self.cache.database_field(field)) {
                Aggregation::Sum
            } else {
                Aggregation::Count
            };
            let name = self.unique_data_name(function, field, None, locale);
            let at = position.unwrap_or(usize::MAX).min(self.data_fields.len());
            self.data_fields.insert(
                at,
                DataField {
                    field,
                    name,
                    function,
                    show_as: ShowAs::Normal,
                    num_fmt: None,
                },
            );
            // References by position follow the value fields pushed down.
            self.shift_data_refs(|d| if d >= at { d + 1 } else { d });
        } else {
            self.row_fields.retain(|a| *a != AxisField::Field(field));
            self.col_fields.retain(|a| *a != AxisField::Field(field));
            self.page_fields.retain(|p| p.field != field);
            match area {
                Area::Rows => {
                    let at = position.unwrap_or(usize::MAX).min(self.row_fields.len());
                    self.row_fields.insert(at, AxisField::Field(field));
                }
                Area::Columns => {
                    let at = position.unwrap_or(usize::MAX).min(self.col_fields.len());
                    self.col_fields.insert(at, AxisField::Field(field));
                }
                _ => {
                    let at = position.unwrap_or(usize::MAX).min(self.page_fields.len());
                    self.page_fields.insert(at, PageField { field, item: None });
                    // Label and value filters only apply on an axis.
                    self.filters
                        .retain(|f| f.field != field || matches!(f.kind, FilterKind::Date { .. }));
                }
            }
        }
        self.normalize();
    }

    /// Remove a value field.
    pub fn remove_data_field(&mut self, index: usize) {
        if index >= self.data_fields.len() {
            return;
        }
        self.data_fields.remove(index);
        self.filters.retain(|f| match f.kind {
            FilterKind::Value { data_field, .. } | FilterKind::Top10 { data_field, .. } => data_field != index,
            FilterKind::Label(_) | FilterKind::Date { .. } => true,
        });
        for f in &mut self.fields {
            if f.sort.by_value == Some(index) {
                f.sort = FieldSort::default();
            }
        }
        self.shift_data_refs(|d| if d > index { d - 1 } else { d });
        self.normalize();
    }

    fn shift_data_refs(&mut self, shift: impl Fn(usize) -> usize) {
        for f in &mut self.filters {
            if let FilterKind::Value { data_field, .. } | FilterKind::Top10 { data_field, .. } = &mut f.kind {
                *data_field = shift(*data_field);
            }
        }
        for f in &mut self.fields {
            f.sort.by_value = f.sort.by_value.map(&shift);
        }
    }

    /// Move the entry at `index` of an area to `position` of another (or of
    /// the same) area, as dragging it in the field list does. An entry of
    /// the values area dropped elsewhere becomes a field there; a field
    /// dropped on the values area becomes a value field; "Σ Values" only
    /// moves between rows and columns. Returns false when nothing moves.
    pub fn move_entry(&mut self, from: Area, index: usize, to: Area, position: usize, locale: &Locale) -> bool {
        let axis = |p: &PivotTable, area: Area| match area {
            Area::Rows => p.row_fields.get(index).copied(),
            Area::Columns => p.col_fields.get(index).copied(),
            Area::Filters => p.page_fields.get(index).map(|pf| AxisField::Field(pf.field)),
            Area::Values => None,
        };
        if from == Area::Values {
            if index >= self.data_fields.len() {
                return false;
            }
            if to == Area::Values {
                let d = self.data_fields.remove(index);
                let at = position.min(self.data_fields.len());
                self.data_fields.insert(at, d);
                let moved = |d: usize| {
                    if d == index {
                        at
                    } else {
                        let without = if d > index { d - 1 } else { d };
                        if without >= at { without + 1 } else { without }
                    }
                };
                self.shift_data_refs(moved);
                self.normalize();
                return true;
            }
            let field = self.data_fields[index].field;
            self.remove_data_field(index);
            self.add_field(field, to, Some(position), locale);
            return true;
        }
        match axis(self, from) {
            None => false,
            Some(AxisField::Values) => {
                if !matches!(to, Area::Rows | Area::Columns) {
                    return false;
                }
                self.row_fields.retain(|a| *a != AxisField::Values);
                self.col_fields.retain(|a| *a != AxisField::Values);
                let list = if to == Area::Rows {
                    &mut self.row_fields
                } else {
                    &mut self.col_fields
                };
                list.insert(position.min(list.len()), AxisField::Values);
                self.normalize();
                true
            }
            Some(AxisField::Field(field)) => {
                if to == Area::Values {
                    self.remove_field(field);
                }
                self.add_field(field, to, Some(position), locale);
                true
            }
        }
    }

    /// Remove every field from the areas and every filter ("Clear All").
    pub fn clear(&mut self) {
        self.row_fields.clear();
        self.col_fields.clear();
        self.page_fields.clear();
        self.data_fields.clear();
        self.filters.clear();
        for f in &mut self.fields {
            f.sort = FieldSort::default();
            for it in &mut f.items {
                it.hidden = false;
                it.collapsed = false;
            }
        }
    }

    // ---- value fields ------------------------------------------------------

    /// "Sum of Sales", followed by a number when the name is taken
    /// ("Sum of Sales2"). `except` is the value field being renamed.
    fn unique_data_name(&self, function: Aggregation, field: usize, except: Option<usize>, locale: &Locale) -> String {
        let base = data_field_name(function, self.field_caption(field), locale);
        let taken = |n: &str| {
            self.data_fields
                .iter()
                .enumerate()
                .any(|(k, d)| Some(k) != except && same_name(&d.name, n))
                || (0..self.fields.len()).any(|f| same_name(self.field_caption(f), n))
        };
        if !taken(&base) {
            return base;
        }
        (2..).map(|k| format!("{base}{k}")).find(|n| !taken(n)).unwrap_or(base)
    }

    /// Change the summary function of a value field. Its name follows
    /// ("Sum of Sales" becomes "Count of Sales") unless it was customised.
    pub fn set_function(&mut self, data_field: usize, function: Aggregation, locale: &Locale) {
        let Some(d) = self.data_fields.get(data_field) else {
            return;
        };
        let field = d.field;
        let generated = self.unique_data_name(d.function, field, Some(data_field), locale);
        let rename = same_name(&d.name, &generated);
        let name = self.unique_data_name(function, field, Some(data_field), locale);
        let d = &mut self.data_fields[data_field];
        d.function = function;
        if rename {
            d.name = name;
        }
    }

    /// Give a value field a custom name. Refused (false) when the name is
    /// empty or is the name of a field or of another value field.
    pub fn set_data_name(&mut self, data_field: usize, name: &str) -> bool {
        let name = name.trim();
        let taken = self
            .data_fields
            .iter()
            .enumerate()
            .any(|(k, d)| k != data_field && same_name(&d.name, name))
            || (0..self.fields.len()).any(|f| same_name(self.field_caption(f), name));
        match self.data_fields.get_mut(data_field) {
            Some(d) if !name.is_empty() && !taken => {
                d.name = name.to_string();
                true
            }
            _ => false,
        }
    }

    /// Give a field a custom name, or its own back with an empty one.
    /// Refused (false) when another field or a value field has the name.
    pub fn set_field_caption(&mut self, field: usize, name: &str) -> bool {
        let name = name.trim();
        let taken = (0..self.fields.len()).any(|f| f != field && same_name(self.field_caption(f), name))
            || self
                .cache
                .fields
                .iter()
                .enumerate()
                .any(|(f, cf)| f != field && same_name(&cf.name, name))
            || self.data_fields.iter().any(|d| same_name(&d.name, name));
        match self.fields.get_mut(field) {
            Some(f) if !taken => {
                f.caption = (!name.is_empty()).then(|| name.to_string());
                true
            }
            _ => false,
        }
    }

    // ---- items -------------------------------------------------------------

    /// Type a label over an item's (an empty one restores it). Refused
    /// (false) when another item of the field shows that label.
    pub fn set_item_caption(&mut self, field: usize, item: u32, label: &str, locale: &Locale) -> bool {
        let label = label.trim();
        let count = self.cache.items(field).len() as u32;
        if (0..count).any(|x| x != item && same_name(&self.item_text(field, x, locale), label)) {
            return false;
        }
        match self.item_mut(field, item) {
            Some(it) => {
                it.caption = (!label.is_empty()).then(|| label.to_string());
                true
            }
            None => false,
        }
    }

    fn item_mut(&mut self, field: usize, item: u32) -> Option<&mut PivotItem> {
        self.fields.get_mut(field)?.items.iter_mut().find(|i| i.x == item)
    }

    /// Check or uncheck an item in the field's list. Choosing items by hand
    /// replaces the selection of a single item in a filter field.
    pub fn set_item_hidden(&mut self, field: usize, item: u32, hidden: bool) {
        if let Some(it) = self.item_mut(field, item) {
            it.hidden = hidden;
        }
        for p in &mut self.page_fields {
            if p.field == field {
                p.item = None;
            }
        }
    }

    /// Show exactly these items of a field.
    pub fn set_visible_items(&mut self, field: usize, visible: &[u32]) {
        if let Some(f) = self.fields.get_mut(field) {
            for it in &mut f.items {
                it.hidden = !visible.contains(&it.x);
            }
        }
        for p in &mut self.page_fields {
            if p.field == field {
                p.item = None;
            }
        }
    }

    /// Select one item of a filter field, or "(All)" with `None`.
    pub fn set_page_item(&mut self, field: usize, item: Option<u32>) {
        if let Some(f) = self.fields.get_mut(field) {
            for it in &mut f.items {
                it.hidden = false;
            }
        }
        for p in &mut self.page_fields {
            if p.field == field {
                p.item = item;
            }
        }
    }

    /// Remove the item, label and value filters of a field ("Clear Filter
    /// From").
    pub fn clear_filters(&mut self, field: usize) {
        self.set_page_item(field, None);
        self.filters.retain(|f| f.field != field);
    }

    /// Collapse or expand one item.
    pub fn set_collapsed(&mut self, field: usize, item: u32, collapsed: bool) {
        if let Some(it) = self.item_mut(field, item) {
            it.collapsed = collapsed;
        }
    }

    /// Collapse or expand every item of a field ("Collapse Entire Field").
    pub fn set_field_collapsed(&mut self, field: usize, collapsed: bool) {
        if let Some(f) = self.fields.get_mut(field) {
            for it in &mut f.items {
                it.collapsed = collapsed;
            }
        }
    }

    /// Move an item to `position` among the items of its field; the field
    /// becomes sorted by hand.
    pub fn move_item(&mut self, field: usize, item: u32, position: usize) {
        let Some(f) = self.fields.get_mut(field) else {
            return;
        };
        let Some(at) = f.items.iter().position(|i| i.x == item) else {
            return;
        };
        let it = f.items.remove(at);
        f.items.insert(position.min(f.items.len()), it);
        f.sort = FieldSort::default();
    }

    /// Sort a field by its labels, or by the totals of a value field.
    pub fn sort_field(&mut self, field: usize, order: SortOrder, by_value: Option<usize>) {
        if let Some(f) = self.fields.get_mut(field) {
            f.sort = FieldSort { order, by_value };
        }
    }

    /// Set, replace or remove (`None`) the label filter of a field.
    pub fn set_label_filter(&mut self, field: usize, filter: Option<LabelFilter>) {
        self.filters
            .retain(|f| f.field != field || !matches!(f.kind, FilterKind::Label(_)));
        if let Some(l) = filter {
            self.filters.push(PivotFilter {
                field,
                kind: FilterKind::Label(l),
            });
        }
    }

    /// Set, replace or remove (`None`) the value filter of a field (a
    /// value test or a Top 10).
    pub fn set_value_filter(&mut self, field: usize, filter: Option<FilterKind>) {
        self.filters
            .retain(|f| f.field != field || matches!(f.kind, FilterKind::Label(_) | FilterKind::Date { .. }));
        if let Some(kind) = filter
            && !matches!(kind, FilterKind::Label(_) | FilterKind::Date { .. })
        {
            self.filters.push(PivotFilter { field, kind });
        }
    }

    /// Set, replace or remove (`None`) the days a date field is limited
    /// to, first and last day included.
    pub fn set_date_filter(&mut self, field: usize, days: Option<(f64, f64)>) {
        self.filters
            .retain(|f| f.field != field || !matches!(f.kind, FilterKind::Date { .. }));
        if let Some((start, end)) = days {
            self.filters.push(PivotFilter {
                field,
                kind: FilterKind::Date { start, end },
            });
        }
    }

    /// The days a date field is limited to.
    pub fn date_filter(&self, field: usize) -> Option<(f64, f64)> {
        self.filters.iter().find_map(|f| match f.kind {
            FilterKind::Date { start, end } if f.field == field => Some((start, end)),
            _ => None,
        })
    }

    // ---- layout ------------------------------------------------------------

    /// "Report Layout": the form of every field.
    pub fn set_report_layout(&mut self, layout: FieldLayout) {
        for f in &mut self.fields {
            f.layout = layout;
        }
    }

    /// "Subtotals": none, at the bottom or at the top of each group.
    pub fn set_subtotals(&mut self, show: bool, top: bool) {
        for f in &mut self.fields {
            f.subtotals = if show { Subtotals::Automatic } else { Subtotals::None };
            f.subtotal_top = top;
        }
    }

    /// "Repeat All Item Labels" / "Do Not Repeat Item Labels".
    pub fn set_repeat_labels(&mut self, repeat: bool) {
        for f in &mut self.fields {
            f.repeat_labels = repeat;
        }
    }

    /// "Insert Blank Line after Each Item" / "Remove Blank Line...".
    pub fn set_blank_lines(&mut self, blank: bool) {
        for f in &mut self.fields {
            f.blank_line = blank;
        }
    }

    /// Where the report and its filter block go for a report of this size.
    fn place(&self, report: &Report) -> Result<(Rect, Option<Rect>), PivotError> {
        let (rows, cols) = report.size();
        let (page_rows, page_cols) = report.page_size();
        // The filters sit above the report, one empty row in between.
        let above = if page_rows > 0 { page_rows + 1 } else { 0 };
        let r0 = self.anchor.0.max(above);
        let c0 = self.anchor.1;
        let width = cols.max(page_cols).max(1);
        if r0 as u64 + rows.max(1) as u64 > MAX_ROWS as u64 || c0 as u64 + width as u64 > MAX_COLS as u64 {
            return Err(PivotError::TooLarge);
        }
        let body = Rect::new(r0, c0, r0 + rows.max(1) - 1, c0 + cols.max(1) - 1);
        let pages = (page_rows > 0).then(|| Rect::new(r0 - above, c0, r0 - 2, c0 + page_cols - 1));
        Ok((body, pages))
    }

    /// What the cell at (`row`, `col`) of the sheet is in this PivotTable.
    pub fn cell(&self, row: u32, col: u32) -> Option<PivotCell> {
        let report = &*self.report;
        let mut out = PivotCell {
            pivot: self.id,
            part: PivotPart::Blank,
            field: None,
            item: None,
            fields: Vec::new(),
            row_items: Vec::new(),
            col_items: Vec::new(),
            data_field: None,
            row_kind: None,
            col_kind: None,
            button: None,
            indent: 0,
        };
        let filter_button = |fields: &[usize]| {
            self.options.show_headers.then(|| PivotButton::Filter {
                filtered: fields.iter().any(|f| self.is_filtered(*f)),
            })
        };
        if let Some(pages) = self.page_location.filter(|r| r.contains(row, col)) {
            let (r, c) = (row - pages.r0, col - pages.c0);
            for page in report.pages.iter().filter(|p| p.row == r) {
                if page.col == c {
                    out.part = PivotPart::FilterLabel;
                    out.field = Some(page.field);
                } else if page.col + 1 == c {
                    out.part = PivotPart::FilterValue;
                    out.field = Some(page.field);
                    out.item = self
                        .page_fields
                        .iter()
                        .find(|p| p.field == page.field)
                        .and_then(|p| p.item);
                    out.fields = vec![page.field];
                    out.button = Some(PivotButton::Filter {
                        filtered: self.is_filtered(page.field),
                    });
                }
            }
            return Some(out);
        }
        let body = self.location.filter(|r| r.contains(row, col))?;
        let (r, c) = (row - body.r0, col - body.c0);
        if report.placeholder || r >= report.rows || c >= report.cols {
            out.part = if report.placeholder {
                PivotPart::Placeholder
            } else {
                PivotPart::Blank
            };
            return Some(out);
        }
        let kind = |l: &compute::Line| match l.kind {
            LineKind::Header => None,
            LineKind::Item => Some(ReportLineKind::Item),
            LineKind::Group => Some(ReportLineKind::Group),
            LineKind::Subtotal => Some(ReportLineKind::Subtotal),
            LineKind::Grand => Some(ReportLineKind::GrandTotal),
            LineKind::Blank => Some(ReportLineKind::Blank),
        };
        let row_line = report.row_lines[r as usize];
        let col_line = report.col_lines[c as usize];
        out.row_kind = kind(&row_line);
        out.col_kind = kind(&col_line);
        if out.row_kind.is_some() {
            out.row_items = report.path(true, row_line.node, self);
        }
        if out.col_kind.is_some() {
            out.col_items = report.path(false, col_line.node, self);
        }
        out.indent = report.indent(r, c).unwrap_or(0);
        let real = |list: &[AxisField]| list.iter().filter_map(|a| a.field()).collect::<Vec<_>>();
        // A label stands for the item of its field in the line's path, and
        // can be expanded or collapsed when a field lies below it.
        let label = |out: &mut PivotCell, rows: bool, level: u8, line: &compute::Line| {
            let (levels, axis) = if rows {
                (&report.row_levels, &self.row_fields)
            } else {
                (&report.col_levels, &self.col_fields)
            };
            let Some(lvl) = levels.get(level as usize) else {
                return;
            };
            match lvl.field {
                None => out.data_field = line.data(),
                Some(f) => {
                    let path = if rows { &mut out.row_items } else { &mut out.col_items };
                    path.truncate(lvl.depth as usize);
                    let item = path.last().filter(|(pf, _)| *pf == f).map(|(_, x)| *x);
                    out.field = Some(f);
                    out.item = item;
                    let below = axis.iter().skip(level as usize + 1).any(|a| a.field().is_some());
                    if below && self.options.show_drill && line.kind != LineKind::Subtotal {
                        let collapsed = out.item.is_some_and(|x| {
                            self.fields
                                .get(f)
                                .is_some_and(|pf| pf.items.iter().any(|i| i.x == x && i.collapsed))
                        });
                        out.button = Some(if collapsed {
                            PivotButton::Expand
                        } else {
                            PivotButton::Collapse
                        });
                    }
                }
            }
        };
        match report.tag(r, c) {
            Tag::Blank => {}
            Tag::DataCaption => {
                out.part = PivotPart::DataCaption;
                out.data_field = (self.data_fields.len() == 1).then_some(0);
            }
            Tag::RowCaption => {
                out.part = PivotPart::RowHeaderCaption;
                out.fields = report
                    .row_levels
                    .iter()
                    .filter(|l| l.slot == c)
                    .filter_map(|l| l.field)
                    .collect();
                out.button = filter_button(&out.fields).filter(|_| !out.fields.is_empty());
            }
            Tag::ColCaption => {
                out.part = PivotPart::ColumnHeaderCaption;
                out.fields = real(&self.col_fields);
                out.button = filter_button(&out.fields);
            }
            Tag::RowHeader(l) | Tag::ColHeader(l) => {
                out.part = PivotPart::FieldHeader;
                let levels = if matches!(report.tag(r, c), Tag::RowHeader(_)) {
                    &report.row_levels
                } else {
                    &report.col_levels
                };
                out.field = levels.get(l as usize).and_then(|v| v.field);
                out.fields = out.field.into_iter().collect();
                out.button = filter_button(&out.fields).filter(|_| out.field.is_some());
            }
            Tag::RowLabel(l) => {
                out.part = PivotPart::RowLabel;
                label(&mut out, true, l, &row_line);
            }
            Tag::ColLabel(l) => {
                out.part = PivotPart::ColumnLabel;
                label(&mut out, false, l, &col_line);
            }
            Tag::RowSubtotal(l) => {
                out.part = PivotPart::RowSubtotalLabel;
                label(&mut out, true, l, &row_line);
                out.data_field = row_line.data();
            }
            Tag::ColSubtotal(l) => {
                out.part = PivotPart::ColumnSubtotalLabel;
                label(&mut out, false, l, &col_line);
                out.data_field = col_line.data();
            }
            Tag::RowGrand | Tag::TotalLabel => {
                out.part = PivotPart::RowGrandTotalLabel;
                out.data_field = row_line.data();
            }
            Tag::ColGrand => {
                out.part = PivotPart::ColumnGrandTotalLabel;
                out.data_field = col_line.data();
            }
            Tag::Value => {
                out.part = PivotPart::Value;
                out.data_field = report.data_at(r, c, self);
            }
        }
        Some(out)
    }
}

/// Follow rows or columns inserted into or deleted from a sheet; `None`
/// when the whole rectangle was deleted.
fn adjust_rect(rect: Rect, edit: StructuralEdit) -> Option<Rect> {
    let interval = |lo: u32, hi: u32, insert: bool, at: u32, count: u32, max: u32| -> Option<(u32, u32)> {
        if insert {
            let lo = if lo >= at { lo + count } else { lo };
            let hi = if hi >= at { hi + count } else { hi };
            (lo < max).then(|| (lo, hi.min(max - 1)))
        } else {
            let end = at + count - 1;
            if lo >= at && hi <= end {
                return None;
            }
            let lo = if lo > end { lo - count } else { lo.min(at) };
            let hi = if hi > end {
                hi - count
            } else if hi >= at {
                at - 1
            } else {
                hi
            };
            Some((lo, hi))
        }
    };
    match edit {
        StructuralEdit::InsertRows { at, count } => {
            interval(rect.r0, rect.r1, true, at, count, MAX_ROWS).map(|(a, b)| Rect::new(a, rect.c0, b, rect.c1))
        }
        StructuralEdit::DeleteRows { at, count } => {
            interval(rect.r0, rect.r1, false, at, count, MAX_ROWS).map(|(a, b)| Rect::new(a, rect.c0, b, rect.c1))
        }
        StructuralEdit::InsertCols { at, count } => {
            interval(rect.c0, rect.c1, true, at, count, MAX_COLS).map(|(a, b)| Rect::new(rect.r0, a, rect.r1, b))
        }
        StructuralEdit::DeleteCols { at, count } => {
            interval(rect.c0, rect.c1, false, at, count, MAX_COLS).map(|(a, b)| Rect::new(rect.r0, a, rect.r1, b))
        }
    }
}

/// True when the edit would insert into or delete part of the rectangle
/// without taking all of it: Excel refuses such an edit across a
/// PivotTable.
fn cuts(rect: Rect, edit: StructuralEdit) -> bool {
    let through = |lo: u32, hi: u32, insert: bool, at: u32, count: u32| {
        if insert {
            at > lo && at <= hi
        } else {
            let end = at + count - 1;
            at <= hi && end >= lo && !(at <= lo && end >= hi)
        }
    };
    match edit {
        StructuralEdit::InsertRows { at, count } => through(rect.r0, rect.r1, true, at, count),
        StructuralEdit::DeleteRows { at, count } => through(rect.r0, rect.r1, false, at, count),
        StructuralEdit::InsertCols { at, count } => through(rect.c0, rect.c1, true, at, count),
        StructuralEdit::DeleteCols { at, count } => through(rect.c0, rect.c1, false, at, count),
    }
}

/// Cache of values derived from the cells (see `features::Caches`). A
/// PivotTable keeps its computed report with its definition, on the sheet,
/// so that undo restores both together: nothing is cached here.
#[derive(Debug, Default)]
pub struct Cache;

/// See `features::on_structural_edit`: the source ranges and the reports
/// follow the rows and columns. A PivotTable whose report was deleted
/// whole goes with it.
pub(crate) fn on_structural_edit(sheet: &mut Sheet, edit: StructuralEdit, edited_sheet: &str) {
    let own = same_name(&sheet.name, edited_sheet);
    for p in &mut sheet.pivots {
        if let PivotSource::Range { sheet: s, range } = &mut p.source
            && same_name(s, edited_sheet)
        {
            match adjust_rect(*range, edit) {
                Some(r) => *range = r,
                None => p.source = PivotSource::Invalid,
            }
        }
    }
    if !own {
        return;
    }
    sheet.pivots.retain_mut(|p| {
        let Some(body) = p.location else {
            return true;
        };
        match adjust_rect(body, edit) {
            Some(r) => {
                p.location = Some(r);
                p.anchor = (r.r0, r.c0);
                p.page_location = p.page_location.and_then(|r| adjust_rect(r, edit));
                true
            }
            None => false,
        }
    });
}

/// See `features::on_sheet_renamed`.
pub(crate) fn on_sheet_renamed(sheet: &mut Sheet, old: &str, new: &str) {
    for p in &mut sheet.pivots {
        if let PivotSource::Range { sheet: s, .. } = &mut p.source
            && same_name(s, old)
        {
            *s = new.to_string();
        }
    }
}

/// See `features::on_sheet_duplicated`: the copies get ids of their own.
/// Like Excel's, they keep reading the source of the original (and share
/// its cache).
pub(crate) fn on_sheet_duplicated(wb: &mut Workbook, copy: usize, _original: &str) {
    let mut next = wb
        .sheets
        .iter()
        .flat_map(|s| s.pivots.iter().map(|p| p.id))
        .max()
        .unwrap_or(0);
    if let Some(sh) = wb.sheets.get_mut(copy) {
        for p in &mut sh.pivots {
            next += 1;
            p.id = next;
            // The charts of the original stay its own; the copies of
            // them on this sheet are plain charts.
            p.charts.clear();
        }
    }
}

/// See `features::on_sheet_deleted`. A PivotTable whose records came from
/// a range of the sheet keeps its cache, as in Excel, but has nothing left
/// to refresh from: a sheet later given the same name is not its source.
pub(crate) fn on_sheet_deleted(wb: &mut crate::workbook::Workbook, deleted: &crate::workbook::Sheet, _order: &[&str]) {
    for sh in &mut wb.sheets {
        for p in &mut sh.pivots {
            if let PivotSource::Range { sheet, .. } = &p.source
                && same_name(sheet, &deleted.name)
            {
                p.source = PivotSource::Invalid;
            }
        }
    }
}

/// See `features::on_sheet_moved`.
pub(crate) fn on_sheet_moved(_wb: &mut crate::workbook::Workbook, _moved: &str, _before: &[&str], _after: &[&str]) {}

/// See `features::on_cells_shifted`.
pub(crate) fn on_cells_shifted(_wb: &mut crate::workbook::Workbook, _sheet: usize, _shift: crate::features::CellShift) {}
