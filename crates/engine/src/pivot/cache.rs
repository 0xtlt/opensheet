//! The pivot cache: a copy of the source records taken when the PivotTable
//! is created or refreshed (`pivotCacheDefinition` + `pivotCacheRecords`).
//!
//! Every field keeps the distinct values of its column (`shared`) and, for
//! each record, the index of its value. Grouping lives here too, as in the
//! file format: a number field grouped by interval carries its groups
//! itself; grouping dates or hand-picked items adds a field derived from
//! the grouped one.

use super::text::{self, Caption};
use super::{PivotError, PivotSource};
use crate::datetime::serial_to_ymd;
use crate::format;
use crate::locale::Locale;
use crate::refs::Rect;
use crate::value::{ErrorKind, Value};
use crate::workbook::{Content, Workbook};
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

/// What a range grouping groups by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum GroupBy {
    /// Numbers by interval.
    Range,
    Seconds,
    Minutes,
    Hours,
    Days,
    Months,
    Quarters,
    Years,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GroupKind {
    /// Numbers by interval or dates by period (`rangePr`).
    Range {
        by: GroupBy,
        start: f64,
        end: f64,
        /// Width of a number group; number of days per group for `Days`.
        interval: f64,
        /// "Starting at" / "Ending at" follow the data at every refresh.
        auto_start: bool,
        auto_end: bool,
    },
    /// Items put together by hand (`discretePr`).
    Discrete,
}

/// How the items of a field are grouped (`fieldGroup`).
#[derive(Clone, Debug, PartialEq)]
pub struct FieldGroup {
    /// The field whose items are grouped; `None` when the field groups its
    /// own source values (numbers by interval).
    pub base: Option<usize>,
    pub kind: GroupKind,
    /// The groups, in their natural order (`groupItems`).
    pub items: Vec<Value>,
    /// For every item of the grouped field, the group it falls into.
    pub map: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CacheField {
    /// The column header; unique in the cache, compared without case.
    pub name: String,
    /// The distinct values of the column, in order of first appearance
    /// (`sharedItems`); `Value::Empty` stands for empty cells. Empty for a
    /// field derived from another one by grouping.
    pub shared: Vec<Value>,
    /// Index into `shared` of every record. Empty for a derived field.
    pub records: Vec<u32>,
    /// Number format of the source column: dates are labelled with it.
    pub num_fmt: Arc<str>,
    pub group: Option<FieldGroup>,
    /// The formula of a calculated field, in English, naming the fields
    /// it computes from (`Sales-Cost`, `'Unit Price'*Units`). Such a field
    /// has no values of its own: it is only ever a value field.
    pub formula: Option<String>,
}

/// A calculated item: an item added to a field, computed from the other
/// items of that field (`calculatedItem`).
#[derive(Clone, Debug, PartialEq)]
pub struct CalcItem {
    /// The field the item belongs to.
    pub field: usize,
    /// Its label: the item is the one of `CacheField::shared` reading so.
    pub name: String,
    /// In English, naming items of the field (`East+West`).
    pub formula: String,
}

impl CacheField {
    /// False for a field created by grouping another one and for a
    /// calculated field: they have no column in the source
    /// (`databaseField="0"`).
    pub fn is_database(&self) -> bool {
        self.formula.is_none() && self.group.as_ref().is_none_or(|g| g.base.is_none())
    }

