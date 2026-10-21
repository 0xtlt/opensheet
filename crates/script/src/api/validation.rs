//! `DataValidation`: the rule of a range, its input message and its
//! error alert, as the Data Validation dialog sets them.

use serde_json::{Value as J, json};

use opensheet_engine::locale::Locale;
use opensheet_engine::protect::Action;
use opensheet_engine::refs::Rect;
use opensheet_engine::validation::{
    AlertStyle, Operator, SelectionValidation, Validation, ValidationKind,
};

use super::areas::{areas_object, rects_of_cells};
use super::util::{absolute_address, range_from, this_range};
use super::{ApiClass, Call, Res, Ret, class, state_of};
use crate::host::Host;

/// The kinds of rule, with the name of each in `DataValidationType` and
/// the key it has in a `DataValidationRule`.
const KINDS: [(ValidationKind, &str, &str); 7] = [
    (ValidationKind::Whole, "WholeNumber", "wholeNumber"),
    (ValidationKind::Decimal, "Decimal", "decimal"),
    (ValidationKind::List, "List", "list"),
    (ValidationKind::Date, "Date", "date"),
    (ValidationKind::Time, "Time", "time"),
    (ValidationKind::TextLength, "TextLength", "textLength"),
    (ValidationKind::Custom, "Custom", "custom"),
];

const OPERATORS: [(Operator, &str); 8] = [
    (Operator::Between, "Between"),
    (Operator::NotBetween, "NotBetween"),
    (Operator::Equal, "EqualTo"),
    (Operator::NotEqual, "NotEqualTo"),
    (Operator::GreaterThan, "GreaterThan"),
    (Operator::LessThan, "LessThan"),
    (Operator::GreaterOrEqual, "GreaterThanOrEqualTo"),
    (Operator::LessOrEqual, "LessThanOrEqualTo"),
];

/// What the cells of the range have as a rule: none, one (which may
/// leave some of them out) or several.
enum Found {
    None,
    /// The rule, with its formulas as they read from the first cell of
    /// the range; whether every cell of the range has it.
    One(Validation, bool),
    Several,
}

fn found(h: &Host, sheet: usize, rect: Rect) -> Found {
    let anchored = |rule: &Validation| Validation {
        formula1: rule.formula1_at(rect.r0, rect.c0).map(Into::into),
        formula2: rule.formula2_at(rect.r0, rect.c0).map(Into::into),
        ranges: vec![Rect::cell(rect.r0, rect.c0)],
        ..rule.clone()
    };
    match h.wb.selection_validation(sheet, &[rect]) {
        SelectionValidation::None => Found::None,
        SelectionValidation::Uniform(rule) => Found::One(anchored(rule), true),
        SelectionValidation::Partial(rule) => Found::One(anchored(rule), false),
        SelectionValidation::Mixed => Found::Several,
    }
}

/// The rule the settings of the range are read from: the default when it
/// has none.
fn rule(h: &Host, call: &Call) -> Res<(usize, Rect, Validation)> {
    let (sheet, rect) = this_range(h, call)?;
    match found(h, sheet, rect) {
        Found::None => Ok((sheet, rect, Validation::default())),
        Found::One(rule, _) => Ok((sheet, rect, rule)),
        Found::Several => Err(call.fail("The range has several data validation rules.")),
    }
}

/// Change the rule of the range, as OK in the dialog: every cell of the
/// range takes the rule.
fn edit(h: &mut Host, call: &Call, change: impl FnOnce(&mut Validation) -> Res<()>) -> Res<Ret> {
    let (sheet, rect, mut rule) = rule(h, call)?;
    change(&mut rule)?;
    h.permit(call, &Action::ChangeSheet { sheet })?;
    h.wb.set_validation(sheet, &[rect], (rect.r0, rect.c0), rule);
    Ok(Ret::Nothing)
}

