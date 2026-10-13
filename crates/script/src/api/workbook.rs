//! `Workbook`, `Application` and `NamedItem`.

use serde_json::Value as J;

use opensheet_engine::ast::Expr;
use opensheet_engine::eval::Calc;
use opensheet_engine::locale::Locale;
use opensheet_engine::names::{NameError, NameScope};
use opensheet_engine::protect::Action;
use opensheet_engine::recalc::CalcMode;
use opensheet_engine::value::Value;
use opensheet_engine::workbook::{DefinedName, Workbook};

use super::util::{absolute_address, range_arg};
use super::{ApiClass, Call, Fail, Res, Ret, class, enum_value, object, state_of, table};
use crate::host::{Host, MAX_SHEETS, to_script};

/// The identifier of a sheet as Excel writes them.
pub(super) fn sheet_guid(id: i64) -> String {
    format!("{{00000000-{:04X}-0000-0000-000000000000}}", id + 1)
}

/// The sheet a script names, by name or by identifier.
pub(super) fn find_sheet(h: &Host, key: &str) -> Option<usize> {
    h.wb.sheet_index(key)
        .or_else(|| (0..h.wb.sheets.len()).find(|i| sheet_guid(h.sheet_id(*i)).eq_ignore_ascii_case(key)))
}

/// Why a sheet cannot be called `name`, as Excel says it.
pub(super) fn sheet_name_problem(wb: &Workbook, name: &str, own: Option<usize>) -> Option<&'static str> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 31 || name.contains(['\\', '/', '?', '*', '[', ']', ':']) {
        return Some("The argument is invalid or missing or has an incorrect format.");
    }
    match wb.sheet_index(name) {
        Some(existing) if Some(existing) != own => Some("A resource with the same name or identifier already exists."),
        _ => None,
    }
}

pub(super) fn all_tables(h: &Host, sheet: Option<usize>) -> J {
    let tables = h
        .wb
        .sheets
        .iter()
        .enumerate()
        .filter(|(index, _)| sheet.is_none_or(|s| s == *index))
        .flat_map(|(_, sh)| sh.tables.iter().map(|t| table::table_object(t.id)))
        .collect::<Vec<J>>();
    J::from(tables)
}

// ---- names -------------------------------------------------------------------

fn scope_state(h: &Host, scope: NameScope) -> J {
    match scope {
        NameScope::Workbook => J::from(-1),
        NameScope::Sheet(sheet) => J::from(h.sheet_id(sheet)),
    }
}

fn named_item(h: &Host, scope: NameScope, name: &str) -> J {
    object("NamedItem", vec![scope_state(h, scope), J::from(name)])
}

pub(super) fn names_in(h: &Host, scope: NameScope) -> J {
    let names = match scope {
        NameScope::Workbook => &h.wb.names,
        NameScope::Sheet(sheet) => &h.wb.sheets[sheet].names,
    };
    J::from(names.values().map(|def| named_item(h, scope, &def.name)).collect::<Vec<J>>())
}

pub(super) fn name_in(h: &Host, call: &Call, scope: NameScope) -> Res<Ret> {
    let name = call.str(0)?;
    Ok(Ret::maybe(h.wb.name_in(scope, name).map(|def| named_item(h, scope, &def.name))))
}

fn name_fail(call: &Call, error: NameError) -> Fail {
    call.fail(match error {
        NameError::Invalid => "The name is not valid for a defined name.",
        NameError::Duplicate => "A resource with the same name or identifier already exists.",
        NameError::NotFound => "The requested resource doesn't exist.",
    })
}

/// The formula of a name given as a Range object, an address or a
/// formula, in stored form.
fn name_formula(h: &mut Host, call: &Call, index: usize, sheet: usize, local: bool) -> Res<Expr> {
    let text = match state_of(call.arg(index), "Range") {
        Some(_) => {
            let (on, rect) = range_arg(h, call, index, sheet)?;
            absolute_address(h, on, rect)
        }
        None => call.str(index)?.to_string(),
    };
    let parsed = if local {
        h.wb.parse_name_formula(&text, sheet, 0, 0)
    } else {
        h.english(|wb| wb.parse_name_formula(&text, sheet, 0, 0))
    };
    parsed.map_err(|_| call.invalid())
}