    /// The items the field shows in a report: its groups when it is
    /// grouped, the distinct source values otherwise.
    pub fn items(&self) -> &[Value] {
        match &self.group {
            Some(g) => &g.items,
            None => &self.shared,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct PivotCache {
    pub fields: Vec<CacheField>,
    /// Number of source records.
    pub rows: usize,
    /// The calculated items, in solve order: where two of them meet, the
    /// later one gives the value.
    pub calc_items: Vec<CalcItem>,
}

/// Identity of an item: text compares without case, as Excel merges
/// "east" and "East" into one item.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ItemKey {
    Blank,
    Number(u64),
    Text(String),
    Bool(bool),
    Error(ErrorKind),
}

pub(crate) fn item_key(v: &Value) -> ItemKey {
    match v {
        Value::Empty => ItemKey::Blank,
        // Adding zero folds -0 into 0.
        Value::Number(n) => ItemKey::Number((n + 0.0).to_bits()),
        Value::Text(s) if s.is_empty() => ItemKey::Blank,
        Value::Text(s) => ItemKey::Text(s.to_lowercase()),
        Value::Bool(b) => ItemKey::Bool(*b),
        Value::Error(e) => ItemKey::Error(*e),
        Value::Array(a) => item_key(a.get(0, 0)),
    }
}

/// Collects the distinct values of one column.
#[derive(Default)]
struct Interner {
    /// Text seen with this exact spelling: spares lowercasing every cell.
    exact: HashMap<Arc<str>, u32>,
    keys: HashMap<ItemKey, u32>,
    items: Vec<Value>,
}

impl Interner {
    fn intern(&mut self, v: &Value) -> u32 {
        if let Value::Text(s) = v
            && let Some(i) = self.exact.get(s)
        {
            return *i;
        }
        let key = item_key(v);
        let next = self.items.len() as u32;
        let idx = *self.keys.entry(key).or_insert(next);
        if idx == next {
            self.items.push(match v {
                Value::Text(s) if s.is_empty() => Value::Empty,
                Value::Array(a) => a.get(0, 0).clone(),
                other => other.clone(),
            });
        }
        if let Value::Text(s) = v {
            self.exact.insert(s.clone(), idx);
        }
        idx
    }
}

/// The name itself, or the name followed by the first number that makes it
/// unique among `taken` ("Sales2").
fn unique_name(name: &str, taken: &[String]) -> String {
    let used = |n: &str| taken.iter().any(|t| t.to_lowercase() == n.to_lowercase());
    if !used(name) {
        return name.to_string();
    }
    let mut k = 2;
    loop {
        let candidate = format!("{name}{k}");
        if !used(&candidate) {
            return candidate;
        }
        k += 1;
    }
}

/// More groups than this is taken for a mistake in the interval.
const MAX_GROUPS: f64 = 100_000.0;

impl PivotCache {
    /// A cache made of columns of values, one per field (used when a file
    /// stores the records inline, and by tests).
    pub fn from_columns(fields: Vec<(String, Arc<str>)>, columns: Vec<Vec<Value>>) -> PivotCache {
        let rows = columns.iter().map(Vec::len).max().unwrap_or(0);
        let mut names: Vec<String> = Vec::new();
        let mut out = Vec::new();
        for (k, (name, num_fmt)) in fields.into_iter().enumerate() {
            let mut interner = Interner::default();
            let column = columns.get(k).map(Vec::as_slice).unwrap_or(&[]);
            let records = (0..rows)
                .map(|r| interner.intern(column.get(r).unwrap_or(&Value::Empty)))
                .collect();
            let name = unique_name(&name, &names);
            names.push(name.clone());
            out.push(CacheField {
                name,
                shared: interner.items,
                records,
                num_fmt,
                group: None,
                formula: None,
            });
        }
        PivotCache {
            fields: out,
            rows,
            calc_items: Vec::new(),
        }
    }

    /// Read the source records. Fails as Excel does when the source is not
    /// a list with a header row: a missing header, or no record at all.
    pub(crate) fn read(wb: &Workbook, source: &PivotSource) -> Result<PivotCache, PivotError> {
        let (si, rect) = source.resolve(wb)?;
        let sh = &wb.sheets[si];
        // A whole-column or whole-row source stops at the data.
        let (er, ec) = sh.content_extent();
        let rect = Rect::new(
            rect.r0,
            rect.c0,
            rect.r1.min(er.max(rect.r0)),
            rect.c1.min(ec.max(rect.c0)),
        );
        if rect.r1 == rect.r0 {
            return Err(PivotError::InvalidSource);
        }
        let n = (rect.r1 - rect.r0) as usize;
        let ncols = rect.cols() as usize;
        let mut names: Vec<String> = Vec::with_capacity(ncols);
        for c in rect.c0..=rect.c1 {
            let text = wb.display_text(si, rect.r0, c).text;
            if text.trim().is_empty() {
                return Err(PivotError::EmptyFieldName);
            }
            let name = unique_name(text.trim(), &names);
            names.push(name);
        }
        let mut interners: Vec<Interner> = (0..ncols).map(|_| Interner::default()).collect();
        let mut records: Vec<Vec<u32>> = vec![vec![u32::MAX; n]; ncols];
        let mut formats: Vec<Option<Arc<str>>> = vec![None; ncols];
        for ((r, c), cell) in sh.cells.range((rect.r0 + 1, 0)..=(rect.r1, u32::MAX)) {
            if *c < rect.c0 || *c > rect.c1 || matches!(cell.content, Content::Empty) {
                continue;
            }
            let k = (*c - rect.c0) as usize;
            records[k][(*r - rect.r0 - 1) as usize] = interners[k].intern(&cell.value);
            if formats[k].is_none() && cell.value.is_number() {
                formats[k] = Some(wb.styles.get(cell.style).num_fmt.clone());
            }
        }
        // Results of dynamic arrays are not cells of their own.
        let data = Rect::new(rect.r0 + 1, rect.c0, rect.r1, rect.c1);
        for sp in &sh.spills {
            let area = Rect::new(
                sp.row,
                sp.col,
                sp.row + sp.array.rows as u32 - 1,
                sp.col + sp.array.cols as u32 - 1,
            );
            let Some(overlap) = area.intersection(&data) else {
                continue;
            };
            for r in overlap.r0..=overlap.r1 {
                for c in overlap.c0..=overlap.c1 {
                    if (r, c) == (sp.row, sp.col) {
                        continue;
                    }
                    let v = sp.array.get((r - sp.row) as usize, (c - sp.col) as usize);
                    let k = (c - rect.c0) as usize;
                    records[k][(r - rect.r0 - 1) as usize] = interners[k].intern(v);
                }
            }
        }
        let general: Arc<str> = Arc::from("General");
        let mut fields = Vec::with_capacity(ncols);
        for (k, name) in names.into_iter().enumerate() {
            let mut interner = std::mem::take(&mut interners[k]);
            let mut column = std::mem::take(&mut records[k]);
            if column.contains(&u32::MAX) {
                let blank = interner.intern(&Value::Empty);
                for x in &mut column {
                    if *x == u32::MAX {
                        *x = blank;
                    }
                }
            }
            fields.push(CacheField {
                name,
                shared: interner.items,
                records: column,
                num_fmt: formats[k].take().unwrap_or_else(|| general.clone()),
                group: None,
                formula: None,
            });
        }
        Ok(PivotCache {
            fields,
            rows: n,
            calc_items: Vec::new(),
        })
    }

    /// Index of the field named `name`, compared without case.
    pub fn field_index(&self, name: &str) -> Option<usize> {
        let wanted = name.to_lowercase();
        self.fields.iter().position(|f| f.name.to_lowercase() == wanted)
    }

    /// The items of a field as a report shows them.
    pub fn items(&self, field: usize) -> &[Value] {
        self.fields.get(field).map(CacheField::items).unwrap_or(&[])
    }

    /// For every record, the index of its item in `items(field)`.
    pub fn item_column(&self, field: usize) -> Cow<'_, [u32]> {
        let Some(f) = self.fields.get(field) else {
            return Cow::Owned(vec![0; self.rows]);
        };
        match &f.group {
            None => Cow::Borrowed(&f.records),
            Some(g) => {
                let base = match g.base {
                    Some(b) if b != field => self.item_column(b),
                    _ => Cow::Borrowed(&f.records[..]),
                };
                Cow::Owned(
                    base.iter()
                        .map(|&i| g.map.get(i as usize).copied().unwrap_or(0))
                        .collect(),
                )
            }
        }
    }

    /// The database field a field takes its values from: itself, or the
    /// field at the bottom of its chain of groupings.
    pub fn database_field(&self, field: usize) -> usize {
        let mut f = field;
        for _ in 0..self.fields.len() {
            match self.fields.get(f).and_then(|x| x.group.as_ref()).and_then(|g| g.base) {
                Some(b) if b != f => f = b,
                _ => break,
            }
        }
        f
    }

    /// The fields derived from `field` by grouping.
    pub fn group_children(&self, field: usize) -> Vec<usize> {
        (0..self.fields.len())
            .filter(|&k| k != field && self.fields[k].group.as_ref().is_some_and(|g| g.base == Some(field)))
            .collect()
    }

    /// True when every value of the field is a number: Excel then sums it
    /// by default, and counts it otherwise.
    pub fn is_numeric(&self, field: usize) -> bool {
        // A calculated field computes numbers.
        self.fields
            .get(field)
            .is_some_and(|f| f.formula.is_some() || (!f.shared.is_empty() && f.shared.iter().all(Value::is_number)))
    }

    /// True for a numeric field formatted as dates or times.
    pub fn is_date(&self, field: usize) -> bool {
        self.is_numeric(field) && format::is_date_format(&self.fields[field].num_fmt)
    }

    /// False when the indices of the cache do not agree with each other,
    /// which only a damaged file can produce.
    pub fn is_valid(&self) -> bool {
        self.fields.iter().enumerate().all(|(k, f)| {
            let own = f.records.iter().all(|&i| (i as usize) < f.shared.len())
                && (f.records.len() == self.rows || (f.records.is_empty() && !f.is_database()));
            let group = match &f.group {
                None => true,
                Some(g) => {
                    let base_len = match g.base {
                        // A derived field must come after the field it groups.
                        Some(b) if b < k => self.items(b).len(),
                        Some(_) => return false,
                        None => f.shared.len(),
                    };
                    g.map.len() == base_len && g.map.iter().all(|&i| (i as usize) < g.items.len())
                }
            };
            own && group
        })
    }

    fn names(&self) -> Vec<String> {
        self.fields.iter().map(|f| f.name.clone()).collect()
    }

    /// The numbers of a field that can be grouped by range: Excel refuses
    /// ("Cannot group that selection") as soon as one value is not a number.
    fn numbers(&self, field: usize) -> Result<Vec<f64>, PivotError> {
        let f = self.fields.get(field).ok_or(PivotError::NoSuchField)?;
        if !f.is_database() || f.shared.is_empty() {
            return Err(PivotError::CannotGroup);
        }
        f.shared
            .iter()
            .map(|v| match v {
                Value::Number(n) => Ok(*n),
                _ => Err(PivotError::CannotGroup),
            })
            .collect()
    }

    /// Group a number field by interval ("Starting at", "Ending at", "By");
    /// `None` bounds follow the data.
    pub(crate) fn with_number_groups(
        &self,
        field: usize,
        start: Option<f64>,
        end: Option<f64>,
        interval: f64,
        locale: &Locale,
    ) -> Result<PivotCache, PivotError> {
        let numbers = self.numbers(field)?;
        let kind = GroupKind::Range {
            by: GroupBy::Range,
            start: start.unwrap_or(0.0),
            end: end.unwrap_or(0.0),
            interval,
            auto_start: start.is_none(),
            auto_end: end.is_none(),
        };
        let mut out = self.without_groups_of(field);
        let f = out
            .field_index(&self.fields[field].name)
            .ok_or(PivotError::NoSuchField)?;
        out.fields[f].group = Some(range_group(kind, None, &numbers, locale)?);
        Ok(out)
    }

    /// Group a date field by the given periods. Every period becomes a new
    /// field ("Years (Date)"); they are returned outermost first.
    pub(crate) fn with_date_groups(
        &self,
        field: usize,
        periods: &[GroupBy],
        start: Option<f64>,
        end: Option<f64>,
        days: u32,
        locale: &Locale,
    ) -> Result<(PivotCache, Vec<usize>), PivotError> {
        let numbers = self.numbers(field)?;
        if numbers.iter().any(|n| *n < 0.0) || periods.is_empty() || periods.contains(&GroupBy::Range) {
            return Err(PivotError::CannotGroup);
        }
        let base_name = self.fields[field].name.clone();
        let mut out = self.without_groups_of(field);
        let base = out.field_index(&base_name).ok_or(PivotError::NoSuchField)?;
        let mut periods = periods.to_vec();
        periods.sort();
        periods.dedup();
        periods.reverse();
        let mut created = Vec::new();
        for by in periods {
            let kind = GroupKind::Range {
                by,
                start: start.unwrap_or(0.0),
                end: end.unwrap_or(0.0),
                interval: if by == GroupBy::Days { days.max(1) as f64 } else { 1.0 },
                auto_start: start.is_none(),
                auto_end: end.is_none(),
            };
            let group = range_group(kind, Some(base), &numbers, locale)?;
            let name = unique_name(&text::date_group_field_name(by, &base_name, locale), &out.names());
            created.push(out.fields.len());
            out.fields.push(CacheField {
                name,
                shared: Vec::new(),
                records: Vec::new(),
                num_fmt: Arc::from("General"),
                group: Some(group),
                formula: None,
            });
        }
        Ok((out, created))
    }

    /// The periods Excel groups a date field by on its own when the field
    /// is added to the rows or columns: years, quarters and months for
    /// dates spanning several years, months within one year, hours and
    /// minutes for times of day. Empty when the field is left as it is.
    pub fn auto_date_periods(&self, field: usize) -> Vec<GroupBy> {
        if !self.is_date(field) || self.fields[field].group.is_some() || !self.group_children(field).is_empty() {
            return Vec::new();
        }
        let numbers: Vec<f64> = self.fields[field]
            .shared
            .iter()
            .filter_map(|v| if let Value::Number(n) = v { Some(*n) } else { None })
            .collect();
        let min = numbers.iter().copied().fold(f64::INFINITY, f64::min);
        let max = numbers.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        if min < 0.0 {
            return Vec::new();
        }
        if max < 1.0 {
            return vec![GroupBy::Hours, GroupBy::Minutes];
        }
        let (y0, m0, _) = serial_to_ymd(min);
        let (y1, m1, _) = serial_to_ymd(max);
        if y0 != y1 {
            vec![GroupBy::Years, GroupBy::Quarters, GroupBy::Months]
        } else if m0 != m1 {
            vec![GroupBy::Months]
        } else {
            Vec::new()
        }
    }

    /// Put the given items of `field` together in a new group ("Group1").
    /// The groups live in a field derived from `field` ("Region2"), created
    /// on first use; its index is returned.
    pub(crate) fn with_item_group(
        &self,
        field: usize,
        selected: &[u32],
        locale: &Locale,
    ) -> Result<(PivotCache, usize), PivotError> {
        let base_items = self.items(field).to_vec();
        let mut selected: Vec<u32> = selected
            .iter()
            .copied()
            .filter(|&x| (x as usize) < base_items.len())
            .collect();
        selected.sort_unstable();
        selected.dedup();
        if selected.len() < 2 || self.fields.get(field).is_none() {
            return Err(PivotError::CannotGroup);
        }
        let mut out = self.clone();
        let existing = self.group_children(field).into_iter().find(|&k| {
            matches!(
                self.fields[k].group,
                Some(FieldGroup {
                    kind: GroupKind::Discrete,
                    ..
                })
            )
        });
        let prefix = text::caption(Caption::Group, locale);
        let group_name = |items: &[Value]| {
            let taken: Vec<String> = items
                .iter()
                .chain(&base_items)
                .filter_map(|v| {
                    if let Value::Text(s) = v {
                        Some(s.to_string())
                    } else {
                        None
                    }
                })
                .collect();
            let mut k = 1;
            loop {
                let name = format!("{prefix}{k}");
                if !taken.iter().any(|t| t.eq_ignore_ascii_case(&name)) {
                    return name;
                }
                k += 1;
            }
        };
        match existing {
            Some(parent) => {
                let g = out.fields[parent].group.as_mut().ok_or(PivotError::CannotGroup)?;
                let name = group_name(&g.items);
                // The new group takes the place of the first item it absorbs.
                let at = g.map[selected[0] as usize] as usize;
                g.items.insert(at, Value::text(name));
                for (i, m) in g.map.iter_mut().enumerate() {
                    if selected.contains(&(i as u32)) {
                        *m = at as u32;
                    } else if *m as usize >= at {
                        *m += 1;
                    }
                }
                compact(g);
                // Groups made of these groups no longer fit them.
                Ok((out.without_fields(&self.group_children(parent)), parent))
            }
            None => {
                let name = group_name(&[]);
                let mut items = Vec::new();
                let mut map = vec![0u32; base_items.len()];
                let mut group_at = None;
                for (i, v) in base_items.iter().enumerate() {
                    if selected.contains(&(i as u32)) {
                        let at = *group_at.get_or_insert_with(|| {
                            items.push(Value::text(&name));
                            items.len() as u32 - 1
                        });
                        map[i] = at;
                    } else {
                        map[i] = items.len() as u32;
                        items.push(v.clone());
                    }
                }
                // "Region2", then "Region3" for groups of its groups.
                let root = &self.fields[self.database_field(field)].name;
                let names = self.names();
                let taken = |n: &str| names.iter().any(|t| t.to_lowercase() == n.to_lowercase());
                let field_name = (2..)
                    .map(|k| format!("{root}{k}"))
                    .find(|n| !taken(n))
                    .unwrap_or_default();
                out.fields.push(CacheField {
                    name: field_name,
                    shared: Vec::new(),
                    records: Vec::new(),
                    num_fmt: self.fields[field].num_fmt.clone(),
                    group: Some(FieldGroup {
                        base: Some(field),
                        kind: GroupKind::Discrete,
                        items,
                        map,
                    }),
                    formula: None,
                });
                let parent = out.fields.len() - 1;
                Ok((out, parent))
            }
        }
    }

    /// Undo a grouping. For a field grouped by interval or by date periods,
    /// the whole grouping goes; for hand-made groups, `item` names the group
    /// to dissolve (all of them when `None`).
    pub(crate) fn without_group(&self, field: usize, item: Option<u32>) -> Result<PivotCache, PivotError> {
        let f = self.fields.get(field).ok_or(PivotError::NoSuchField)?;
        match &f.group {
            Some(g) if g.base.is_none() => Ok(self.without_groups_of(field)),
            Some(FieldGroup {
                base: Some(base),
                kind: GroupKind::Range { .. },
                ..
            }) => Ok(self.without_groups_of(*base)),
            Some(g) => {
                let Some(item) = item else {
                    return Ok(self.without_fields(&[field]));
                };
                let members: Vec<usize> = (0..g.map.len()).filter(|&i| g.map[i] == item).collect();
                if members.len() < 2 {
                    return Err(PivotError::CannotUngroup);
                }
                let base_items = self.items(g.base.unwrap_or(field)).to_vec();
                let mut out = self.clone();
                let Some(group) = out.fields[field].group.as_mut() else {
                    return Err(PivotError::CannotUngroup);
                };
                // Each member becomes an item of its own again, where the
                // group was.
                for (n, &i) in members.iter().enumerate() {
                    let at = item as usize + n;
                    if n == 0 {
                        group.items[at] = base_items[i].clone();
                    } else {
                        group.items.insert(at, base_items[i].clone());
                        for m in group.map.iter_mut() {
                            if *m as usize >= at {
                                *m += 1;
                            }
                        }
                    }
                    group.map[i] = at as u32;
                }
                // Without any group left, the field has no reason to be;
                // nor have, either way, the groups made of its groups.
                let grouped = (0..group.items.len() as u32).any(|x| group.map.iter().filter(|m| **m == x).count() > 1);
                if grouped {
                    Ok(out.without_fields(&self.group_children(field)))
                } else {
                    Ok(out.without_fields(&[field]))
                }
            }
            None => {
                if self.group_children(field).is_empty() {
                    return Err(PivotError::CannotUngroup);
                }
                Ok(self.without_groups_of(field))
            }
        }
    }

    /// The cache without the groupings of `field`: its own number groups
    /// and the fields derived from it.
    fn without_groups_of(&self, field: usize) -> PivotCache {
        let mut out = self.without_fields(&self.group_children(field));
        if let Some(f) = self.fields.get(field).and_then(|f| out.field_index(&f.name))
            && out.fields[f].is_database()
        {
            out.fields[f].group = None;
        }
        out
    }

    /// The cache without the given derived fields, nor the fields derived
    /// from those.
    fn without_fields(&self, remove: &[usize]) -> PivotCache {
        let mut gone = vec![false; self.fields.len()];
        for &k in remove {
            if k < gone.len() && !self.fields[k].is_database() {
                gone[k] = true;
            }
        }
        // Derived fields come after their base, so one pass is enough.
        for k in 0..self.fields.len() {
            if let Some(Some(b)) = self.fields[k].group.as_ref().map(|g| g.base)
                && gone[b]
            {
                gone[k] = true;
            }
        }
        let mut new_index = vec![0usize; self.fields.len()];
        let mut fields = Vec::new();
        for (k, f) in self.fields.iter().enumerate() {
            if gone[k] {
                continue;
            }
            new_index[k] = fields.len();
            let mut f = f.clone();
            if let Some(g) = &mut f.group
                && let Some(b) = g.base
            {
                g.base = Some(new_index[b]);
            }
            fields.push(f);
        }
        let calc_items = self
            .calc_items
            .iter()
            .filter(|item| !gone[item.field])
            .map(|item| CalcItem {
                field: new_index[item.field],
                ..item.clone()
            })
            .collect();
        PivotCache {
            fields,
            rows: self.rows,
            calc_items,
        }
    }

    /// Compute what a range grouping read from a file leaves out: which
    /// group each value falls into, and the groups themselves when the file
    /// did not list them. Call it on a cache built from a file before
    /// handing it to a PivotTable.
    pub fn complete_groups(&mut self, locale: &Locale) {
        for f in 0..self.fields.len() {
            let Some(g) = self.fields[f].group.clone() else {
                continue;
            };
            if g.kind == GroupKind::Discrete {
                continue;
            }
            let numbers: Vec<f64> = self
                .items_of_base(f, &g)
                .iter()
                .map(|v| if let Value::Number(n) = v { *n } else { f64::NAN })
                .collect();
            // The bounds are the file's, not the data's.
            let kind = match g.kind {
                GroupKind::Range {
                    by,
                    start,
                    end,
                    interval,
                    ..
                } => GroupKind::Range {
                    by,
                    start,
                    end,
                    interval,
                    auto_start: false,
                    auto_end: false,
                },
                GroupKind::Discrete => continue,
            };
            if let Ok(mut computed) = range_group(kind, g.base, &numbers, locale) {
                computed.kind = g.kind;
                if g.items.len() == computed.items.len() {
                    computed.items = g.items;
                }
                self.fields[f].group = Some(computed);
            }
        }
    }

    /// The items a grouping of field `f` maps from.
    fn items_of_base(&self, f: usize, g: &FieldGroup) -> &[Value] {
        match g.base {
            Some(b) if b != f => self.items(b),
            _ => &self.fields[f].shared,
        }
    }

    /// Give a cache freshly read from the source the groupings of the one
    /// it replaces: ranges are recomputed over the new values, hand-made
    /// groups keep their members and new values stay on their own.
    pub(crate) fn carry_groups(&mut self, old: &PivotCache, locale: &Locale) {
        for of in &old.fields {
            let Some(g) = &of.group else {
                continue;
            };
            match g.base {
                None => {
                    let Some(f) = self.field_index(&of.name) else {
                        continue;
                    };
                    if let Ok(numbers) = self.numbers(f)
                        && let Ok(group) = range_group(g.kind.clone(), None, &numbers, locale)
                    {
                        self.fields[f].group = Some(group);
                    }
                }
                Some(ob) => {
                    let Some(base) = old.fields.get(ob).and_then(|b| self.field_index(&b.name)) else {
                        continue;
                    };
                    let group = match &g.kind {
                        GroupKind::Range { .. } => {
                            let Ok(numbers) = self.numbers(base) else {
                                continue;
                            };
                            match range_group(g.kind.clone(), Some(base), &numbers, locale) {
                                Ok(group) => group,
                                Err(_) => continue,
                            }
                        }
                        GroupKind::Discrete => {
                            let by_key: HashMap<ItemKey, u32> = old
                                .items(ob)
                                .iter()
                                .zip(&g.map)
                                .map(|(v, m)| (item_key(v), *m))
                                .collect();
                            let mut items = g.items.clone();
                            let mut map = Vec::new();
                            for v in self.items(base) {
                                match by_key.get(&item_key(v)) {
                                    Some(m) => map.push(*m),
                                    None => {
                                        map.push(items.len() as u32);
                                        items.push(v.clone());
                                    }
                                }
                            }
                            let mut group = FieldGroup {
                                base: Some(base),
                                kind: GroupKind::Discrete,
                                items,
                                map,
                            };
                            compact(&mut group);
                            group
                        }
                    };
                    self.fields.push(CacheField {
                        name: of.name.clone(),
                        shared: Vec::new(),
                        records: Vec::new(),
                        num_fmt: of.num_fmt.clone(),
                        group: Some(group),
                        formula: None,
                    });
                }
            }
        }
    }
}

/// Drop the groups nothing falls into any more, keeping the order.
fn compact(g: &mut FieldGroup) {
    let mut used = vec![false; g.items.len()];
    for &m in &g.map {
        if let Some(u) = used.get_mut(m as usize) {
            *u = true;
        }
    }
    let mut new_index = vec![0u32; g.items.len()];
    let mut items = Vec::new();
    for (k, v) in g.items.iter().enumerate() {
        if used[k] {
            new_index[k] = items.len() as u32;
            items.push(v.clone());
        }
    }
    for m in &mut g.map {
        *m = new_index.get(*m as usize).copied().unwrap_or(0);
    }
    g.items = items;
}

/// Build a range grouping over the values of the grouped field. The first
/// and last groups collect what lies before the start and after the end
/// ("<10", ">100"), as in the file format.
fn range_group(
    kind: GroupKind,
    base: Option<usize>,
    numbers: &[f64],
    locale: &Locale,
) -> Result<FieldGroup, PivotError> {
    let GroupKind::Range {
        by,
        mut start,
        mut end,
        interval,
        auto_start,
        auto_end,
    } = kind
    else {
        return Err(PivotError::CannotGroup);
    };
    let min = numbers.iter().copied().fold(f64::INFINITY, f64::min);
    let max = numbers.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if auto_start {
        start = if by == GroupBy::Range { min } else { min.floor() };
    }
    if auto_end {
        end = if by == GroupBy::Range { max } else { max.floor() };
    }
    if !(start.is_finite() && end.is_finite() && interval.is_finite()) || end < start || interval <= 0.0 {
        return Err(PivotError::CannotGroup);
    }
    let mut items: Vec<Value> = Vec::new();
    let map: Vec<u32> = if by == GroupBy::Range {
        let bins = ((end - start) / interval).floor() + 1.0;
        if bins > MAX_GROUPS {
            return Err(PivotError::CannotGroup);
        }
        let bins = bins as u32;
        // Whole numbers are labelled "0-9", "10-19"; others "0-10", "10-20".
        let whole = numbers.iter().all(|n| n.fract() == 0.0) && start.fract() == 0.0 && interval.fract() == 0.0;
        let num = |n: f64| locale.number_to_text(n);
        items.push(Value::text(format!("<{}", num(start))));
        for k in 0..bins {
            let lo = start + k as f64 * interval;
            let hi = if whole { lo + interval - 1.0 } else { lo + interval };
            items.push(Value::text(format!("{}-{}", num(lo), num(hi))));
        }
        items.push(Value::text(format!(">{}", num(start + bins as f64 * interval))));
        numbers
            .iter()
            .map(|&n| {
                if n < start {
                    0
                } else {
                    (((n - start) / interval).floor() as u32).min(bins) + 1
                }
            })
            .collect()
    } else {
        let first_day = start.floor();
        let last_day = end.floor();
        items.push(Value::text(format!("<{}", text::short_date(first_day, locale))));
        let inside: Box<dyn Fn(f64) -> u32> = match by {
            GroupBy::Years => {
                let y0 = serial_to_ymd(first_day).0;
                let y1 = serial_to_ymd(last_day).0;
                items.extend((y0..=y1).map(|y| Value::text(y.to_string())));
                Box::new(move |n| (serial_to_ymd(n).0 - y0) as u32)
            }
            GroupBy::Days if interval > 1.0 => {
                let bins = ((last_day - first_day) / interval).floor() + 1.0;
                if bins > MAX_GROUPS {
                    return Err(PivotError::CannotGroup);
                }
                for k in 0..bins as u32 {
                    let lo = first_day + k as f64 * interval;
                    let hi = (lo + interval - 1.0).min(last_day);
                    items.push(Value::text(format!(
                        "{} - {}",
                        text::short_date(lo, locale),
                        text::short_date(hi, locale)
                    )));
                }
                Box::new(move |n| ((n.floor() - first_day) / interval).floor() as u32)
            }
            _ => {
                items.extend(text::date_group_items(by, locale).into_iter().map(Value::from));
                Box::new(move |n| text::date_group_index(by, n) as u32)
            }
        };
        items.push(Value::text(format!(">{}", text::short_date(last_day, locale))));
        let last = items.len() as u32 - 1;
        numbers
            .iter()
            .map(|&n| {
                if n < first_day {
                    0
                } else if n >= last_day + 1.0 {
                    last
                } else {
                    (inside(n) + 1).min(last - 1)
                }
            })
            .collect()
    };
    Ok(FieldGroup {
        base,
        kind: GroupKind::Range {
            by,
            start,
            end,
            interval,
            auto_start,
            auto_end,
        },
        items,
        map,
    })
}