/// A value, a formula or a list as scripts give them, as the text typed
/// in a box of the dialog: `5`, `"=A1"`, `"a,b,c"`, a Range.
fn typed(h: &Host, call: &Call, value: Option<&J>) -> Res<String> {
    match value {
        None | Some(J::Null) => Ok(String::new()),
        Some(J::Number(n)) => Ok(n.to_string()),
        Some(J::String(text)) => Ok(text.clone()),
        Some(other) => {
            let state = state_of(other, "Range").ok_or_else(|| call.invalid())?;
            let (sheet, rect) = range_from(h, call, state)?;
            Ok(format!("={}", absolute_address(h, sheet, rect)))
        }
    }
}

fn text_field(of: &J, key: &str) -> String {
    of.get(key)
        .and_then(J::as_str)
        .unwrap_or_default()
        .to_string()
}

pub(super) const DATA_VALIDATION: ApiClass = class!("DataValidation", "The data validation of a range: what may be entered in its cells, the message shown when one is selected and the alert shown for a wrong entry.", {
    getType "(): ExcelScript.DataValidationType" "The kind of rule: None, WholeNumber, Decimal, List, Date, Time, TextLength or Custom; Inconsistent when only some cells have the rule, MixedCriteria when the cells have different rules." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        Ok(match found(h, sheet, rect) {
            Found::None => "None",
            Found::Several => "MixedCriteria",
            Found::One(_, false) => "Inconsistent",
            Found::One(rule, true) => KINDS.iter().find(|(kind, _, _)| *kind == rule.kind).map_or("None", |(_, name, _)| name),
        }.into())
    };
    getRule "(): ExcelScript.DataValidationRule" "The rule: an object with one key for its kind (wholeNumber, decimal, list, date, time, textLength, custom), empty without a rule. Formulas are in English and read from the first cell of the range." => |h, c| {
        let (_, rect, rule) = rule(h, c)?;
        let Some((_, _, key)) = KINDS.iter().find(|(kind, _, _)| *kind == rule.kind) else {
            return Ok(Ret::Value(json!({})));
        };
        let first = rule.operand1_text(rect.r0, rect.c0, Locale::en());
        let second = rule.operand2_text(rect.r0, rect.c0, Locale::en());
        let inner = match rule.kind {
            ValidationKind::Custom => json!({ "formula": first }),
            ValidationKind::List => json!({ "inCellDropDown": rule.in_cell_dropdown, "source": first }),
            _ => {
                let operator = OPERATORS.iter().find(|(op, _)| *op == rule.operator).map_or("Between", |(_, name)| name);
                let mut inner = json!({ "formula1": first, "operator": operator });
                if rule.operator.has_two_operands() {
                    inner["formula2"] = second.into();
                }
                inner
            }
        };
        Ok(Ret::Value(json!({ *key: inner })))
    };
    setRule "(rule: ExcelScript.DataValidationRule): void" "Gives every cell of the range a rule: { wholeNumber | decimal | date | time | textLength: { formula1, formula2?, operator } }, { list: { inCellDropDown, source } } or { custom: { formula } }. Values, formulas in English (=A1) or, for a list, items separated by commas or a Range." => |h, c| {
        let given = c.arg(0);
        let (kind, inner) = KINDS
            .iter()
            .find_map(|(kind, _, key)| given.get(*key).filter(|inner| inner.is_object()).map(|inner| (*kind, inner)))
            .ok_or_else(|| c.invalid())?;
        let (first, second) = match kind {
            ValidationKind::Custom => (typed(h, c, inner.get("formula"))?, String::new()),
            ValidationKind::List => (typed(h, c, inner.get("source"))?, String::new()),
            _ => (typed(h, c, inner.get("formula1"))?, typed(h, c, inner.get("formula2"))?),
        };
        let operator = match inner.get("operator").and_then(J::as_str) {
            Some(name) => OPERATORS.iter().find(|(_, n)| *n == name).map(|(op, _)| *op).ok_or_else(|| c.invalid())?,
            None if kind.has_operator() => return Err(c.invalid()),
            None => Operator::Between,
        };
        edit(h, c, |rule| {
            rule.kind = kind;
            rule.operator = operator;
            if kind == ValidationKind::List {
                rule.in_cell_dropdown = inner.get("inCellDropDown").and_then(J::as_bool).unwrap_or(true);
            }
            // The checks of the dialog, with its messages.
            rule.set_operands(&first, &second, Locale::en()).map_err(|error| c.fail(error.message(Locale::en())))
        })
    };
    clear "(): void" "Removes the data validation of the cells of the range." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        if matches!(found(h, sheet, rect), Found::None) {
            return Ok(Ret::Nothing);
        }
        h.permit(c, &Action::ChangeSheet { sheet })?;
        h.wb.clear_validation(sheet, &[rect]);
        Ok(Ret::Nothing)
    };
    getIgnoreBlanks "(): boolean" "Whether empty cells are accepted whatever the rule." => |h, c| Ok(rule(h, c)?.2.allow_blank.into());
    setIgnoreBlanks "(ignoreBlanks: boolean): void" "Ignore blank: empty cells are accepted whatever the rule." => |h, c| {
        let ignore = c.bool(0)?;
        edit(h, c, |rule| {
            rule.allow_blank = ignore;
            Ok(())
        })
    };
    getErrorAlert "(): ExcelScript.DataValidationErrorAlert" "The alert shown for an entry that breaks the rule: { message, showAlert, style, title }." => |h, c| {
        let rule = rule(h, c)?.2;
        let style = match rule.error_style {
            AlertStyle::Stop => "Stop",
            AlertStyle::Warning => "Warning",
            AlertStyle::Information => "Information",
        };
        Ok(Ret::Value(json!({ "message": rule.error_message, "showAlert": rule.show_error, "style": style, "title": rule.error_title })))
    };
    setErrorAlert "(errorAlert: ExcelScript.DataValidationErrorAlert): void" "Sets the alert: its style (Stop refuses the entry, Warning and Information let it through), title and message, and whether it shows." => |h, c| {
        let given = c.arg(0);
        if !given.is_object() {
            return Err(c.invalid());
        }
        let style = match given.get("style").and_then(J::as_str) {
            Some("Warning") => AlertStyle::Warning,
            Some("Information") => AlertStyle::Information,
            Some("Stop") | None => AlertStyle::Stop,
            Some(_) => return Err(c.invalid()),
        };
        edit(h, c, |rule| {
            rule.error_style = style;
            rule.error_title = text_field(given, "title");
            rule.error_message = text_field(given, "message");
            rule.show_error = given.get("showAlert").and_then(J::as_bool).unwrap_or(true);
            Ok(())
        })
    };
    getPrompt "(): ExcelScript.DataValidationPrompt" "The message shown when a cell of the range is selected: { message, showPrompt, title }." => |h, c| {
        let rule = rule(h, c)?.2;
        Ok(Ret::Value(json!({ "message": rule.input_message, "showPrompt": rule.show_input, "title": rule.input_title })))
    };
    setPrompt "(prompt: ExcelScript.DataValidationPrompt): void" "Sets the input message: title, message, and whether it shows." => |h, c| {
        let given = c.arg(0);
        if !given.is_object() {
            return Err(c.invalid());
        }
        edit(h, c, |rule| {
            rule.input_title = text_field(given, "title");
            rule.input_message = text_field(given, "message");
            rule.show_input = given.get("showPrompt").and_then(J::as_bool).unwrap_or(true);
            Ok(())
        })
    };
    getValid "(): boolean" "Whether every cell of the range holds a value its rule accepts." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        h.calculate();
        Ok(h.wb.invalid_cells(sheet).cells.iter().all(|(row, col)| !rect.contains(*row, *col)).into())
    };
    getInvalidCells "(): RangeAreas | undefined" "The cells of the range whose value breaks their rule (what Circle Invalid Data circles); undefined when every cell is valid." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        h.calculate();
        let cells: Vec<(u32, u32)> = h.wb.invalid_cells(sheet).cells.into_iter().filter(|(row, col)| rect.contains(*row, *col)).collect();
        Ok(Ret::maybe((!cells.is_empty()).then(|| areas_object(h, sheet, &rects_of_cells(cells)))))
    };
});