/// `addNamedItem` and `addNamedItemFormulaLocal`, for the workbook or a
/// sheet.
pub(super) fn add_name(h: &mut Host, call: &Call, scope: NameScope, local: bool) -> Res<Ret> {
    let name = call.str(0)?.trim();
    let sheet = match scope {
        NameScope::Sheet(sheet) => sheet,
        NameScope::Workbook => h.active_sheet(),
    };
    let expr = name_formula(h, call, 1, sheet, local)?;
    let comment = call.opt_str(2)?.unwrap_or_default();
    h.wb.add_name(scope, name, expr, comment).map_err(|e| name_fail(call, e))?;
    h.restructured();
    Ok(Ret::Object(named_item(h, scope, name)))
}

/// The name a NamedItem object stands for.
fn item(h: &Host, call: &Call) -> Res<(NameScope, DefinedName)> {
    let scope = match call.this_int(0) {
        -1 => NameScope::Workbook,
        id => NameScope::Sheet(h.sheet(call, id)?),
    };
    let name = call.this.get(1).and_then(J::as_str).unwrap_or_default();
    let def = h.wb.name_in(scope, name).ok_or_else(|| call.gone())?;
    Ok((scope, def.clone()))
}

/// The sheet formulas of a name are read from.
fn item_sheet(h: &Host, scope: NameScope) -> usize {
    match scope {
        NameScope::Sheet(sheet) => sheet,
        NameScope::Workbook => h.active_sheet(),
    }
}

/// The cells a name stands for, when it stands for one range.
fn item_range(h: &Host, scope: NameScope, def: &DefinedName) -> Option<(usize, opensheet_engine::Rect)> {
    let mut expr = def.expr_at(0, 0);
    while let Expr::Paren(inner) = expr {
        expr = *inner;
    }
    match expr {
        Expr::Ref(r) => {
            let sheet = match &r.sheet {
                Some(name) => h.wb.sheet_index(name)?,
                None => item_sheet(h, scope),
            };
            Some((sheet, r.rect()))
        }
        _ => None,
    }
}

fn item_value(h: &mut Host, scope: NameScope, def: &DefinedName) -> Value {
    h.calculate();
    Calc::eval_once(&h.wb, item_sheet(h, scope), 0, 0, &def.expr_at(0, 0))
}

