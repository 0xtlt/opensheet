//! Calculated fields and calculated items ("Fields, Items, & Sets").
//!
//! A calculated field is a field of the cache that has a formula instead
//! of a column: `Sales-Cost`. Its formula names other fields and applies
//! to their *sums* over the records of each cell of the report, not to the
//! records one by one, as in Excel. A calculated item is an item added to
//! a field, computed from the other items of that field: `East+West`.
//!
//! Both are kept in the cache (as in the file format), so every PivotTable
//! built on it has them. Formulas are stored in English, with names as the
//! user reads them; a name that is not a plain word goes between single
//! quotes. To parse one, the names are first swapped for placeholders the
//! formula parser takes for defined names.

use super::cache::{CacheField, CalcItem, item_key};
use super::{Aggregation, Area, PivotCache, PivotError, PivotTable, Subtotals};
use crate::ast::{BinaryOp, Expr, UnaryOp};
use crate::eval::Calc;
use crate::locale::Locale;
use crate::parser::parse_formula;
use crate::value::{ErrorKind, Value};
use crate::workbook::Workbook;
use std::sync::Arc;

/// A formula ready to be computed: its names are `Expr::Name` placeholders.
#[derive(Clone, Debug, PartialEq)]
pub struct CalcFormula {
    pub(crate) expr: Expr,
    /// The names it computes from, as indices into the list it was parsed
    /// with (fields of the cache, or items of the field).
    pub(crate) uses: Vec<usize>,
}

fn placeholder(index: usize) -> String {
    format!("__pf{index}__")
}

/// The index a placeholder stands for.
fn placeholder_index(name: &str) -> Option<usize> {
    let lower = name.to_lowercase();
    lower.strip_prefix("__pf")?.strip_suffix("__")?.parse().ok()
}

/// True for a name that reads as one in a formula without quotes.
fn is_plain(name: &str) -> bool {
    let mut chars = name.chars();
    let starts = chars
        .next()
        .is_some_and(|ch| ch.is_alphabetic() || ch == '_');
    let word = name.chars().all(|ch| ch.is_alphanumeric() || ch == '_');
    // "Q1" and "ABC12" would read as cells, "TRUE" as a value.
    let letters = name
        .chars()
        .take_while(|ch| ch.is_ascii_alphabetic())
        .count();
    let cell = letters > 0
        && letters <= 3
        && letters < name.len()
        && name[letters..].chars().all(|ch| ch.is_ascii_digit());
    let word_value = matches!(name.to_uppercase().as_str(), "TRUE" | "FALSE");
    starts && word && !cell && !word_value
}

