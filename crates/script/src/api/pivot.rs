//! PivotTables: `PivotTable`, its hierarchies (one per field of the
//! source), the fields and items behind them, and its layout.
//!
//! A change made by a script is the change the field list makes: the
//! report is written again at once, and refused, with Excel's message,
//! when it would cover cells holding data.

use std::sync::Arc;

use serde_json::{Value as J, json};

use opensheet_engine::pivot::{
    Aggregation, Area, AxisField, BaseItem, FieldLayout, FilterKind, LabelFilter, PivotDestination, PivotError, PivotSource, PivotTable,
    ShowAs, SortOrder, Subtotals, Top10By, ValueFilter,
};
use opensheet_engine::protect::Action;
use opensheet_engine::refs::Rect;

use super::util::range_arg;
use super::{ApiClass, Call, Fail, Res, Ret, class, enum_value, object, state_of};
use crate::host::Host;

fn fail(call: &Call, error: PivotError) -> Fail {
    call.fail(match error {
        PivotError::NoSuchPivot | PivotError::NoSuchField | PivotError::NotAValue => "The requested resource doesn't exist.",
        PivotError::InvalidSource => "Data source reference is not valid.",
        PivotError::EmptyFieldName => "The PivotTable field name is not valid. To create a PivotTable report, you must use data that is organized as a list with labeled columns.",
        PivotError::OverlapsPivot(_) => "A PivotTable report cannot overlap another PivotTable report.",
        PivotError::OverlapsTable => "A PivotTable report cannot overlap a table or XML mapping.",
        PivotError::OverlapsMerged => "We can't do that to a merged cell.",
        PivotError::WouldOverwrite(_) => "A PivotTable report cannot overlap cells with data.",
        PivotError::TooLarge => "The PivotTable report will not fit on the sheet. Please select a different location.",
        PivotError::InvalidName => "PivotTable name is not valid.",
        _ => "The PivotTable cannot be changed this way.",
    })
}

// ---- the PivotTable behind an object ---------------------------------------------

fn pivot_object(id: u32) -> J {
    object("PivotTable", vec![id.into()])
}

fn pivot<'h>(h: &'h Host, call: &Call) -> Res<(usize, &'h PivotTable)> {
    let id = u32::try_from(call.this_int(0)).map_err(|_| call.gone())?;
    h.wb.pivot(id).ok_or_else(|| call.gone())
}

/// A number of the state of the object: a field, a value field, an item.
fn part(call: &Call, index: usize) -> Res<usize> {
    usize::try_from(call.this_int(index)).map_err(|_| call.gone())
}

/// The PivotTable and the field of a hierarchy or of a field object.
fn field<'h>(h: &'h Host, call: &Call) -> Res<(&'h PivotTable, usize)> {
    let (_, p) = pivot(h, call)?;
    let field = part(call, 1)?;
    if field >= p.fields.len() {
        return Err(call.gone());
    }
    Ok((p, field))
}

/// The PivotTable and the value field of a DataPivotHierarchy.
fn data<'h>(h: &'h Host, call: &Call) -> Res<(&'h PivotTable, usize)> {
    let (_, p) = pivot(h, call)?;
    let index = part(call, 1)?;
    if index >= p.data_fields.len() {
        return Err(call.gone());
    }
    Ok((p, index))
}

fn item<'h>(h: &'h Host, call: &Call) -> Res<(&'h PivotTable, usize, u32)> {
    let (p, field) = field(h, call)?;
    let x = part(call, 2)? as u32;
    if !p.fields[field].items.iter().any(|it| it.x == x) {
        return Err(call.gone());
    }
    Ok((p, field, x))
}

/// Change the PivotTable a call is made on and write its report again.
fn update(h: &mut Host, call: &Call, edit: impl FnOnce(&mut PivotTable)) -> Res<Ret> {
    let (sheet, p) = pivot(h, call)?;
    let id = p.id;
    h.permit(call, &Action::UsePivotTable { sheet })?;
    h.wb.pivot_update(id, false, edit).map_err(|e| fail(call, e))?;
    h.restructured();
    Ok(Ret::Nothing)
}

/// A change that the PivotTable may refuse (a name already taken).
fn update_checked(h: &mut Host, call: &Call, edit: impl FnOnce(&mut PivotTable) -> bool) -> Res<Ret> {
    let mut done = true;
    update(h, call, |p| done = edit(p))?;
    if done { Ok(Ret::Nothing) } else { Err(call.fail("A field or an item of the PivotTable already has this name.")) }
}

fn sub_object(call: &Call, class: &str, rest: &[usize]) -> J {
    let mut state = vec![J::from(call.this_int(0))];
    state.extend(rest.iter().map(|n| J::from(*n)));
    object(class, state)
}

/// The class of the hierarchies of an area.
fn area_class(area: Area) -> &'static str {
    match area {
        Area::Filters => "FilterPivotHierarchy",
        Area::Values => "DataPivotHierarchy",
        Area::Rows | Area::Columns => "RowColumnPivotHierarchy",
    }
}

/// The fields of an area in their order (the value fields by index).
fn entries(p: &PivotTable, area: Area) -> Vec<usize> {
    match area {
        Area::Rows => p.row_fields.iter().filter_map(AxisField::field).collect(),
        Area::Columns => p.col_fields.iter().filter_map(AxisField::field).collect(),
        Area::Filters => p.page_fields.iter().map(|page| page.field).collect(),
        Area::Values => (0..p.data_fields.len()).collect(),
    }
}

/// The name a hierarchy of an area goes by.
fn entry_name(p: &PivotTable, area: Area, entry: usize) -> &str {
    match area {
        Area::Values => p.data_fields.get(entry).map_or("", |d| d.name.as_str()),
        _ => p.field_caption(entry),
    }
}

fn hierarchies(h: &Host, call: &Call, area: Area) -> Res<Ret> {
    let (_, p) = pivot(h, call)?;
    let objects: Vec<J> = entries(p, area).into_iter().map(|entry| sub_object(call, area_class(area), &[entry])).collect();
    Ok(Ret::Object(J::from(objects)))
}

fn hierarchy(h: &Host, call: &Call, area: Area) -> Res<Ret> {
    let (_, p) = pivot(h, call)?;
    let name = call.str(0)?;
    let found = entries(p, area).into_iter().find(|entry| entry_name(p, area, *entry).eq_ignore_ascii_case(name));
    Ok(Ret::maybe(found.map(|entry| sub_object(call, area_class(area), &[entry]))))
}

/// The field of a PivotHierarchy (or of a hierarchy of an area) given as
/// an argument, which must belong to this PivotTable.
fn hierarchy_arg(call: &Call, index: usize, classes: &[&str]) -> Res<usize> {
    let given = call.arg(index);
    let state = classes.iter().find_map(|class| state_of(given, class)).ok_or_else(|| call.invalid())?;
    if state.first().and_then(J::as_i64) != Some(call.this_int(0)) {
        return Err(call.fail("The hierarchy belongs to another PivotTable."));
    }
    state.get(1).and_then(J::as_u64).map(|n| n as usize).ok_or_else(|| call.invalid())
}