pub(super) const WORKBOOK: ApiClass = class!("Workbook", "The workbook the script runs on: its worksheets, tables and names.", {
    getActiveWorksheet "(): Worksheet" "The worksheet in front." => |h, _| Ok(Ret::Object(h.worksheet(h.active_sheet())));
    getWorksheets "(): Worksheet[]" "Every worksheet, in the order of the tabs." => |h, _| {
        Ok(Ret::Object(J::from((0..h.wb.sheets.len()).map(|i| h.worksheet(i)).collect::<Vec<J>>())))
    };
    getWorksheet "(key: string): Worksheet | undefined" "The worksheet with this name or ID; undefined when there is none." => |h, c| {
        Ok(Ret::maybe(find_sheet(h, c.str(0)?).map(|i| h.worksheet(i))))
    };
    getFirstWorksheet "(visibleOnly?: boolean): Worksheet" "The first worksheet." => |h, _| Ok(Ret::Object(h.worksheet(0)));
    getLastWorksheet "(visibleOnly?: boolean): Worksheet" "The last worksheet." => |h, _| Ok(Ret::Object(h.worksheet(h.wb.sheets.len() - 1)));
    addWorksheet "(name?: string): Worksheet" "Adds a worksheet after the others. It is not activated." => |h, c| {
        h.permit(c, &Action::InsertSheet)?;
        let name = c.opt_str(0)?;
        if let Some(problem) = name.and_then(|n| sheet_name_problem(&h.wb, n, None)) {
            return Err(c.fail(problem));
        }
        if h.wb.sheets.len() >= MAX_SHEETS {
            return Err(c.fail(format!("A script cannot make a workbook of more than {MAX_SHEETS} worksheets.")));
        }
        let index = h.wb.add_sheet(h.wb.sheets.len());
        h.sheet_added(index);
        if let Some(name) = name {
            h.wb.rename_sheet(index, name);
        }
        Ok(Ret::Object(h.worksheet(index)))
    };
    getSelectedRange "(): Range" "The range selected in the window." => |h, _| Ok(Ret::Object(h.range(h.active_sheet(), h.selection)));
    getActiveCell "(): Range" "The active cell of the window." => |h, _| {
        Ok(Ret::Object(h.range(h.active_sheet(), opensheet_engine::Rect::cell(h.active_cell.0, h.active_cell.1))))
    };
    getTables "(): Table[]" "Every table of the workbook." => |h, _| Ok(Ret::Object(all_tables(h, None)));
    getTable "(key: string): Table | undefined" "The table with this name or ID; undefined when there is none." => |h, c| table::find(h, c, None);
    addTable "(address: Range | string, hasHeaders: boolean): Table" "Makes a table of a range. An address must name its sheet (Sheet1!A1:D5)." => |h, c| {
        let active = h.active_sheet();
        let (sheet, rect) = range_arg(h, c, 0, active)?;
        table::add(h, c, sheet, rect)
    };
    getNames "(): NamedItem[]" "The defined names whose scope is the workbook." => |h, _| Ok(Ret::Object(names_in(h, NameScope::Workbook)));
    getNamedItem "(name: string): NamedItem | undefined" "The defined name of the workbook spelled this way; undefined when there is none." => |h, c| name_in(h, c, NameScope::Workbook);
    addNamedItem "(name: string, reference: Range | string, comment?: string): NamedItem" "Defines a name for a range, a formula or a constant (formulas in English)." => |h, c| add_name(h, c, NameScope::Workbook, false);
    addNamedItemFormulaLocal "(name: string, formula: string, comment?: string): NamedItem" "Defines a name from a formula written in the language of the workbook." => |h, c| add_name(h, c, NameScope::Workbook, true);
    getName "(): string" "The name of the workbook's file." => |h, _| Ok(h.workbook_name.clone().into());
    getApplication "(): Application" "The application: calculation." => |_, _| Ok(Ret::object("Application", vec![]));
});

pub(super) const APPLICATION: ApiClass = class!("Application", "The application the workbook is open in: calculation.", {
    calculate "(calculationType?: ExcelScript.CalculationType): void" "Calculates the formulas of the workbook now (everything with Full or FullRebuild)." => |h, c| {
        if enum_value(c, 0, "CalculationType")?.is_some_and(|kind| kind != "Recalculate") {
            h.restructured();
        }
        h.calculate_now();
        Ok(Ret::Nothing)
    };
    getCalculationMode "(): ExcelScript.CalculationMode" "When formulas are calculated: Automatic, AutomaticExceptTables or Manual." => |h, _| {
        Ok(match h.wb.calc_mode() {
            CalcMode::Automatic => "Automatic",
            CalcMode::AutomaticExceptTables => "AutomaticExceptTables",
            CalcMode::Manual => "Manual",
        }.into())
    };
    setCalculationMode "(calculationMode: ExcelScript.CalculationMode): void" "Changes when formulas are calculated." => |h, c| {
        let mode = match enum_value(c, 0, "CalculationMode")?.ok_or_else(|| c.invalid())? {
            "Manual" => CalcMode::Manual,
            "AutomaticExceptTables" => CalcMode::AutomaticExceptTables,
            _ => CalcMode::Automatic,
        };
        h.wb.set_calc_mode(mode);
        h.calculated = true;
        Ok(Ret::Nothing)
    };
});