/// A name as a formula writes it.
fn quoted(name: &str) -> String {
    if is_plain(name) {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

/// Swap the names of a formula for placeholders: quoted names, and words
/// that are one of `names` (compared without case) and no function.
fn to_placeholders(text: &str, names: &[String]) -> String {
    let find = |name: &str| {
        names
            .iter()
            .position(|known| known.to_lowercase() == name.to_lowercase())
    };
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == '"' {
            // Text is copied as it is, doubled quotes included.
            out.push(ch);
            i += 1;
            while i < chars.len() {
                out.push(chars[i]);
                i += 1;
                if chars[i - 1] == '"' {
                    if chars.get(i) == Some(&'"') {
                        out.push('"');
                        i += 1;
                    } else {
                        break;
                    }
                }
            }
        } else if ch == '\'' {
            let mut name = String::new();
            let mut j = i + 1;
            let mut closed = false;
            while j < chars.len() {
                if chars[j] == '\'' {
                    if chars.get(j + 1) == Some(&'\'') {
                        name.push('\'');
                        j += 2;
                        continue;
                    }
                    closed = true;
                    j += 1;
                    break;
                }
                name.push(chars[j]);
                j += 1;
            }
            match find(&name).filter(|_| closed) {
                Some(index) => out.push_str(&placeholder(index)),
                // Left as typed: the parser will refuse it.
                None => out.extend(&chars[i..j]),
            }
            i = j;
        } else if ch.is_alphabetic() || ch == '_' {
            let mut j = i;
            while j < chars.len()
                && (chars[j].is_alphanumeric() || chars[j] == '_' || chars[j] == '.')
            {
                j += 1;
            }
            let word: String = chars[i..j].iter().collect();
            let call = chars[j..].iter().find(|c| !c.is_whitespace()) == Some(&'(');
            match find(&word).filter(|_| !call) {
                Some(index) => out.push_str(&placeholder(index)),
                None => out.push_str(&word),
            }
            i = j;
        } else {
            out.push(ch);
            i += 1;
        }
    }
    out
}

/// The reverse: placeholders become the names again, quoted where needed.
fn from_placeholders(text: &str, names: &[String]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.to_lowercase().find("__pf") {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let end = tail[4..].find("__").map(|e| e + 6);
        match end.and_then(|end| placeholder_index(&tail[..end]).map(|index| (end, index))) {
            Some((end, index)) if index < names.len() => {
                out.push_str(&quoted(&names[index]));
                rest = &tail[end..];
            }
            _ => {
                out.push_str(&tail[..4]);
                rest = &tail[4..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The functions a PivotTable formula may not use: those whose result
/// varies, and those reading lists or PivotTables.
fn refused_function(name: &str) -> bool {
    matches!(
        name,
        "RAND"
            | "RANDBETWEEN"
            | "RANDARRAY"
            | "NOW"
            | "TODAY"
            | "GETPIVOTDATA"
            | "OFFSET"
            | "INDIRECT"
            | "CELL"
            | "INFO"
    ) || (name.len() > 1
        && name.starts_with('D')
        && matches!(
            &name[1..],
            "SUM"
                | "COUNT"
                | "COUNTA"
                | "AVERAGE"
                | "MAX"
                | "MIN"
                | "GET"
                | "PRODUCT"
                | "STDEV"
                | "STDEVP"
                | "VAR"
                | "VARP"
        ))
}

impl CalcFormula {
    /// Read a formula in the language of `locale`; `names` are what it
    /// may compute from.
    pub fn parse(text: &str, names: &[String], locale: &Locale) -> Result<CalcFormula, PivotError> {
        let text = text.trim();
        let text = text.strip_prefix('=').unwrap_or(text);
        if text.trim().is_empty() {
            return Err(PivotError::BadFormula);
        }
        let expr = parse_formula(&to_placeholders(text, names), locale)
            .map_err(|_| PivotError::BadFormula)?;
        let mut uses: Vec<usize> = Vec::new();
        let mut fault: Option<PivotError> = None;
        expr.walk(&mut |e| match e {
            Expr::Name(name) => {
                match placeholder_index(name).filter(|index| *index < names.len()) {
                    Some(index) if !uses.contains(&index) => uses.push(index),
                    Some(_) => {}
                    None => fault = fault.take().or(Some(PivotError::FormulaReference)),
                }
            }
            Expr::Func(name, _) if refused_function(name) => {
                fault = fault.take().or(Some(PivotError::FormulaFunction))
            }
            Expr::Ref(_)
            | Expr::Ref3D(_)
            | Expr::RefError(_)
            | Expr::SheetName(..)
            | Expr::Table(_)
            | Expr::Spill(_)
            | Expr::Implicit(_)
            | Expr::Intersect(..)
            | Expr::Union(_)
            | Expr::Array(_) => fault = fault.take().or(Some(PivotError::FormulaReference)),
            _ => {}
        });
        match fault {
            Some(err) => Err(err),
            None => Ok(CalcFormula { expr, uses }),
        }
    }

    /// The formula as it is stored: in English, with its names.
    pub fn stored(&self, names: &[String]) -> String {
        from_placeholders(&self.expr.to_text(Locale::en()), names)
    }

    /// A stored formula in the language of `locale`, as the dialogs show
    /// it (without the equals sign).
    pub fn localized(stored: &str, names: &[String], locale: &Locale) -> String {
        match parse_formula(&to_placeholders(stored, names), Locale::en()) {
            Ok(expr) => from_placeholders(&expr.to_text(locale), names),
            Err(_) => stored.to_string(),
        }
    }

    /// Compute the formula. `value` gives what each name stands for;
    /// `scratch` is any workbook, for the functions.
    pub(crate) fn compute(
        &self,
        value: &dyn Fn(usize) -> Result<f64, ErrorKind>,
        scratch: &Workbook,
    ) -> Value {
        // Plain arithmetic, which is nearly every formula, without the
        // machinery of the evaluator.
        fn quick(
            e: &Expr,
            value: &dyn Fn(usize) -> Result<f64, ErrorKind>,
        ) -> Option<Result<f64, ErrorKind>> {
            Some(match e {
                Expr::Number(n) => Ok(*n),
                Expr::Name(name) => value(placeholder_index(name)?),
                Expr::Paren(inner) => quick(inner, value)?,
                Expr::Percent(inner) => quick(inner, value)?.map(|n| n / 100.0),
                Expr::Unary(op, inner) => {
                    quick(inner, value)?.map(|n| if *op == UnaryOp::Neg { -n } else { n })
                }
                Expr::Binary(op, a, b) => {
                    let (a, b) = (quick(a, value)?, quick(b, value)?);
                    match (a, b) {
                        (Err(e), _) | (_, Err(e)) => Err(e),
                        (Ok(a), Ok(b)) => match op {
                            BinaryOp::Add => Ok(a + b),
                            BinaryOp::Sub => Ok(a - b),
                            BinaryOp::Mul => Ok(a * b),
                            BinaryOp::Div if b == 0.0 => Err(ErrorKind::Div0),
                            BinaryOp::Div => Ok(a / b),
                            _ => return None,
                        },
                    }
                }
                _ => return None,
            })
        }
        if let Some(result) = quick(&self.expr, value) {
            return match result {
                Ok(n) if n.is_finite() => Value::Number(n),
                Ok(_) => Value::Error(ErrorKind::Num),
                Err(e) => Value::Error(e),
            };
        }
        let filled = self.expr.map(&mut |e| match &e {
            Expr::Name(name) => match placeholder_index(name).map(value) {
                Some(Ok(n)) => Expr::Number(n),
                Some(Err(err)) => Expr::Error(err),
                None => e,
            },
            _ => e,
        });
        match Calc::eval_once(scratch, 0, 0, 0, &filled) {
            Value::Number(n) => Value::Number(n),
            Value::Bool(b) => Value::Number(b as u8 as f64),
            Value::Empty => Value::Number(0.0),
            Value::Error(e) => Value::Error(e),
            Value::Array(a) => match a.get(0, 0) {
                Value::Number(n) => Value::Number(*n),
                Value::Error(e) => Value::Error(*e),
                _ => Value::Error(ErrorKind::Value),
            },
            Value::Text(_) => Value::Error(ErrorKind::Value),
        }
    }
}

impl PivotCache {
    /// The names of the fields, which the formulas of calculated fields
    /// compute from.
    pub fn field_names(&self) -> Vec<String> {
        self.fields.iter().map(|f| f.name.clone()).collect()
    }

    /// The labels of the items of a field, which the formulas of its
    /// calculated items compute from.
    pub fn item_names(&self, field: usize) -> Vec<String> {
        self.fields.get(field).map_or(Vec::new(), |f| {
            f.shared
                .iter()
                .map(|v| match v {
                    Value::Text(s) => s.to_string(),
                    Value::Number(_) => {
                        crate::format::format_value(v, "General", Locale::en()).text
                    }
                    Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
                    _ => String::new(),
                })
                .collect()
        })
    }

    /// The index of a calculated item among the items of its field.
    pub fn calc_item_index(&self, item: &CalcItem) -> Option<u32> {
        let key = item_key(&Value::text(&item.name));
        self.fields
            .get(item.field)?
            .shared
            .iter()
            .position(|v| item_key(v) == key)
            .map(|x| x as u32)
    }

    /// True when the item `x` of `field` is a calculated one.
    pub fn is_calc_item(&self, field: usize, x: u32) -> bool {
        self.calc_items
            .iter()
            .any(|item| item.field == field && self.calc_item_index(item) == Some(x))
    }

    /// The fields a calculated field computes from, directly or through
    /// other calculated fields; `None` when it computes from itself.
    fn field_closure(&self, field: usize) -> Option<Vec<usize>> {
        let names = self.field_names();
        let mut seen = vec![field];
        let mut todo = vec![field];
        let mut out = Vec::new();
        while let Some(f) = todo.pop() {
            let Some(formula) = self.fields.get(f).and_then(|cf| cf.formula.as_ref()) else {
                out.push(f);
                continue;
            };
            for used in CalcFormula::parse(formula, &names, Locale::en()).ok()?.uses {
                if used == field {
                    return None;
                }
                if !seen.contains(&used) {
                    seen.push(used);
                    todo.push(used);
                }
            }
        }
        Some(out)
    }

    /// Give a cache freshly read from the source the calculated fields and
    /// items of the one it replaces. An item whose name the new data now
    /// has as a real item is dropped, as Excel does.
    pub(crate) fn carry_calculated(&mut self, old: &PivotCache) {
        for of in old.fields.iter().filter(|f| f.formula.is_some()) {
            if self.field_index(&of.name).is_none() {
                self.fields.push(of.clone());
            }
        }
        for item in &old.calc_items {
            let Some(field) = old
                .fields
                .get(item.field)
                .and_then(|f| self.field_index(&f.name))
            else {
                continue;
            };
            let key = item_key(&Value::text(&item.name));
            if self.fields[field].shared.iter().any(|v| item_key(v) == key) {
                continue;
            }
            self.fields[field].shared.push(Value::text(&item.name));
            self.calc_items.push(CalcItem {
                field,
                ..item.clone()
            });
        }
    }
}

/// The texts of the sheet "List Formulas" writes, in the language of the
/// interface.
#[derive(Clone, Debug, PartialEq)]
pub struct ListLabels {
    pub calculated_field: String,
    pub calculated_item: String,
    pub solve_order: String,
    pub field: String,
    pub item: String,
    pub formula: String,
    /// The lines of the note under the lists.
    pub note: Vec<String>,
}

impl PivotTable {
    /// The calculated fields of the cache: (index, name, formula in the
    /// language of `locale`).
    pub fn calculated_fields(&self, locale: &Locale) -> Vec<(usize, String, String)> {
        let names = self.cache.field_names();
        self.cache
            .fields
            .iter()
            .enumerate()
            .filter_map(|(k, f)| {
                let formula = f.formula.as_ref()?;
                Some((
                    k,
                    f.name.clone(),
                    CalcFormula::localized(formula, &names, locale),
                ))
            })
            .collect()
    }

    /// The calculated items of a field: (item, name, formula).
    pub fn calculated_items(&self, field: usize, locale: &Locale) -> Vec<(u32, String, String)> {
        let names = self.cache.item_names(field);
        self.cache
            .calc_items
            .iter()
            .filter(|item| item.field == field)
            .filter_map(|item| {
                let x = self.cache.calc_item_index(item)?;
                Some((
                    x,
                    item.name.clone(),
                    CalcFormula::localized(&item.formula, &names, locale),
                ))
            })
            .collect()
    }

    /// The calculated items in solve order, as the dialog lists them:
    /// `Total = East +West`.
    pub fn solve_order(&self, locale: &Locale) -> Vec<String> {
        self.cache
            .calc_items
            .iter()
            .map(|item| {
                let names = self.cache.item_names(item.field);
                format!(
                    "{} = {}",
                    quoted(&item.name),
                    CalcFormula::localized(&item.formula, &names, locale)
                )
            })
            .collect()
    }

    /// Why the field cannot take a calculated item, if it cannot.
    pub fn calculated_item_refusal(&self, field: usize) -> Option<PivotError> {
        let cf = self.cache.fields.get(field)?;
        if cf.group.is_some() || !cf.is_database() || !self.cache.group_children(field).is_empty() {
            return Some(PivotError::ItemsInGroupedField);
        }
        if self.page_fields.iter().any(|p| p.field == field) {
            return Some(PivotError::ItemsInPageField);
        }
        let averaged = |f: Aggregation| {
            matches!(
                f,
                Aggregation::Average
                    | Aggregation::StdDev
                    | Aggregation::StdDevp
                    | Aggregation::Var
                    | Aggregation::Varp
            )
        };
        if self.data_fields.iter().any(|d| averaged(d.function)) {
            return Some(PivotError::ItemsWithAverages);
        }
        if self
            .fields
            .iter()
            .any(|f| matches!(f.subtotals, Subtotals::Custom(_)))
        {
            return Some(PivotError::ItemsWithCustomSubtotals);
        }
        let repeated = self
            .data_fields
            .iter()
            .enumerate()
            .any(|(k, d)| self.data_fields[..k].iter().any(|e| e.field == d.field));
        repeated.then_some(PivotError::ItemsWithRepeatedField)
    }
}

impl Workbook {
    /// Put `cache` in the place of the cache of PivotTable `id`, in every
    /// PivotTable built on it; `adjust` then changes the definition of
    /// `id` itself.
    fn pivot_swap_cache(
        &mut self,
        id: u32,
        cache: PivotCache,
        overwrite: bool,
        adjust: impl FnOnce(&mut PivotTable),
    ) -> Result<(), PivotError> {
        let (_, p) = self.pivot(id).ok_or(PivotError::NoSuchPivot)?;
        let old = p.cache.clone();
        let mut batch = self.pivot_rebound(&old, &Arc::new(cache));
        if let Some((_, _, p)) = batch.iter_mut().find(|(_, _, p)| p.id == id) {
            adjust(p);
        }
        self.pivot_install(batch, overwrite)
    }

    /// Add a calculated field named `name`, or give the one of that name
    /// another formula ("Add" and "Modify" of Insert Calculated Field).
    /// The formula is in the workbook's language. A new field joins the
    /// values of PivotTable `id`, as in Excel. Returns its index.
    pub fn pivot_set_calculated_field(
        &mut self,
        id: u32,
        name: &str,
        formula: &str,
        overwrite: bool,
    ) -> Result<usize, PivotError> {
        let locale = self.locale;
        let name = name.trim();
        let (_, p) = self.pivot(id).ok_or(PivotError::NoSuchPivot)?;
        let mut cache = (*p.cache).clone();
        let existing = cache.field_index(name);
        if name.is_empty() {
            return Err(PivotError::EmptyFieldName);
        }
        if existing.is_some_and(|f| cache.fields[f].formula.is_none()) {
            return Err(PivotError::FormulaNameTaken);
        }
        let field = existing.unwrap_or(cache.fields.len());
        if existing.is_none() {
            cache.fields.push(CacheField {
                name: name.to_string(),
                shared: Vec::new(),
                records: Vec::new(),
                num_fmt: Arc::from("General"),
                group: None,
                formula: Some(String::new()),
            });
        }
        let names = cache.field_names();
        let parsed = CalcFormula::parse(formula, &names, locale)?;
        cache.fields[field].formula = Some(parsed.stored(&names));
        if cache.field_closure(field).is_none() {
            return Err(PivotError::FormulaCircular);
        }
        self.pivot_swap_cache(id, cache, overwrite, |p| {
            if existing.is_none() {
                p.add_field(field, Area::Values, None, locale);
            }
        })?;
        Ok(field)
    }

    /// Delete a calculated field; the formulas naming it then read #NAME?.
    pub fn pivot_delete_calculated_field(
        &mut self,
        id: u32,
        field: usize,
        overwrite: bool,
    ) -> Result<(), PivotError> {
        let (_, p) = self.pivot(id).ok_or(PivotError::NoSuchPivot)?;
        let mut cache = (*p.cache).clone();
        if cache.fields.get(field).is_none_or(|f| f.formula.is_none()) {
            return Err(PivotError::NoSuchField);
        }
        cache.fields.remove(field);
        for item in &mut cache.calc_items {
            if item.field > field {
                item.field -= 1;
            }
        }
        for f in &mut cache.fields {
            if let Some(group) = &mut f.group
                && let Some(base) = &mut group.base
                && *base > field
            {
                *base -= 1;
            }
        }
        self.pivot_swap_cache(id, cache, overwrite, |_| {})
    }

    /// Add a calculated item named `name` to a field, or give the one of
    /// that name another formula. Returns the item.
    pub fn pivot_set_calculated_item(
        &mut self,
        id: u32,
        field: usize,
        name: &str,
        formula: &str,
        overwrite: bool,
    ) -> Result<u32, PivotError> {
        let locale = self.locale;
        let name = name.trim();
        let (_, p) = self.pivot(id).ok_or(PivotError::NoSuchPivot)?;
        if field >= p.cache.fields.len() {
            return Err(PivotError::NoSuchField);
        }
        if let Some(refusal) = p.calculated_item_refusal(field) {
            return Err(refusal);
        }
        if name.is_empty() {
            return Err(PivotError::EmptyFieldName);
        }
        let mut cache = (*p.cache).clone();
        let key = item_key(&Value::text(name));
        let at = cache
            .calc_items
            .iter()
            .position(|item| item.field == field && item_key(&Value::text(&item.name)) == key);
        if at.is_none()
            && cache.fields[field]
                .shared
                .iter()
                .any(|v| item_key(v) == key)
        {
            return Err(PivotError::FormulaNameTaken);
        }
        if at.is_none() {
            cache.fields[field].shared.push(Value::text(name));
        }
        let names = cache.item_names(field);
        let own = cache.fields[field]
            .shared
            .iter()
            .position(|v| item_key(v) == key)
            .unwrap_or(0);
        let parsed = CalcFormula::parse(formula, &names, locale)?;
        if parsed.uses.contains(&own) {
            return Err(PivotError::FormulaCircular);
        }
        let item = CalcItem {
            field,
            name: name.to_string(),
            formula: parsed.stored(&names),
        };
        match at {
            Some(at) => cache.calc_items[at] = item,
            None => cache.calc_items.push(item),
        }
        self.pivot_swap_cache(id, cache, overwrite, |_| {})?;
        Ok(own as u32)
    }

    /// Delete the calculated item `x` of a field.
    pub fn pivot_delete_calculated_item(
        &mut self,
        id: u32,
        field: usize,
        x: u32,
        overwrite: bool,
    ) -> Result<(), PivotError> {
        let (_, p) = self.pivot(id).ok_or(PivotError::NoSuchPivot)?;
        let mut cache = (*p.cache).clone();
        let at = cache
            .calc_items
            .iter()
            .position(|item| item.field == field && cache.calc_item_index(item) == Some(x))
            .ok_or(PivotError::NoSuchField)?;
        cache.calc_items.remove(at);
        let cf = &mut cache.fields[field];
        cf.shared.remove(x as usize);
        // No record has the item, but those listed after it move up.
        for record in &mut cf.records {
            if *record > x {
                *record -= 1;
            }
        }
        self.pivot_swap_cache(id, cache, overwrite, |_| {})
    }

    /// Put the calculated items in another solve order: `order` lists
    /// their present positions in the order wanted.
    pub fn pivot_set_solve_order(
        &mut self,
        id: u32,
        order: &[usize],
        overwrite: bool,
    ) -> Result<(), PivotError> {
        let (_, p) = self.pivot(id).ok_or(PivotError::NoSuchPivot)?;
        let mut cache = (*p.cache).clone();
        let mut sorted = order.to_vec();
        sorted.sort_unstable();
        if sorted != (0..cache.calc_items.len()).collect::<Vec<_>>() {
            return Err(PivotError::NoSuchField);
        }
        cache.calc_items = order.iter().map(|k| cache.calc_items[*k].clone()).collect();
        self.pivot_swap_cache(id, cache, overwrite, |_| {})
    }

    /// "List Formulas": write the calculated fields and items of a
    /// PivotTable on a new sheet put before its own; returns the index of
    /// that sheet.
    pub fn pivot_list_formulas(
        &mut self,
        id: u32,
        labels: &ListLabels,
    ) -> Result<usize, PivotError> {
        let locale = self.locale;
        let (si, p) = self.pivot(id).ok_or(PivotError::NoSuchPivot)?;
        let p = p.clone();
        let sheet = self.add_sheet(si);
        let mut row = 0u32;
        let put = |wb: &mut Workbook, row: u32, cells: &[Value]| {
            for (col, value) in cells.iter().enumerate() {
                wb.set_value(sheet, row, col as u32, value.clone());
            }
        };
        let text = |s: &str| Value::text(s);
        put(self, row, &[text(&labels.calculated_field)]);
        put(
            self,
            row + 1,
            &[
                text(&labels.solve_order),
                text(&labels.field),
                text(&labels.formula),
            ],
        );
        row += 2;
        for (k, (_, name, formula)) in p.calculated_fields(locale).into_iter().enumerate() {
            put(
                self,
                row,
                &[
                    Value::Number(k as f64 + 1.0),
                    text(&name),
                    text(&format!("={formula}")),
                ],
            );
            row += 1;
        }
        row += 1;
        put(self, row, &[text(&labels.calculated_item)]);
        put(
            self,
            row + 1,
            &[
                text(&labels.solve_order),
                text(&labels.item),
                text(&labels.formula),
            ],
        );
        row += 2;
        for (k, item) in p.cache.calc_items.iter().enumerate() {
            let names = p.cache.item_names(item.field);
            let formula = CalcFormula::localized(&item.formula, &names, locale);
            put(
                self,
                row,
                &[
                    Value::Number(k as f64 + 1.0),
                    text(&quoted(&item.name)),
                    text(&format!("={formula}")),
                ],
            );
            row += 1;
        }
        row += 2;
        for line in &labels.note {
            put(self, row, &[text(line)]);
            row += 1;
        }
        Ok(sheet)
    }
}