/// `addRowHierarchy` and its like: the field goes to the area, as when
/// it is dragged there in the field list.
fn add_to(h: &mut Host, call: &Call, area: Area) -> Res<Ret> {
    let (sheet, p) = pivot(h, call)?;
    let id = p.id;
    let field = hierarchy_arg(call, 0, &["PivotHierarchy", "RowColumnPivotHierarchy", "FilterPivotHierarchy"])?;
    if field >= p.fields.len() {
        return Err(call.gone());
    }
    h.permit(call, &Action::UsePivotTable { sheet })?;
    h.wb.pivot_add_field(id, field, area, None, false).map_err(|e| fail(call, e))?;
    h.restructured();
    let p = h.wb.pivot(id).ok_or_else(|| call.gone())?.1;
    let entry = match area {
        Area::Values => p.data_fields.len().saturating_sub(1),
        _ => field,
    };
    Ok(Ret::Object(sub_object(call, area_class(area), &[entry])))
}

fn remove_from(h: &mut Host, call: &Call, area: Area) -> Res<Ret> {
    let entry = hierarchy_arg(call, 0, &[area_class(area)])?;
    let (_, p) = pivot(h, call)?;
    if !entries(p, area).contains(&entry) {
        return Err(call.gone());
    }
    update(h, call, |p| match area {
        Area::Values => p.remove_data_field(entry),
        _ => p.remove_field(entry),
    })
}

/// The place of a hierarchy in its area.
fn position(h: &Host, call: &Call, area: Option<Area>) -> Res<(Area, usize)> {
    let (_, p) = pivot(h, call)?;
    let entry = part(call, 1)?;
    let areas = area.map_or(vec![Area::Rows, Area::Columns], |area| vec![area]);
    areas
        .into_iter()
        .find_map(|area| entries(p, area).iter().position(|e| *e == entry).map(|at| (area, at)))
        .ok_or_else(|| call.gone())
}

/// `setPosition`: the hierarchy moves among those of its area.
fn set_position(h: &mut Host, call: &Call, area: Option<Area>) -> Res<Ret> {
    let (area, from) = position(h, call, area)?;
    let (_, p) = pivot(h, call)?;
    let to = usize::try_from(call.int(0)?).ok().filter(|to| *to < entries(p, area).len()).ok_or_else(|| call.invalid())?;
    // "Σ Values" may stand among the fields of the rows or the columns.
    let slot = |p: &PivotTable, nth: usize| match area {
        Area::Rows => p.row_fields.iter().enumerate().filter(|(_, a)| a.field().is_some()).nth(nth).map(|(at, _)| at),
        Area::Columns => p.col_fields.iter().enumerate().filter(|(_, a)| a.field().is_some()).nth(nth).map(|(at, _)| at),
        _ => Some(nth),
    };
    let (from, to) = (slot(p, from).ok_or_else(|| call.gone())?, slot(p, to).ok_or_else(|| call.gone())?);
    let locale = h.wb.locale;
    update(h, call, |p| {
        p.move_entry(area, from, area, to, locale);
    })
}

/// `setToDefault`: the hierarchy leaves its area.
fn set_to_default(h: &mut Host, call: &Call, area: Option<Area>) -> Res<Ret> {
    let (area, _) = position(h, call, area)?;
    let entry = part(call, 1)?;
    update(h, call, |p| match area {
        Area::Values => p.remove_data_field(entry),
        _ => p.remove_field(entry),
    })
}

fn field_object(h: &Host, call: &Call) -> Res<J> {
    let (_, field) = field(h, call)?;
    Ok(sub_object(call, "PivotField", &[field]))
}

fn rename_field(h: &mut Host, call: &Call) -> Res<Ret> {
    let (_, at) = field(h, call)?;
    let name = call.str(0)?;
    if name.trim().is_empty() {
        return Err(call.invalid());
    }
    update_checked(h, call, |p| p.set_field_caption(at, name))
}

// ---- adding and finding ----------------------------------------------------------------

/// `addPivotTable(name, source, destination)`.
pub(super) fn add(h: &mut Host, call: &Call, sheet: usize) -> Res<Ret> {
    let name = call.str(0)?;
    let source = match (call.arg(1), state_of(call.arg(1), "Table")) {
        (_, Some(state)) => {
            let id = state.first().and_then(J::as_u64).ok_or_else(|| call.invalid())? as u32;
            let table = h.wb.sheets.iter().flat_map(|sh| sh.tables.iter()).find(|t| t.id == id).ok_or_else(|| call.gone())?;
            PivotSource::Name(table.name.clone())
        }
        (J::String(text), None) if h.wb.table_by_name(text).is_some() => PivotSource::Name(text.clone()),
        _ => {
            let (on, range) = range_arg(h, call, 1, sheet)?;
            PivotSource::Range { sheet: h.wb.sheets[on].name.clone(), range }
        }
    };
    let (on, at) = range_arg(h, call, 2, sheet)?;
    h.permit(call, &Action::UsePivotTable { sheet: on })?;
    // The source is read as it is now.
    h.calculate();
    let dest = PivotDestination::Existing { sheet: on, row: at.r0, col: at.c0 };
    let id = h.wb.pivot_create(source, dest, false).map_err(|e| fail(call, e))?;
    if let Err(error) = h.wb.pivot_rename(id, name) {
        let _ = h.wb.pivot_delete(id);
        return Err(fail(call, error));
    }
    h.restructured();
    Ok(Ret::Object(pivot_object(id)))
}

pub(super) fn all(h: &Host, only: Option<usize>) -> Ret {
    let objects: Vec<J> = h.wb.pivots().filter(|(sheet, _)| only.is_none_or(|s| s == *sheet)).map(|(_, p)| pivot_object(p.id)).collect();
    Ret::Object(J::from(objects))
}

pub(super) fn by_name(h: &Host, call: &Call, only: Option<usize>) -> Res<Ret> {
    let name = call.str(0)?;
    let found = h.wb.pivots().find(|(sheet, p)| only.is_none_or(|s| s == *sheet) && p.name.eq_ignore_ascii_case(name));
    Ok(Ret::maybe(found.map(|(_, p)| pivot_object(p.id))))
}

/// `refreshAllPivotTables`, for the workbook or one sheet.
pub(super) fn refresh_all(h: &mut Host, call: &Call, only: Option<usize>) -> Res<Ret> {
    let found: Vec<(usize, u32)> = h.wb.pivots().filter(|(sheet, _)| only.is_none_or(|s| s == *sheet)).map(|(sheet, p)| (sheet, p.id)).collect();
    h.calculate();
    for (sheet, id) in found {
        h.permit(call, &Action::UsePivotTable { sheet })?;
        h.wb.pivot_refresh(id, false).map_err(|e| fail(call, e))?;
        h.restructured();
    }
    Ok(Ret::Nothing)
}