pub(super) const NAMED_ITEM: ApiClass = class!("NamedItem", "A defined name: a name for a range, a formula or a constant.", {
    getName "(): string" "The name." => |h, c| Ok(item(h, c)?.1.name.into());
    getScope "(): ExcelScript.NamedItemScope" "Workbook, or Worksheet for a name of one sheet." => |h, c| {
        Ok(match item(h, c)?.0 {
            NameScope::Workbook => "Workbook",
            NameScope::Sheet(_) => "Worksheet",
        }.into())
    };
    getWorksheet "(): Worksheet | undefined" "The worksheet the name belongs to; undefined for a name of the workbook." => |h, c| {
        Ok(Ret::maybe(match item(h, c)?.0 {
            NameScope::Workbook => None,
            NameScope::Sheet(sheet) => Some(h.worksheet(sheet)),
        }))
    };
    getFormula "(): string" "What the name refers to, as a formula in English (=Sheet1!$A$1:$B$2)." => |h, c| {
        Ok(item(h, c)?.1.refers_to(Locale::en(), 0, 0).into())
    };
    setFormula "(formula: string): void" "Changes what the name refers to (a formula in English)." => |h, c| {
        let (scope, def) = item(h, c)?;
        let expr = name_formula(h, c, 0, item_sheet(h, scope), false)?;
        h.wb.edit_name(scope, &def.name, &def.name, expr, &def.comment).map_err(|e| name_fail(c, e))?;
        h.restructured();
        Ok(Ret::Nothing)
    };
    getComment "(): string" "The comment of the name." => |h, c| Ok(item(h, c)?.1.comment.into());
    setComment "(comment: string): void" "Changes the comment of the name." => |h, c| {
        let (scope, def) = item(h, c)?;
        let comment = c.str(0)?;
        h.wb.edit_name(scope, &def.name, &def.name, (*def.expr).clone(), comment).map_err(|e| name_fail(c, e))?;
        Ok(Ret::Nothing)
    };
    getRange "(): Range" "The range the name stands for. Fails when it stands for something else." => |h, c| {
        let (scope, def) = item(h, c)?;
        let (sheet, rect) = item_range(h, scope, &def).ok_or_else(|| c.fail("The name does not refer to a range."))?;
        Ok(Ret::Object(h.range(sheet, rect)))
    };
    getType "(): ExcelScript.NamedItemType" "What the formula of the name gives: Range, String, Integer, Double, Boolean, Error or Array." => |h, c| {
        let (scope, def) = item(h, c)?;
        if item_range(h, scope, &def).is_some() {
            return Ok("Range".into());
        }
        Ok(match item_value(h, scope, &def) {
            Value::Number(n) if n.fract() == 0.0 => "Integer",
            Value::Number(_) => "Double",
            Value::Bool(_) => "Boolean",
            Value::Error(_) => "Error",
            Value::Array(_) => "Array",
            Value::Text(_) | Value::Empty => "String",
        }.into())
    };
    getValue "(): string | number" "The value of the formula of the name; for a range, its address." => |h, c| {
        let (scope, def) = item(h, c)?;
        if let Some((sheet, rect)) = item_range(h, scope, &def) {
            return Ok(absolute_address(h, sheet, rect).into());
        }
        Ok(Ret::Value(to_script(&item_value(h, scope, &def))))
    };
    getVisible "(): boolean" "Whether the name shows in the Name Manager: always true in OpenSheet." => |h, c| {
        item(h, c)?;
        Ok(true.into())
    };
    delete "(): void" "Deletes the name. Formulas using it show #NAME?." => |h, c| {
        let (scope, def) = item(h, c)?;
        h.wb.delete_name(scope, &def.name);
        h.restructured();
        Ok(Ret::Nothing)
    };
});