// ---- summaries and calculations -----------------------------------------------------------

const FUNCTIONS: [(&str, Aggregation); 11] = [
    ("Sum", Aggregation::Sum),
    ("Count", Aggregation::Count),
    ("Average", Aggregation::Average),
    ("Max", Aggregation::Max),
    ("Min", Aggregation::Min),
    ("Product", Aggregation::Product),
    ("CountNumbers", Aggregation::CountNums),
    ("StandardDeviation", Aggregation::StdDev),
    ("StandardDeviationP", Aggregation::StdDevp),
    ("Variance", Aggregation::Var),
    ("VarianceP", Aggregation::Varp),
];

/// The keys of a `Subtotals` object, beside `automatic`.
const SUBTOTAL_KEYS: [(&str, Aggregation); 11] = [
    ("sum", Aggregation::Sum),
    ("count", Aggregation::Count),
    ("average", Aggregation::Average),
    ("max", Aggregation::Max),
    ("min", Aggregation::Min),
    ("product", Aggregation::Product),
    ("countNumbers", Aggregation::CountNums),
    ("standardDeviation", Aggregation::StdDev),
    ("standardDeviationP", Aggregation::StdDevp),
    ("variance", Aggregation::Var),
    ("varianceP", Aggregation::Varp),
];

/// A `ShowAsRule` as the calculation of a value field.
fn show_as(call: &Call, given: &J) -> Res<ShowAs> {
    let state = |key: &str, class: &str| -> Res<Option<Vec<u64>>> {
        match given.get(key).filter(|value| !value.is_null()) {
            None => Ok(None),
            Some(value) => {
                let state = state_of(value, class).ok_or_else(|| call.invalid())?;
                if state.first().and_then(J::as_i64) != Some(call.this_int(0)) {
                    return Err(call.fail("The base field belongs to another PivotTable."));
                }
                Ok(Some(state.iter().filter_map(J::as_u64).collect()))
            }
        }
    };
    let base_field = state("baseField", "PivotField")?.and_then(|s| s.get(1).copied()).map(|f| f as usize);
    let base_item = state("baseItem", "PivotItem")?;
    let field = || base_field.or(base_item.as_ref().and_then(|s| s.get(1).copied()).map(|f| f as usize)).ok_or_else(|| call.fail("This calculation needs a base field."));
    let item = || base_item.as_ref().and_then(|s| s.get(2).copied()).map(|x| BaseItem::Item(x as u32)).ok_or_else(|| call.fail("This calculation needs a base item."));
    Ok(match given.get("calculation").and_then(J::as_str).ok_or_else(|| call.invalid())? {
        "None" => ShowAs::Normal,
        "PercentOfGrandTotal" => ShowAs::PercentOfTotal,
        "PercentOfRowTotal" => ShowAs::PercentOfRow,
        "PercentOfColumnTotal" => ShowAs::PercentOfColumn,
        "PercentOfParentRowTotal" => ShowAs::PercentOfParentRow,
        "PercentOfParentColumnTotal" => ShowAs::PercentOfParentColumn,
        "PercentOfParentTotal" => ShowAs::PercentOfParent { field: field()? },
        "PercentOf" => ShowAs::PercentOf { field: field()?, item: item()? },
        "DifferenceFrom" => ShowAs::Difference { field: field()?, item: item()? },
        "PercentDifferenceFrom" => ShowAs::PercentDifference { field: field()?, item: item()? },
        "RunningTotal" => ShowAs::RunningTotal { field: field()? },
        "PercentRunningTotal" => ShowAs::PercentRunningTotal { field: field()? },
        "RankAscending" => ShowAs::RankAscending { field: field()? },
        "RankDecending" => ShowAs::RankDescending { field: field()? },
        "Index" => ShowAs::Index,
        _ => return Err(call.invalid()),
    })
}

/// The calculation of a value field as a `ShowAsRule`.
fn show_as_out(call: &Call, rule: &ShowAs) -> J {
    let (name, field, item) = match rule {
        ShowAs::Normal => ("None", None, None),
        ShowAs::PercentOfTotal => ("PercentOfGrandTotal", None, None),
        ShowAs::PercentOfRow => ("PercentOfRowTotal", None, None),
        ShowAs::PercentOfColumn => ("PercentOfColumnTotal", None, None),
        ShowAs::PercentOfParentRow => ("PercentOfParentRowTotal", None, None),
        ShowAs::PercentOfParentColumn => ("PercentOfParentColumnTotal", None, None),
        ShowAs::PercentOfParent { field } => ("PercentOfParentTotal", Some(*field), None),
        ShowAs::PercentOf { field, item } => ("PercentOf", Some(*field), Some(*item)),
        ShowAs::Difference { field, item } => ("DifferenceFrom", Some(*field), Some(*item)),
        ShowAs::PercentDifference { field, item } => ("PercentDifferenceFrom", Some(*field), Some(*item)),
        ShowAs::RunningTotal { field } => ("RunningTotal", Some(*field), None),
        ShowAs::PercentRunningTotal { field } => ("PercentRunningTotal", Some(*field), None),
        ShowAs::RankAscending { field } => ("RankAscending", Some(*field), None),
        ShowAs::RankDescending { field } => ("RankDecending", Some(*field), None),
        ShowAs::Index => ("Index", None, None),
    };
    let mut out = json!({ "calculation": name });
    if let Some(field) = field {
        out["baseField"] = sub_object(call, "PivotField", &[field]);
        // The previous and the next item have no object to stand for them.
        if let Some(BaseItem::Item(x)) = item {
            out["baseItem"] = sub_object(call, "PivotItem", &[field, x as usize]);
        }
    }
    out
}

// ---- filters ---------------------------------------------------------------------------

fn text_of(value: Option<&J>) -> Option<String> {
    match value? {
        J::String(text) => Some(text.clone()),
        J::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// A `PivotLabelFilter` as the engine filters labels.
fn label_filter(call: &Call, given: &J) -> Res<LabelFilter> {
    let text = |key: &str| text_of(given.get(key)).ok_or_else(|| call.invalid());
    let not = given.get("exclusive").and_then(J::as_bool).unwrap_or(false);
    Ok(match (given.get("condition").and_then(J::as_str).ok_or_else(|| call.invalid())?, not) {
        ("BeginsWith", false) => LabelFilter::BeginsWith(text("substring")?),
        ("BeginsWith", true) => LabelFilter::NotBeginsWith(text("substring")?),
        ("EndsWith", false) => LabelFilter::EndsWith(text("substring")?),
        ("EndsWith", true) => LabelFilter::NotEndsWith(text("substring")?),
        ("Contains", false) => LabelFilter::Contains(text("substring")?),
        ("Contains", true) => LabelFilter::NotContains(text("substring")?),
        ("Equals", false) => LabelFilter::Equals(text("comparator")?),
        ("Equals", true) => LabelFilter::NotEquals(text("comparator")?),
        // "Not greater than" is "less than or equal to", and so on.
        ("GreaterThan", false) | ("LessThanOrEqualTo", true) => LabelFilter::GreaterThan(text("comparator")?),
        ("GreaterThanOrEqualTo", false) | ("LessThan", true) => LabelFilter::GreaterOrEqual(text("comparator")?),
        ("LessThan", false) | ("GreaterThanOrEqualTo", true) => LabelFilter::LessThan(text("comparator")?),
        ("LessThanOrEqualTo", false) | ("GreaterThan", true) => LabelFilter::LessOrEqual(text("comparator")?),
        ("Between", false) => LabelFilter::Between(text("lowerBound")?, text("upperBound")?),
        ("Between", true) => LabelFilter::NotBetween(text("lowerBound")?, text("upperBound")?),
        _ => return Err(call.invalid()),
    })
}

/// A `PivotValueFilter` as the engine filters on a value field.
fn value_filter(call: &Call, p: &PivotTable, given: &J) -> Res<FilterKind> {
    let number = |key: &str| given.get(key).and_then(J::as_f64).ok_or_else(|| call.invalid());
    let name = given.get("value").and_then(J::as_str).ok_or_else(|| call.invalid())?;
    let data_field = p.data_fields.iter().position(|d| d.name.eq_ignore_ascii_case(name)).ok_or_else(|| call.fail("The PivotTable has no value field of this name."))?;
    let not = given.get("exclusive").and_then(J::as_bool).unwrap_or(false);
    let condition = given.get("condition").and_then(J::as_str).ok_or_else(|| call.invalid())?;
    if let "TopN" | "BottomN" = condition {
        let by = match given.get("selectionType").and_then(J::as_str).unwrap_or("Items") {
            "Items" => Top10By::Items,
            "Percent" => Top10By::Percent,
            "Sum" => Top10By::Sum,
            _ => return Err(call.invalid()),
        };
        return Ok(FilterKind::Top10 { data_field, top: condition == "TopN", count: number("threshold")?, by });
    }
    let test = match (condition, not) {
        ("Equals", false) => ValueFilter::Equals(number("comparator")?),
        ("Equals", true) => ValueFilter::NotEquals(number("comparator")?),
        ("GreaterThan", false) | ("LessThanOrEqualTo", true) => ValueFilter::GreaterThan(number("comparator")?),
        ("GreaterThanOrEqualTo", false) | ("LessThan", true) => ValueFilter::GreaterOrEqual(number("comparator")?),
        ("LessThan", false) | ("GreaterThanOrEqualTo", true) => ValueFilter::LessThan(number("comparator")?),
        ("LessThanOrEqualTo", false) | ("GreaterThan", true) => ValueFilter::LessOrEqual(number("comparator")?),
        ("Between", false) => ValueFilter::Between(number("lowerBound")?, number("upperBound")?),
        ("Between", true) => ValueFilter::NotBetween(number("lowerBound")?, number("upperBound")?),
        _ => return Err(call.invalid()),
    };
    Ok(FilterKind::Value { data_field, test })
}

/// The items a manual filter keeps: names, or PivotItem objects.
fn chosen_items(h: &Host, call: &Call, p: &PivotTable, field: usize, given: &J) -> Res<Vec<u32>> {
    let list = given.get("selectedItems").and_then(J::as_array).ok_or_else(|| call.invalid())?;
    let locale = h.wb.locale;
    list.iter()
        .map(|chosen| match (chosen.as_str(), state_of(chosen, "PivotItem")) {
            (Some(name), _) => p.fields[field].items.iter().map(|it| it.x).find(|x| p.item_text(field, *x, locale).eq_ignore_ascii_case(name)).ok_or_else(|| call.fail(format!("The field has no item named {name}."))),
            (None, Some(state)) => state.get(2).and_then(J::as_u64).map(|x| x as u32).ok_or_else(|| call.invalid()),
            _ => Err(call.invalid()),
        })
        .collect()
}

/// Which kinds of filter a field has: (manual, label, value).
fn filtered(p: &PivotTable, field: usize) -> (bool, bool, bool) {
    let manual = p.fields[field].items.iter().any(|it| it.hidden) || p.page_fields.iter().any(|page| page.field == field && page.item.is_some());
    let of = |label: bool| p.filters.iter().any(|f| f.field == field && matches!(f.kind, FilterKind::Label(_)) == label && !matches!(f.kind, FilterKind::Date { .. }));
    (manual, of(true), of(false))
}

// ---- the classes -------------------------------------------------------------------------

pub(super) const PIVOT_TABLE: ApiClass = class!("PivotTable", "A PivotTable: a report that summarises a list by the fields put in its rows, columns, values and filters.", {
    getId "(): string" "The identifier of the PivotTable." => |h, c| Ok(pivot(h, c)?.1.id.to_string().into());
    getName "(): string" "The name of the PivotTable." => |h, c| Ok(pivot(h, c)?.1.name.clone().into());
    setName "(name: string): void" "Renames the PivotTable; the name must not be that of another PivotTable of the worksheet." => |h, c| {
        let (sheet, p) = pivot(h, c)?;
        let id = p.id;
        h.permit(c, &Action::UsePivotTable { sheet })?;
        h.wb.pivot_rename(id, c.str(0)?).map_err(|e| fail(c, e))?;
        Ok(Ret::Nothing)
    };
    getWorksheet "(): Worksheet" "The worksheet the PivotTable is on." => |h, c| Ok(Ret::Object(h.worksheet(pivot(h, c)?.0)));
    getLayout "(): PivotLayout" "The layout of the report: its form, its totals, its ranges." => |h, c| {
        pivot(h, c)?;
        Ok(Ret::Object(sub_object(c, "PivotLayout", &[])))
    };
    getHierarchies "(): PivotHierarchy[]" "One hierarchy per field of the source, in the order of its columns." => |h, c| {
        let (_, p) = pivot(h, c)?;
        Ok(Ret::Object(J::from((0..p.fields.len()).map(|field| sub_object(c, "PivotHierarchy", &[field])).collect::<Vec<J>>())))
    };
    getHierarchy "(name: string): PivotHierarchy | undefined" "The hierarchy of the field with this name; undefined when the source has no such field." => |h, c| {
        let (_, p) = pivot(h, c)?;
        Ok(Ret::maybe(p.field_index(c.str(0)?).map(|field| sub_object(c, "PivotHierarchy", &[field]))))
    };
    getRowHierarchies "(): RowColumnPivotHierarchy[]" "The fields of the rows, the outermost first." => |h, c| hierarchies(h, c, Area::Rows);
    getRowHierarchy "(name: string): RowColumnPivotHierarchy | undefined" "The row field with this name; undefined when there is none." => |h, c| hierarchy(h, c, Area::Rows);
    addRowHierarchy "(pivotHierarchy: PivotHierarchy): RowColumnPivotHierarchy" "Puts a field in the rows, after those already there (a field of dates is grouped by years, quarters and months as in the field list)." => |h, c| add_to(h, c, Area::Rows);
    removeRowHierarchy "(rowColumnPivotHierarchy: RowColumnPivotHierarchy): void" "Takes a field out of the rows." => |h, c| remove_from(h, c, Area::Rows);
    getColumnHierarchies "(): RowColumnPivotHierarchy[]" "The fields of the columns, the outermost first." => |h, c| hierarchies(h, c, Area::Columns);
    getColumnHierarchy "(name: string): RowColumnPivotHierarchy | undefined" "The column field with this name; undefined when there is none." => |h, c| hierarchy(h, c, Area::Columns);
    addColumnHierarchy "(pivotHierarchy: PivotHierarchy): RowColumnPivotHierarchy" "Puts a field in the columns, after those already there." => |h, c| add_to(h, c, Area::Columns);
    removeColumnHierarchy "(rowColumnPivotHierarchy: RowColumnPivotHierarchy): void" "Takes a field out of the columns." => |h, c| remove_from(h, c, Area::Columns);
    getFilterHierarchies "(): FilterPivotHierarchy[]" "The fields of the filters." => |h, c| hierarchies(h, c, Area::Filters);
    getFilterHierarchy "(name: string): FilterPivotHierarchy | undefined" "The filter field with this name; undefined when there is none." => |h, c| hierarchy(h, c, Area::Filters);
    addFilterHierarchy "(pivotHierarchy: PivotHierarchy): FilterPivotHierarchy" "Puts a field in the filters, above the report." => |h, c| add_to(h, c, Area::Filters);
    removeFilterHierarchy "(filterPivotHierarchy: FilterPivotHierarchy): void" "Takes a field out of the filters." => |h, c| remove_from(h, c, Area::Filters);
    getDataHierarchies "(): DataPivotHierarchy[]" "The value fields, in their order." => |h, c| hierarchies(h, c, Area::Values);
    getDataHierarchy "(name: string): DataPivotHierarchy | undefined" "The value field with this name (Sum of Sales); undefined when there is none." => |h, c| hierarchy(h, c, Area::Values);
    addDataHierarchy "(pivotHierarchy: PivotHierarchy): DataPivotHierarchy" "Adds a value field: the sum of a field of numbers, the count of another. A field can be a value several times." => |h, c| add_to(h, c, Area::Values);
    removeDataHierarchy "(DataPivotHierarchy: DataPivotHierarchy): void" "Removes a value field." => |h, c| remove_from(h, c, Area::Values);
    refresh "(): void" "Reads the source again and writes the report, with the other PivotTables built on the same data." => |h, c| {
        let (sheet, p) = pivot(h, c)?;
        let id = p.id;
        h.permit(c, &Action::UsePivotTable { sheet })?;
        h.calculate();
        h.wb.pivot_refresh(id, false).map_err(|e| fail(c, e))?;
        h.restructured();
        Ok(Ret::Nothing)
    };
    delete "(): void" "Deletes the PivotTable and what it wrote." => |h, c| {
        let (sheet, p) = pivot(h, c)?;
        let id = p.id;
        h.permit(c, &Action::UsePivotTable { sheet })?;
        h.wb.pivot_delete(id).map_err(|e| fail(c, e))?;
        h.restructured();
        Ok(Ret::Nothing)
    };
});

pub(super) const PIVOT_HIERARCHY: ApiClass = class!("PivotHierarchy", "A field of the source of a PivotTable, wherever it is used.", {
    getId "(): string" "The identifier of the hierarchy: the place of its field in the source." => |h, c| Ok(field(h, c)?.1.to_string().into());
    getName "(): string" "The name of the field, as the PivotTable shows it." => |h, c| {
        let (p, field) = field(h, c)?;
        Ok(p.field_caption(field).to_string().into())
    };
    setName "(name: string): void" "Gives the field another name in this PivotTable." => |h, c| rename_field(h, c);
    getFields "(): PivotField[]" "The field of the hierarchy: one, in an array." => |h, c| Ok(Ret::Object(J::from(vec![field_object(h, c)?])));
    getPivotField "(name: string): PivotField | undefined" "The field of the hierarchy when it has this name; undefined otherwise." => |h, c| {
        let (p, at) = field(h, c)?;
        Ok(Ret::maybe(p.field_caption(at).eq_ignore_ascii_case(c.str(0)?).then(|| sub_object(c, "PivotField", &[at]))))
    };
});

pub(super) const ROW_COLUMN_HIERARCHY: ApiClass = class!("RowColumnPivotHierarchy", "A field in the rows or in the columns of a PivotTable.", {
    getId "(): string" "The identifier of the hierarchy: the place of its field in the source." => |h, c| Ok(field(h, c)?.1.to_string().into());
    getName "(): string" "The name of the field, as the PivotTable shows it." => |h, c| {
        let (p, field) = field(h, c)?;
        Ok(p.field_caption(field).to_string().into())
    };
    setName "(name: string): void" "Gives the field another name in this PivotTable." => |h, c| rename_field(h, c);
    getFields "(): PivotField[]" "The field of the hierarchy: one, in an array." => |h, c| Ok(Ret::Object(J::from(vec![field_object(h, c)?])));
    getPivotField "(name: string): PivotField | undefined" "The field of the hierarchy when it has this name; undefined otherwise." => |h, c| {
        let (p, at) = field(h, c)?;
        Ok(Ret::maybe(p.field_caption(at).eq_ignore_ascii_case(c.str(0)?).then(|| sub_object(c, "PivotField", &[at]))))
    };
    getPosition "(): number" "The place of the field among those of its area, 0 for the outermost." => |h, c| Ok((position(h, c, None)?.1 as u64).into());
    setPosition "(position: number): void" "Moves the field among those of its area." => |h, c| set_position(h, c, None);
    setToDefault "(): void" "Takes the field out of the rows or the columns." => |h, c| set_to_default(h, c, None);
});

pub(super) const FILTER_HIERARCHY: ApiClass = class!("FilterPivotHierarchy", "A field in the filters of a PivotTable.", {
    getId "(): string" "The identifier of the hierarchy: the place of its field in the source." => |h, c| Ok(field(h, c)?.1.to_string().into());
    getName "(): string" "The name of the field, as the PivotTable shows it." => |h, c| {
        let (p, field) = field(h, c)?;
        Ok(p.field_caption(field).to_string().into())
    };
    setName "(name: string): void" "Gives the field another name in this PivotTable." => |h, c| rename_field(h, c);
    getFields "(): PivotField[]" "The field of the hierarchy: one, in an array." => |h, c| Ok(Ret::Object(J::from(vec![field_object(h, c)?])));
    getPivotField "(name: string): PivotField | undefined" "The field of the hierarchy when it has this name; undefined otherwise." => |h, c| {
        let (p, at) = field(h, c)?;
        Ok(Ret::maybe(p.field_caption(at).eq_ignore_ascii_case(c.str(0)?).then(|| sub_object(c, "PivotField", &[at]))))
    };
    getPosition "(): number" "The place of the field among the filters." => |h, c| Ok((position(h, c, Some(Area::Filters))?.1 as u64).into());
    setPosition "(position: number): void" "Moves the field among the filters." => |h, c| set_position(h, c, Some(Area::Filters));
    setToDefault "(): void" "Takes the field out of the filters." => |h, c| set_to_default(h, c, Some(Area::Filters));
    getEnableMultipleFilterItems "(): boolean" "Whether several items may be chosen: true unless one item is selected alone." => |h, c| {
        let (p, at) = field(h, c)?;
        Ok(p.page_fields.iter().any(|page| page.field == at && page.item.is_none()).into())
    };
    setEnableMultipleFilterItems "(enableMultipleFilterItems: boolean): void" "Select Multiple Items: turned off, the filter shows one item, the first that was shown; turned on, it keeps showing it." => |h, c| {
        let (p, at) = field(h, c)?;
        let multiple = c.bool(0)?;
        let single = p.page_fields.iter().find(|page| page.field == at).and_then(|page| page.item);
        let first = p.items_in_order(at).into_iter().find(|x| p.fields[at].items.iter().any(|it| it.x == *x && !it.hidden));
        match (multiple, single, first) {
            (true, Some(x), _) => update(h, c, |p| p.set_visible_items(at, &[x])),
            (false, None, Some(x)) => update(h, c, |p| p.set_page_item(at, Some(x))),
            _ => Ok(Ret::Nothing),
        }
    };
});

pub(super) const DATA_HIERARCHY: ApiClass = class!("DataPivotHierarchy", "A value field of a PivotTable: a field of the source summarised by a function.", {
    getId "(): string" "The identifier of the value field: its place among the values." => |h, c| Ok(data(h, c)?.1.to_string().into());
    getName "(): string" "The name of the value field (Sum of Sales)." => |h, c| {
        let (p, index) = data(h, c)?;
        Ok(p.data_fields[index].name.clone().into())
    };
    setName "(name: string): void" "Gives the value field a custom name, which must not be the name of a field." => |h, c| {
        let (_, index) = data(h, c)?;
        let name = c.str(0)?;
        update_checked(h, c, |p| p.set_data_name(index, name))
    };
    getField "(): PivotField" "The field of the source the values are taken from." => |h, c| {
        let (p, index) = data(h, c)?;
        Ok(Ret::Object(sub_object(c, "PivotField", &[p.data_fields[index].field])))
    };
    getPosition "(): number" "The place of the value field among the values." => |h, c| Ok((data(h, c)?.1 as u64).into());
    setPosition "(position: number): void" "Moves the value field among the values." => |h, c| {
        data(h, c)?;
        set_position(h, c, Some(Area::Values))
    };
    getSummarizeBy "(): ExcelScript.AggregationFunction" "The function the values are summarised by: Sum, Count, Average, Max, Min..." => |h, c| {
        let (p, index) = data(h, c)?;
        let function = p.data_fields[index].function;
        Ok(FUNCTIONS.iter().find(|(_, f)| *f == function).map_or("Unknown", |(name, _)| name).into())
    };
    setSummarizeBy "(summarizeBy: ExcelScript.AggregationFunction): void" "Summarize Values By: the name of the value field follows unless it was customised. Automatic gives the sum of numbers and the count of anything else." => |h, c| {
        let (p, index) = data(h, c)?;
        let name = enum_value(c, 0, "AggregationFunction")?.ok_or_else(|| c.invalid())?;
        let function = match name {
            "Automatic" if p.cache.is_numeric(p.cache.database_field(p.data_fields[index].field)) => Aggregation::Sum,
            "Automatic" => Aggregation::Count,
            _ => FUNCTIONS.iter().find(|(n, _)| *n == name).map(|(_, f)| *f).ok_or_else(|| c.invalid())?,
        };
        let locale = h.wb.locale;
        update(h, c, |p| p.set_function(index, function, locale))
    };
    getShowAs "(): ExcelScript.ShowAsRule" "Show Values As: { calculation, baseField?, baseItem? }." => |h, c| {
        let (p, index) = data(h, c)?;
        Ok(Ret::Object(show_as_out(c, &p.data_fields[index].show_as)))
    };
    setShowAs "(showAs: ExcelScript.ShowAsRule): void" "Show Values As: a calculation of ExcelScript.ShowAsCalculation, with the base field and the base item those that need one take." => |h, c| {
        let (_, index) = data(h, c)?;
        let rule = show_as(c, c.arg(0))?;
        update(h, c, |p| p.data_fields[index].show_as = rule)
    };
    getNumberFormat "(): string" "The number format code of the values, in English; empty text when they have none of their own." => |h, c| {
        let (p, index) = data(h, c)?;
        Ok(p.data_fields[index].num_fmt.as_deref().unwrap_or_default().into())
    };
    setNumberFormat "(numberFormat: string): void" "Gives the values a number format code, in English; empty text removes it." => |h, c| {
        let (_, index) = data(h, c)?;
        let code = c.str(0)?;
        update(h, c, |p| p.data_fields[index].num_fmt = (!code.is_empty()).then(|| Arc::from(code)))
    };
    setToDefault "(): void" "Removes the value field." => |h, c| {
        data(h, c)?;
        set_to_default(h, c, Some(Area::Values))
    };
});

pub(super) const PIVOT_FIELD: ApiClass = class!("PivotField", "A field of a PivotTable: its items, how they are sorted, filtered and subtotalled.", {
    getId "(): string" "The identifier of the field: its place in the source." => |h, c| Ok(field(h, c)?.1.to_string().into());
    getName "(): string" "The name of the field, as the PivotTable shows it." => |h, c| {
        let (p, field) = field(h, c)?;
        Ok(p.field_caption(field).to_string().into())
    };
    setName "(name: string): void" "Gives the field another name in this PivotTable." => |h, c| rename_field(h, c);
    getItems "(): PivotItem[]" "The items of the field, in the order of its list." => |h, c| {
        let (p, at) = field(h, c)?;
        Ok(Ret::Object(J::from(p.items_in_order(at).into_iter().map(|x| sub_object(c, "PivotItem", &[at, x as usize])).collect::<Vec<J>>())))
    };
    getPivotItem "(name: string): PivotItem | undefined" "The item with this label; undefined when the field has none." => |h, c| {
        let (p, at) = field(h, c)?;
        let (name, locale) = (c.str(0)?, h.wb.locale);
        let found = p.fields[at].items.iter().map(|it| it.x).find(|x| p.item_text(at, *x, locale).eq_ignore_ascii_case(name));
        Ok(Ret::maybe(found.map(|x| sub_object(c, "PivotItem", &[at, x as usize]))))
    };
    getShowAllItems "(): boolean" "Whether items without data are shown." => |h, c| {
        let (p, at) = field(h, c)?;
        Ok(p.fields[at].show_all.into())
    };
    setShowAllItems "(showAllItems: boolean): void" "Show items with no data." => |h, c| {
        let (_, at) = field(h, c)?;
        let show = c.bool(0)?;
        update(h, c, |p| p.fields[at].show_all = show)
    };
    getSubtotals "(): ExcelScript.Subtotals" "The subtotals of the field: { automatic, sum, count, average, max, min, product, countNumbers, standardDeviation, standardDeviationP, variance, varianceP }." => |h, c| {
        let (p, at) = field(h, c)?;
        let subtotals = &p.fields[at].subtotals;
        let mut out = json!({ "automatic": *subtotals == Subtotals::Automatic });
        for (key, function) in SUBTOTAL_KEYS {
            out[key] = matches!(subtotals, Subtotals::Custom(list) if list.contains(&function)).into();
        }
        Ok(Ret::Value(out))
    };
    setSubtotals "(subtotals: ExcelScript.Subtotals): void" "Sets the subtotals of the field: automatic, the functions named true, or none when nothing is true." => |h, c| {
        let (_, at) = field(h, c)?;
        let given = c.arg(0);
        if !given.is_object() {
            return Err(c.invalid());
        }
        let on = |key: &str| given.get(key).and_then(J::as_bool).unwrap_or(false);
        let custom: Vec<Aggregation> = SUBTOTAL_KEYS.iter().filter(|(key, _)| on(key)).map(|(_, function)| *function).collect();
        let subtotals = match (on("automatic"), custom.is_empty()) {
            (true, _) => Subtotals::Automatic,
            (false, true) => Subtotals::None,
            (false, false) => Subtotals::Custom(custom),
        };
        update(h, c, |p| p.fields[at].subtotals = subtotals)
    };
    sortByLabels "(sortBy: ExcelScript.SortBy): void" "Sorts the items by their labels, ascending or descending." => |h, c| {
        let (_, at) = field(h, c)?;
        let order = if enum_value(c, 0, "SortBy")?.ok_or_else(|| c.invalid())? == "Descending" { SortOrder::Descending } else { SortOrder::Ascending };
        update(h, c, |p| p.sort_field(at, order, None))
    };
    sortByValues "(sortBy: ExcelScript.SortBy, valuesHierarchy: DataPivotHierarchy, pivotItemScope?: Array<PivotItem | string>): void" "Sorts the items by a value field, on its grand totals; sorting within the scope of other items is not supported." => |h, c| {
        let (_, at) = field(h, c)?;
        let order = if enum_value(c, 0, "SortBy")?.ok_or_else(|| c.invalid())? == "Descending" { SortOrder::Descending } else { SortOrder::Ascending };
        let by = hierarchy_arg(c, 1, &["DataPivotHierarchy"])?;
        if c.arg(2).as_array().is_some_and(|scope| !scope.is_empty()) {
            return Err(c.unsupported("PivotField.sortByValues within a scope of items"));
        }
        update(h, c, |p| p.sort_field(at, order, Some(by)))
    };
    applyFilter "(filter: ExcelScript.PivotFilters): void" "Filters the field: { manualFilter: { selectedItems } }, { labelFilter: { condition, ... } } or { valueFilter: { condition, value, ... } }. Date filters are not supported." => |h, c| {
        let (p, at) = field(h, c)?;
        let given = c.arg(0);
        let has = |key: &str| given.get(key).filter(|value| !value.is_null());
        if has("dateFilter").is_some() {
            return Err(c.unsupported("A date filter of a PivotTable field (PivotDateFilter)"));
        }
        let manual = has("manualFilter").map(|filter| chosen_items(h, c, p, at, filter)).transpose()?;
        let label = has("labelFilter").map(|filter| label_filter(c, filter)).transpose()?;
        let value = has("valueFilter").map(|filter| value_filter(c, p, filter)).transpose()?;
        if manual.is_none() && label.is_none() && value.is_none() {
            return Err(c.invalid());
        }
        update(h, c, |p| {
            if let Some(visible) = manual {
                p.set_visible_items(at, &visible);
            }
            if let Some(filter) = label {
                p.set_label_filter(at, Some(filter));
            }
            if let Some(filter) = value {
                p.set_value_filter(at, Some(filter));
            }
        })
    };
    clearAllFilters "(): void" "Removes every filter of the field." => |h, c| {
        let (_, at) = field(h, c)?;
        update(h, c, |p| {
            p.clear_filters(at);
            p.fields[at].items.iter_mut().for_each(|it| it.hidden = false);
        })
    };
    clearFilter "(filterType: ExcelScript.PivotFilterType): void" "Removes the filter of one kind: Manual, Label, Value or Date." => |h, c| {
        let (_, at) = field(h, c)?;
        let kind = enum_value(c, 0, "PivotFilterType")?.ok_or_else(|| c.invalid())?;
        update(h, c, |p| match kind {
            "Manual" => {
                p.set_page_item(at, None);
            }
            "Label" => p.set_label_filter(at, None),
            "Value" => p.set_value_filter(at, None),
            "Date" => p.set_date_filter(at, None),
            _ => {}
        })
    };
    isFiltered "(filterType?: ExcelScript.PivotFilterType): boolean" "Whether the field is filtered, or filtered in one way: Manual, Label, Value or Date." => |h, c| {
        let (p, at) = field(h, c)?;
        let (manual, label, value) = filtered(p, at);
        Ok(match enum_value(c, 0, "PivotFilterType")? {
            Some("Manual") => manual,
            Some("Label") => label,
            Some("Value") => value,
            Some("Date") => p.date_filter(at).is_some(),
            _ => p.is_filtered(at),
        }.into())
    };
});

pub(super) const PIVOT_ITEM: ApiClass = class!("PivotItem", "An item of a field of a PivotTable: one of its distinct values.", {
    getId "(): string" "The identifier of the item in its field." => |h, c| Ok(item(h, c)?.2.to_string().into());
    getName "(): string" "The label of the item, as the report shows it." => |h, c| {
        let (p, field, x) = item(h, c)?;
        Ok(p.item_text(field, x, h.wb.locale).into())
    };
    setName "(name: string): void" "Types a label over the item's; empty text gives its own back." => |h, c| {
        let (_, field, x) = item(h, c)?;
        let (name, locale) = (c.str(0)?, h.wb.locale);
        update_checked(h, c, |p| p.set_item_caption(field, x, name, locale))
    };
    getVisible "(): boolean" "Whether the item is checked in the list of its field." => |h, c| {
        let (p, field, x) = item(h, c)?;
        Ok(p.fields[field].items.iter().any(|it| it.x == x && !it.hidden).into())
    };
    setVisible "(visible: boolean): void" "Checks or unchecks the item in the list of its field." => |h, c| {
        let (_, field, x) = item(h, c)?;
        let visible = c.bool(0)?;
        update(h, c, |p| p.set_item_hidden(field, x, !visible))
    };
    getIsExpanded "(): boolean" "Whether the item shows the items under it." => |h, c| {
        let (p, field, x) = item(h, c)?;
        Ok(p.fields[field].items.iter().any(|it| it.x == x && !it.collapsed).into())
    };
    setIsExpanded "(isExpanded: boolean): void" "Expands or collapses the item." => |h, c| {
        let (_, field, x) = item(h, c)?;
        let expanded = c.bool(0)?;
        update(h, c, |p| p.set_collapsed(field, x, !expanded))
    };
});

/// A part of the report as a range, when the report has that part.
fn layout_range(h: &Host, call: &Call, part: fn(&PivotTable, Rect) -> Option<Rect>) -> Res<Ret> {
    let (sheet, p) = pivot(h, call)?;
    let rect = p.location.and_then(|body| part(p, body));
    Ok(Ret::maybe(rect.map(|rect| h.range(sheet, rect))))
}

pub(super) const PIVOT_LAYOUT: ApiClass = class!("PivotLayout", "The layout of the report of a PivotTable.", {
    getLayoutType "(): ExcelScript.PivotLayoutType" "The form of the report: Compact, Outline or Tabular (that of its first field when they differ)." => |h, c| {
        Ok(match pivot(h, c)?.1.fields.first().map(|f| f.layout).unwrap_or_default() {
            FieldLayout::Compact => "Compact",
            FieldLayout::Outline => "Outline",
            FieldLayout::Tabular => "Tabular",
        }.into())
    };
    setLayoutType "(layoutType: ExcelScript.PivotLayoutType): void" "Report Layout: Compact, Outline or Tabular form, for every field." => |h, c| {
        let layout = match enum_value(c, 0, "PivotLayoutType")?.ok_or_else(|| c.invalid())? {
            "Outline" => FieldLayout::Outline,
            "Tabular" => FieldLayout::Tabular,
            _ => FieldLayout::Compact,
        };
        update(h, c, |p| p.set_report_layout(layout))
    };
    getSubtotalLocation "(): ExcelScript.SubtotalLocationType" "Where groups show their subtotal: AtTop, AtBottom or Off (that of the first field when they differ)." => |h, c| {
        Ok(match pivot(h, c)?.1.fields.first() {
            Some(f) if f.subtotals == Subtotals::None => "Off",
            Some(f) if f.subtotal_top => "AtTop",
            _ => "AtBottom",
        }.into())
    };
    setSubtotalLocation "(subtotalLocation: ExcelScript.SubtotalLocationType): void" "Subtotals: at the top of each group, at its bottom, or none." => |h, c| {
        let (show, top) = match enum_value(c, 0, "SubtotalLocationType")?.ok_or_else(|| c.invalid())? {
            "AtTop" => (true, true),
            "AtBottom" => (true, false),
            _ => (false, false),
        };
        update(h, c, |p| p.set_subtotals(show, top))
    };
    getShowRowGrandTotals "(): boolean" "Whether each row ends with its grand total." => |h, c| Ok(pivot(h, c)?.1.options.row_grand_totals.into());
    setShowRowGrandTotals "(showRowGrandTotals: boolean): void" "Grand totals for rows: on or off." => |h, c| {
        let show = c.bool(0)?;
        update(h, c, |p| p.options.row_grand_totals = show)
    };
    getShowColumnGrandTotals "(): boolean" "Whether each column ends with its grand total." => |h, c| Ok(pivot(h, c)?.1.options.col_grand_totals.into());
    setShowColumnGrandTotals "(showColumnGrandTotals: boolean): void" "Grand totals for columns: on or off." => |h, c| {
        let show = c.bool(0)?;
        update(h, c, |p| p.options.col_grand_totals = show)
    };
    getShowFieldHeaders "(): boolean" "Whether the captions of the fields and their filter buttons show." => |h, c| Ok(pivot(h, c)?.1.options.show_headers.into());
    setShowFieldHeaders "(showFieldHeaders: boolean): void" "Field Headers: on or off." => |h, c| {
        let show = c.bool(0)?;
        update(h, c, |p| p.options.show_headers = show)
    };
    getFillEmptyCells "(): boolean" "Whether empty cells of the report show a text of their own." => |h, c| Ok(pivot(h, c)?.1.options.show_missing.into());
    setFillEmptyCells "(fillEmptyCells: boolean): void" "For empty cells show: on or off." => |h, c| {
        let fill = c.bool(0)?;
        update(h, c, |p| p.options.show_missing = fill)
    };
    getEmptyCellText "(): string" "The text empty cells of the report show." => |h, c| Ok(pivot(h, c)?.1.options.missing_caption.clone().into());
    setEmptyCellText "(emptyCellText: string): void" "Sets the text empty cells show, when they show one." => |h, c| {
        let text = c.str(0)?;
        update(h, c, |p| p.options.missing_caption = text.to_string())
    };
    repeatAllItemLabels "(repeatLabels: boolean): void" "Repeat All Item Labels, or do not repeat them." => |h, c| {
        let repeat = c.bool(0)?;
        update(h, c, |p| p.set_repeat_labels(repeat))
    };
    displayBlankLineAfterEachItem "(display: boolean): void" "Inserts a blank line after each item, or removes them." => |h, c| {
        let blank = c.bool(0)?;
        update(h, c, |p| p.set_blank_lines(blank))
    };
    getRange "(): Range" "The cells of the report, without its filters." => |h, c| layout_range(h, c, |_, body| Some(body));
    getFilterAxisRange "(): Range | undefined" "The cells of the filters above the report; undefined without filter fields." => |h, c| {
        let (sheet, p) = pivot(h, c)?;
        Ok(Ret::maybe(p.page_location.map(|rect| h.range(sheet, rect))))
    };
    getBodyAndTotalRange "(): Range | undefined" "The cells of the values, grand totals included; undefined for a report without values." => |h, c| {
        layout_range(h, c, |p, body| {
            let (top, left) = (body.r0 + p.report.first_data_row(), body.c0 + p.report.first_data_col());
            (!p.data_fields.is_empty() && top <= body.r1 && left <= body.c1).then(|| Rect::new(top, left, body.r1, body.c1))
        })
    };
    getRowLabelRange "(): Range | undefined" "The cells of the row labels; undefined for a report without row fields." => |h, c| {
        layout_range(h, c, |p, body| {
            let (top, width) = (body.r0 + p.report.first_data_row(), p.report.first_data_col());
            (width > 0 && top <= body.r1).then(|| Rect::new(top, body.c0, body.r1, body.c0 + width - 1))
        })
    };
    getColumnLabelRange "(): Range | undefined" "The cells of the column labels; undefined for a report without header rows." => |h, c| {
        layout_range(h, c, |p, body| {
            let (first, rows, left) = (p.report.first_header_row(), p.report.first_data_row(), body.c0 + p.report.first_data_col());
            (rows > first && left <= body.c1).then(|| Rect::new(body.r0 + first, left, body.r0 + rows - 1, body.c1))
        })
    };
});
