//! Conditional formats: `ConditionalFormat`, one class per kind of rule
//! and the format a rule gives its cells.
//!
//! A rule made by a script is the rule the ribbon makes: it takes the
//! highest priority, and a protected sheet refuses it unless it allows
//! formatting cells. Formulas are in English and written for the first
//! cell of the range, as in the rule dialog.

use std::sync::Arc;

use serde_json::{Value as J, json};

use opensheet_engine::ast::Expr;
use opensheet_engine::condfmt::{
    BarAxis, BarDirection, CellOp, Cfvo, ColorScale, ColorStop, CondFormat, DataBar, IconRule, IconSet, IconThreshold, Rule,
    ScalePreset, TextOp, TimePeriod, operand_text, parse_operand,
};
use opensheet_engine::condfmt::BarColor;
use opensheet_engine::locale::Locale;
use opensheet_engine::protect::Action;
use opensheet_engine::refs::Rect;
use opensheet_engine::style::{Border, BorderStyle, Color, StylePatch, Underline};

use super::areas::{areas_arg, areas_object};
use super::util::{color_arg, color_text, parse_color, this_range};
use super::{ApiClass, Call, Res, Ret, class, enum_value, object};
use crate::host::Host;

// ---- the rule behind an object ---------------------------------------------------

fn handle_object(class: &str, handle: i64) -> J {
    object(class, vec![handle.into()])
}

/// The sheet, the place and the rule an object of these classes stands
/// for.
fn rule<'h>(h: &'h Host, call: &Call) -> Res<(usize, usize, &'h CondFormat)> {
    let (sheet, index) = h.cond_format(call, call.this_int(0))?;
    Ok((sheet, index, &h.wb.sheets[sheet].cond_formats[index]))
}

/// Change the rule a call is made on, as one edit of the Rules Manager.
fn edit(h: &mut Host, call: &Call, change: impl FnOnce(&mut CondFormat) -> Res<()>) -> Res<Ret> {
    let (sheet, index, old) = rule(h, call)?;
    let mut new = old.clone();
    change(&mut new)?;
    h.permit(call, &Action::FormatCells { sheet })?;
    h.wb.update_cond_format(sheet, index, |cf| *cf = new);
    Ok(Ret::Nothing)
}

/// The kind of a rule as `ConditionalFormatType` names it.
fn kind(rule: &Rule) -> &'static str {
    match rule {
        Rule::CellIs { .. } => "CellValue",
        Rule::Formula(_) => "Custom",
        Rule::TopBottom { .. } => "TopBottom",
        Rule::Text { .. } => "ContainsText",
        Rule::ColorScale(_) => "ColorScale",
        Rule::DataBar(_) => "DataBar",
        Rule::IconSet(_) => "IconSet",
        Rule::Average { .. }
        | Rule::DuplicateValues
        | Rule::UniqueValues
        | Rule::Blanks
        | Rule::NoBlanks
        | Rule::Errors
        | Rule::NoErrors
        | Rule::DateOccurring(_) => "PresetCriteria",
    }
}

/// The rule a kind starts as, before the script sets it: what the rule
/// dialog proposes for that kind.
fn starting(kind: &str) -> Rule {
    match kind {
        "CellValue" => Rule::cell_is(CellOp::Between, Expr::Number(0.0), Some(Expr::Number(0.0))),
        "Custom" => Rule::Formula(Expr::Bool(false)),
        "TopBottom" => Rule::top_items(10),
        "ContainsText" => Rule::Text {
            op: TextOp::Contains,
            text: Expr::Text(Arc::from("")),
        },
        "ColorScale" => Rule::ColorScale(ScalePreset::GreenYellowRed.scale()),
        "DataBar" => Rule::DataBar(DataBar::preset(BarColor::Blue, true)),
        "IconSet" => Rule::IconSet(IconRule::preset(IconSet::ThreeTrafficLights1)),
        _ => Rule::DuplicateValues,
    }
}

/// `Range.addConditionalFormat`.
pub(super) fn add(h: &mut Host, call: &Call) -> Res<Ret> {
    let (sheet, rect) = this_range(h, call)?;
    let kind = enum_value(call, 0, "ConditionalFormatType")?.ok_or_else(|| call.invalid())?;
    h.permit(call, &Action::FormatCells { sheet })?;
    h.wb.add_cond_format(sheet, CondFormat::new(vec![rect], starting(kind)));
    // The new rule is first: the others step down.
    h.cond_formats_moved(sheet, |index| Some(index + 1));
    Ok(Ret::Object(handle_object("ConditionalFormat", h.cond_handle(sheet, 0))))
}

/// `Range.getConditionalFormats`: the rules that reach the range, in
/// priority order.
pub(super) fn list(h: &mut Host, call: &Call) -> Res<Ret> {
    let (sheet, rect) = this_range(h, call)?;
    let found = h.wb.cond_formats_in(sheet, &[rect]);
    let objects: Vec<J> = found.into_iter().map(|index| handle_object("ConditionalFormat", h.cond_handle(sheet, index))).collect();
    Ok(Ret::Object(J::from(objects)))
}

/// `Range.getConditionalFormat(id)`.
pub(super) fn by_id(h: &mut Host, call: &Call) -> Res<Ret> {
    let (sheet, _) = this_range(h, call)?;
    let handle = call.str(0)?.parse::<i64>().map_err(|_| call.invalid())?;
    let found = h.cond_format(call, handle).ok().filter(|(of, _)| *of == sheet);
    Ok(Ret::maybe(found.map(|_| handle_object("ConditionalFormat", handle))))
}

/// `Range.clearAllConditionalFormats`: the cells of the range leave every
/// rule, as Clear Rules from Selected Cells.
pub(super) fn clear(h: &mut Host, call: &Call) -> Res<Ret> {
    let (sheet, rect) = this_range(h, call)?;
    if h.wb.cond_formats_in(sheet, &[rect]).is_empty() {
        return Ok(Ret::Nothing);
    }
    h.permit(call, &Action::FormatCells { sheet })?;
    // A rule goes when all its cells are in the range.
    let inside = |r: &Rect| rect.intersection(r) == Some(*r);
    let gone: Vec<bool> = h.wb.sheets[sheet].cond_formats.iter().map(|cf| cf.ranges.iter().all(inside)).collect();
    h.wb.clear_cond_formats(sheet, &[rect]);
    h.cond_formats_moved(sheet, |index| match gone.get(index) {
        Some(false) => Some(index - gone[..index].iter().filter(|g| **g).count()),
        _ => None,
    });
    Ok(Ret::Nothing)
}

/// The object of a kind of rule, when the rule is of that kind.
fn part(h: &Host, call: &Call, class: &str, kinds: &str) -> Res<Ret> {
    let (_, _, cf) = rule(h, call)?;
    Ok(Ret::maybe((kind(&cf.rule) == kinds).then(|| handle_object(class, call.this_int(0)))))
}

// ---- formulas and thresholds -------------------------------------------------------

/// A value or a formula as scripts write them: `5`, `"5"`, `"=A1*2"`.
fn operand(call: &Call, value: &J, anchor: (u32, u32)) -> Res<Expr> {
    match value {
        J::Number(n) => n.as_f64().map(Expr::Number).ok_or_else(|| call.invalid()),
        J::String(text) => parse_operand(text, Locale::en(), anchor, anchor).map_err(|_| call.invalid()),
        _ => Err(call.invalid()),
    }
}

fn operand_out(expr: &Expr, anchor: (u32, u32)) -> String {
    operand_text(expr, Locale::en(), anchor, anchor, false)
}

/// A threshold of a colour scale, a data bar or an icon set, from its
/// kind (`LowestValue`, `Number`, `Percent`...) and its formula.
fn threshold(call: &Call, kind: &str, formula: Option<&J>, anchor: (u32, u32)) -> Res<Cfvo> {
    let value = || operand(call, formula.ok_or_else(|| call.invalid())?, anchor);
    Ok(match kind {
        "LowestValue" => Cfvo::Min,
        "HighestValue" => Cfvo::Max,
        "Automatic" => Cfvo::Automatic,
        "Number" => Cfvo::Number(value()?),
        "Percent" => Cfvo::Percent(value()?),
        "Percentile" => Cfvo::Percentile(value()?),
        "Formula" => Cfvo::Formula(value()?),
        _ => return Err(call.invalid()),
    })
}

/// The kind and the formula of a threshold, as scripts read them.
fn threshold_out(at: &Cfvo, anchor: (u32, u32)) -> (&'static str, Option<String>) {
    match at {
        Cfvo::Min => ("LowestValue", None),
        Cfvo::Max => ("HighestValue", None),
        Cfvo::Automatic => ("Automatic", None),
        Cfvo::Number(e) => ("Number", Some(operand_out(e, anchor))),
        Cfvo::Percent(e) => ("Percent", Some(operand_out(e, anchor))),
        Cfvo::Percentile(e) => ("Percentile", Some(operand_out(e, anchor))),
        Cfvo::Formula(e) => ("Formula", Some(operand_out(e, anchor))),
    }
}

fn field<'a>(call: &Call, of: &'a J, key: &str) -> Res<&'a str> {
    of.get(key).and_then(J::as_str).ok_or_else(|| call.invalid())
}

fn names_eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn icon_set(call: &Call, name: &str) -> Res<IconSet> {
    IconSet::ALL.into_iter().find(|set| names_eq(&format!("{set:?}"), name)).ok_or_else(|| call.invalid())
}

const CELL_OPS: [(&str, CellOp); 8] = [
    ("Between", CellOp::Between),
    ("NotBetween", CellOp::NotBetween),
    ("EqualTo", CellOp::Equal),
    ("NotEqualTo", CellOp::NotEqual),
    ("GreaterThan", CellOp::Greater),
    ("LessThan", CellOp::Less),
    ("GreaterThanOrEqual", CellOp::GreaterOrEqual),
    ("LessThanOrEqual", CellOp::LessOrEqual),
];

const TEXT_OPS: [(&str, TextOp); 4] = [
    ("Contains", TextOp::Contains),
    ("NotContains", TextOp::NotContains),
    ("BeginsWith", TextOp::BeginsWith),
    ("EndsWith", TextOp::EndsWith),
];

/// The preset criteria and the rules they are.
fn presets() -> Vec<(&'static str, Rule)> {
    let average = |above: bool, or_equal: bool, std_dev: u32| Rule::Average { above, or_equal, std_dev };
    vec![
        ("Blanks", Rule::Blanks),
        ("NonBlanks", Rule::NoBlanks),
        ("Errors", Rule::Errors),
        ("NonErrors", Rule::NoErrors),
        ("Yesterday", Rule::DateOccurring(TimePeriod::Yesterday)),
        ("Today", Rule::DateOccurring(TimePeriod::Today)),
        ("Tomorrow", Rule::DateOccurring(TimePeriod::Tomorrow)),
        ("LastSevenDays", Rule::DateOccurring(TimePeriod::Last7Days)),
        ("LastWeek", Rule::DateOccurring(TimePeriod::LastWeek)),
        ("ThisWeek", Rule::DateOccurring(TimePeriod::ThisWeek)),
        ("NextWeek", Rule::DateOccurring(TimePeriod::NextWeek)),
        ("LastMonth", Rule::DateOccurring(TimePeriod::LastMonth)),
        ("ThisMonth", Rule::DateOccurring(TimePeriod::ThisMonth)),
        ("NextMonth", Rule::DateOccurring(TimePeriod::NextMonth)),
        ("AboveAverage", average(true, false, 0)),
        ("BelowAverage", average(false, false, 0)),
        ("EqualOrAboveAverage", average(true, true, 0)),
        ("EqualOrBelowAverage", average(false, true, 0)),
        ("OneStdDevAboveAverage", average(true, false, 1)),
        ("OneStdDevBelowAverage", average(false, false, 1)),
        ("TwoStdDevAboveAverage", average(true, false, 2)),
        ("TwoStdDevBelowAverage", average(false, false, 2)),
        ("ThreeStdDevAboveAverage", average(true, false, 3)),
        ("ThreeStdDevBelowAverage", average(false, false, 3)),
        ("UniqueValues", Rule::UniqueValues),
        ("DuplicateValues", Rule::DuplicateValues),
    ]
}

// ---- parts of a rule -------------------------------------------------------------

fn scale<'h>(h: &'h Host, call: &Call) -> Res<(&'h ColorScale, (u32, u32))> {
    match rule(h, call)? {
        (_, _, cf @ CondFormat { rule: Rule::ColorScale(scale), .. }) => Ok((scale, cf.anchor())),
        _ => Err(call.gone()),
    }
}

fn bar<'h>(h: &'h Host, call: &Call) -> Res<(&'h DataBar, (u32, u32))> {
    match rule(h, call)? {
        (_, _, cf @ CondFormat { rule: Rule::DataBar(bar), .. }) => Ok((bar, cf.anchor())),
        _ => Err(call.gone()),
    }
}

fn edit_bar(h: &mut Host, call: &Call, change: impl FnOnce(&mut DataBar, (u32, u32)) -> Res<()>) -> Res<Ret> {
    edit(h, call, |cf| {
        let anchor = cf.anchor();
        match &mut cf.rule {
            Rule::DataBar(bar) => change(bar, anchor),
            _ => Err(call.gone()),
        }
    })
}

fn icons<'h>(h: &'h Host, call: &Call) -> Res<(&'h IconRule, (u32, u32))> {
    match rule(h, call)? {
        (_, _, cf @ CondFormat { rule: Rule::IconSet(icons), .. }) => Ok((icons, cf.anchor())),
        _ => Err(call.gone()),
    }
}

fn edit_icons(h: &mut Host, call: &Call, change: impl FnOnce(&mut IconRule, (u32, u32)) -> Res<()>) -> Res<Ret> {
    edit(h, call, |cf| {
        let anchor = cf.anchor();
        match &mut cf.rule {
            Rule::IconSet(icons) => change(icons, anchor),
            _ => Err(call.gone()),
        }
    })
}

/// The rule of a data bar end: `{ type, formula? }`.
fn bar_end(call: &Call, anchor: (u32, u32)) -> Res<Cfvo> {
    let given = call.arg(0);
    threshold(call, field(call, given, "type")?, given.get("formula"), anchor)
}

fn bar_end_out(at: &Cfvo, anchor: (u32, u32)) -> Ret {
    let (kind, formula) = threshold_out(at, anchor);
    Ret::Value(match formula {
        Some(formula) => json!({ "type": kind, "formula": formula }),
        None => json!({ "type": kind }),
    })
}

fn optional_color(call: &Call, index: usize) -> Res<Color> {
    color_arg(call, index)?.ok_or_else(|| call.invalid())
}

// ---- the format of the cells that match ----------------------------------------------

fn patch<'h>(h: &'h Host, call: &Call) -> Res<&'h StylePatch> {
    Ok(&rule(h, call)?.2.format)
}

fn edit_patch(h: &mut Host, call: &Call, change: impl FnOnce(&mut StylePatch) -> Res<()>) -> Res<Ret> {
    edit(h, call, |cf| change(&mut cf.format))
}

const BORDER_SIDES: [&str; 4] = ["EdgeTop", "EdgeBottom", "EdgeLeft", "EdgeRight"];

fn border_object(call: &Call, side: usize) -> J {
    object("ConditionalRangeBorder", vec![call.this_int(0).into(), side.into()])
}

fn border_slot(patch: &mut StylePatch, side: i64) -> &mut Option<Border> {
    match side {
        0 => &mut patch.border_top,
        1 => &mut patch.border_bottom,
        2 => &mut patch.border_left,
        _ => &mut patch.border_right,
    }
}

fn border_of(h: &Host, call: &Call) -> Res<Option<Border>> {
    let mut patch = patch(h, call)?.clone();
    Ok(*border_slot(&mut patch, call.this_int(1)))
}

/// The format a kind of rule gives, shared by the five kinds that have
/// one.
fn format_object(h: &Host, call: &Call) -> Res<Ret> {
    rule(h, call)?;
    Ok(Ret::Object(handle_object("ConditionalRangeFormat", call.this_int(0))))
}

// ---- the classes -----------------------------------------------------------------

pub(super) const CONDITIONAL_FORMAT: ApiClass = class!("ConditionalFormat", "A conditional format: a rule, the cells it applies to and its place among the rules of the sheet.", {
    getId "(): string" "An identifier of the rule for this run of the script (Range.getConditionalFormat finds it again)." => |h, c| {
        rule(h, c)?;
        Ok(c.this_int(0).to_string().into())
    };
    getType "(): ExcelScript.ConditionalFormatType" "The kind of rule: CellValue, ColorScale, DataBar, IconSet, TopBottom, PresetCriteria, ContainsText or Custom." => |h, c| Ok(kind(&rule(h, c)?.2.rule).into());
    getPriority "(): number" "The place of the rule among the rules of its worksheet: 0 is looked at first." => |h, c| Ok((rule(h, c)?.1 as u64).into());
    setPriority "(priority: number): void" "Moves the rule among the rules of its worksheet (0 first), as Move Up and Move Down of the Rules Manager." => |h, c| {
        let (sheet, from, _) = rule(h, c)?;
        let count = h.wb.sheets[sheet].cond_formats.len();
        let to = usize::try_from(c.int(0)?).map_err(|_| c.invalid())?.min(count - 1);
        if to == from {
            return Ok(Ret::Nothing);
        }
        h.permit(c, &Action::FormatCells { sheet })?;
        h.wb.reorder_cond_format(sheet, from, to);
        h.cond_formats_moved(sheet, |index| {
            Some(match index {
                i if i == from => to,
                i if from < i && i <= to => i - 1,
                i if to <= i && i < from => i + 1,
                i => i,
            })
        });
        Ok(Ret::Nothing)
    };
    getStopIfTrue "(): boolean" "Whether rules of lower priority are left out for the cells this one matches." => |h, c| Ok(rule(h, c)?.2.stop_if_true.into());
    setStopIfTrue "(stopIfTrue: boolean): void" "Stop If True: rules of lower priority are not looked at for the cells this one matches." => |h, c| {
        let stop = c.bool(0)?;
        edit(h, c, |cf| {
            cf.stop_if_true = stop;
            Ok(())
        })
    };
    getRange "(): Range" "The range the rule applies to. Fails when it applies to several: use getRanges." => |h, c| {
        let (sheet, _, cf) = rule(h, c)?;
        match cf.ranges[..] {
            [one] => Ok(Ret::Object(h.range(sheet, one))),
            _ => Err(c.fail("The conditional format applies to several ranges.")),
        }
    };
    getRanges "(): RangeAreas" "The ranges the rule applies to." => |h, c| {
        let (sheet, _, cf) = rule(h, c)?;
        Ok(Ret::Object(areas_object(h, sheet, &cf.ranges)))
    };
    setRanges "(ranges: Range | RangeAreas | string): void" "Changes the cells the rule applies to (Applies To): its formulas go on reading the same cells from each cell." => |h, c| {
        let (sheet, index, _) = rule(h, c)?;
        let (on, ranges) = areas_arg(h, c, 0, sheet)?;
        if on != sheet {
            return Err(c.fail("A conditional format stays on its worksheet."));
        }
        h.permit(c, &Action::FormatCells { sheet })?;
        h.wb.set_cond_format_ranges(sheet, index, ranges);
        Ok(Ret::Nothing)
    };
    delete "(): void" "Deletes the rule." => |h, c| {
        let (sheet, index, _) = rule(h, c)?;
        h.permit(c, &Action::FormatCells { sheet })?;
        h.wb.remove_cond_format(sheet, index);
        h.cond_formats_moved(sheet, |i| match i.cmp(&index) {
            std::cmp::Ordering::Less => Some(i),
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(i - 1),
        });
        Ok(Ret::Nothing)
    };
    getCellValue "(): CellValueConditionalFormat | undefined" "The rule as a comparison of the cell with values; undefined for another kind." => |h, c| part(h, c, "CellValueConditionalFormat", "CellValue");
    getColorScale "(): ColorScaleConditionalFormat | undefined" "The rule as a colour scale; undefined for another kind." => |h, c| part(h, c, "ColorScaleConditionalFormat", "ColorScale");
    getDataBar "(): DataBarConditionalFormat | undefined" "The rule as data bars; undefined for another kind." => |h, c| part(h, c, "DataBarConditionalFormat", "DataBar");
    getIconSet "(): IconSetConditionalFormat | undefined" "The rule as an icon set; undefined for another kind." => |h, c| part(h, c, "IconSetConditionalFormat", "IconSet");
    getPreset "(): PresetCriteriaConditionalFormat | undefined" "The rule as a preset criterion (blanks, errors, dates, average, duplicates); undefined for another kind." => |h, c| part(h, c, "PresetCriteriaConditionalFormat", "PresetCriteria");
    getTextComparison "(): TextConditionalFormat | undefined" "The rule as a comparison of the text of the cell; undefined for another kind." => |h, c| part(h, c, "TextConditionalFormat", "ContainsText");
    getTopBottom "(): TopBottomConditionalFormat | undefined" "The rule as top or bottom values; undefined for another kind." => |h, c| part(h, c, "TopBottomConditionalFormat", "TopBottom");
    getCustom "(): CustomConditionalFormat | undefined" "The rule as a formula; undefined for another kind." => |h, c| part(h, c, "CustomConditionalFormat", "Custom");
});

pub(super) const CELL_VALUE: ApiClass = class!("CellValueConditionalFormat", "A rule that compares the value of each cell with one or two values.", {
    getFormat "(): ConditionalRangeFormat" "The format of the cells that match." => |h, c| format_object(h, c);
    getRule "(): ExcelScript.ConditionalCellValueRule" "The comparison: { formula1, formula2?, operator }." => |h, c| {
        let (_, _, cf) = rule(h, c)?;
        let Rule::CellIs { op, first, second } = &cf.rule else {
            return Err(c.gone());
        };
        let operator = CELL_OPS.iter().find(|(_, o)| o == op).map_or("Invalid", |(name, _)| name);
        let mut out = json!({ "formula1": operand_out(first, cf.anchor()), "operator": operator });
        if let Some(second) = second {
            out["formula2"] = operand_out(second, cf.anchor()).into();
        }
        Ok(Ret::Value(out))
    };
    setRule "(rule: ExcelScript.ConditionalCellValueRule): void" "Sets the comparison: an operator and one value or formula, two for Between and NotBetween." => |h, c| {
        let given = c.arg(0);
        let name = field(c, given, "operator")?;
        let op = CELL_OPS.iter().find(|(n, _)| names_eq(n, name)).map(|(_, op)| *op).ok_or_else(|| c.invalid())?;
        edit(h, c, |cf| {
            let first = operand(c, given.get("formula1").ok_or_else(|| c.invalid())?, cf.anchor())?;
            let second = match given.get("formula2").filter(|v| !v.is_null()) {
                Some(value) => Some(operand(c, value, cf.anchor())?),
                None if matches!(op, CellOp::Between | CellOp::NotBetween) => return Err(c.invalid()),
                None => None,
            };
            cf.rule = Rule::cell_is(op, first, second.filter(|_| matches!(op, CellOp::Between | CellOp::NotBetween)));
            Ok(())
        })
    };
});

pub(super) const COLOR_SCALE: ApiClass = class!("ColorScaleConditionalFormat", "A rule that colours each cell between two or three colours, by its value.", {
    getThreeColorScale "(): boolean" "Whether the scale has a midpoint." => |h, c| Ok((scale(h, c)?.0.stops.len() == 3).into());
    getCriteria "(): ExcelScript.ConditionalColorScaleCriteria" "The ends of the scale and its midpoint: { minimum, midpoint?, maximum }, each { type, formula?, color }." => |h, c| {
        let (scale, anchor) = scale(h, c)?;
        let stop = |stop: &ColorStop| {
            let (kind, formula) = threshold_out(&stop.at, anchor);
            let mut out = json!({ "type": kind, "color": color_text(stop.color) });
            if let Some(formula) = formula {
                out["formula"] = formula.into();
            }
            out
        };
        let (first, last) = (scale.stops.first().ok_or_else(|| c.gone())?, scale.stops.last().ok_or_else(|| c.gone())?);
        let mut out = json!({ "minimum": stop(first), "maximum": stop(last) });
        if let [_, middle, _] = &scale.stops[..] {
            out["midpoint"] = stop(middle);
        }
        Ok(Ret::Value(out))
    };
    setCriteria "(criteria: ExcelScript.ConditionalColorScaleCriteria): void" "Sets the ends of the scale, and its midpoint for a scale of three colours." => |h, c| {
        let given = c.arg(0);
        edit(h, c, |cf| {
            let anchor = cf.anchor();
            let stop = |key: &str| -> Res<Option<ColorStop>> {
                let Some(of) = given.get(key).filter(|v| !v.is_null()) else {
                    return Ok(None);
                };
                let color = parse_color(field(c, of, "color")?).ok_or_else(|| c.invalid())?;
                let at = threshold(c, field(c, of, "type")?, of.get("formula"), anchor)?;
                Ok(Some(ColorStop { at, color }))
            };
            let (minimum, maximum) = (stop("minimum")?.ok_or_else(|| c.invalid())?, stop("maximum")?.ok_or_else(|| c.invalid())?);
            let stops = [Some(minimum), stop("midpoint")?, Some(maximum)].into_iter().flatten().collect();
            cf.rule = Rule::ColorScale(ColorScale { stops });
            Ok(())
        })
    };
});

pub(super) const DATA_BAR: ApiClass = class!("DataBarConditionalFormat", "A rule that draws in each cell a bar as long as its value is large.", {
    getAxisColor "(): string" "The colour of the axis between negative and positive bars, as #RRGGBB." => |h, c| Ok(color_text(bar(h, c)?.0.axis_color).into());
    setAxisColor "(axisColor: string): void" "Colours the axis (#RRGGBB or a colour name)." => |h, c| {
        let color = optional_color(c, 0)?;
        edit_bar(h, c, |bar, _| {
            bar.axis_color = color;
            Ok(())
        })
    };
    getAxisFormat "(): ExcelScript.ConditionalDataBarAxisFormat" "Where the axis is: Automatic, CellMidPoint or None." => |h, c| {
        Ok(match bar(h, c)?.0.axis {
            BarAxis::Automatic => "Automatic",
            BarAxis::Midpoint => "CellMidPoint",
            BarAxis::None => "None",
        }.into())
    };
    setAxisFormat "(axisFormat: ExcelScript.ConditionalDataBarAxisFormat): void" "Places the axis: by the values, in the middle of the cell, or none (negative bars then run the same way as positive ones)." => |h, c| {
        let axis = match enum_value(c, 0, "ConditionalDataBarAxisFormat")?.ok_or_else(|| c.invalid())? {
            "CellMidPoint" => BarAxis::Midpoint,
            "None" => BarAxis::None,
            _ => BarAxis::Automatic,
        };
        edit_bar(h, c, |bar, _| {
            bar.axis = axis;
            Ok(())
        })
    };
    getBarDirection "(): ExcelScript.ConditionalDataBarDirection" "The way the bars run: Context, LeftToRight or RightToLeft." => |h, c| {
        Ok(match bar(h, c)?.0.direction {
            BarDirection::Context => "Context",
            BarDirection::LeftToRight => "LeftToRight",
            BarDirection::RightToLeft => "RightToLeft",
        }.into())
    };
    setBarDirection "(barDirection: ExcelScript.ConditionalDataBarDirection): void" "Sets the way the bars run." => |h, c| {
        let direction = match enum_value(c, 0, "ConditionalDataBarDirection")?.ok_or_else(|| c.invalid())? {
            "LeftToRight" => BarDirection::LeftToRight,
            "RightToLeft" => BarDirection::RightToLeft,
            _ => BarDirection::Context,
        };
        edit_bar(h, c, |bar, _| {
            bar.direction = direction;
            Ok(())
        })
    };
    getShowDataBarOnly "(): boolean" "Whether the cells show their bar without their value." => |h, c| Ok((!bar(h, c)?.0.show_value).into());
    setShowDataBarOnly "(showDataBarOnly: boolean): void" "Shows the bar alone, or the bar with the value." => |h, c| {
        let only = c.bool(0)?;
        edit_bar(h, c, |bar, _| {
            bar.show_value = !only;
            Ok(())
        })
    };
    getLowerBoundRule "(): ExcelScript.ConditionalDataBarRule" "What the shortest bar stands for: { type, formula? }." => |h, c| {
        let (bar, anchor) = bar(h, c)?;
        Ok(bar_end_out(&bar.min, anchor))
    };
    setLowerBoundRule "(lowerBoundRule: ExcelScript.ConditionalDataBarRule): void" "Sets what the shortest bar stands for: Automatic, LowestValue, Number, Percent, Percentile or Formula." => |h, c| {
        edit_bar(h, c, |bar, anchor| {
            bar.min = bar_end(c, anchor)?;
            Ok(())
        })
    };
    getUpperBoundRule "(): ExcelScript.ConditionalDataBarRule" "What the longest bar stands for: { type, formula? }." => |h, c| {
        let (bar, anchor) = bar(h, c)?;
        Ok(bar_end_out(&bar.max, anchor))
    };
    setUpperBoundRule "(upperBoundRule: ExcelScript.ConditionalDataBarRule): void" "Sets what the longest bar stands for: Automatic, HighestValue, Number, Percent, Percentile or Formula." => |h, c| {
        edit_bar(h, c, |bar, anchor| {
            bar.max = bar_end(c, anchor)?;
            Ok(())
        })
    };
    getPositiveFormat "(): ConditionalDataBarPositiveFormat" "The look of the bars of positive values." => |h, c| {
        bar(h, c)?;
        Ok(Ret::Object(handle_object("ConditionalDataBarPositiveFormat", c.this_int(0))))
    };
    getNegativeFormat "(): ConditionalDataBarNegativeFormat" "The look of the bars of negative values." => |h, c| {
        bar(h, c)?;
        Ok(Ret::Object(handle_object("ConditionalDataBarNegativeFormat", c.this_int(0))))
    };
});

pub(super) const BAR_POSITIVE: ApiClass = class!("ConditionalDataBarPositiveFormat", "The look of the data bars of positive values.", {
    getFillColor "(): string" "The colour of the bars, as #RRGGBB." => |h, c| Ok(color_text(bar(h, c)?.0.fill).into());
    setFillColor "(fillColor: string): void" "Colours the bars (#RRGGBB or a colour name)." => |h, c| {
        let color = optional_color(c, 0)?;
        edit_bar(h, c, |bar, _| {
            bar.fill = color;
            Ok(())
        })
    };
    getGradientFill "(): boolean" "Whether the bars fade towards their end rather than being of one colour." => |h, c| Ok(bar(h, c)?.0.gradient.into());
    setGradientFill "(gradientFill: boolean): void" "Gradient fill or solid fill." => |h, c| {
        let gradient = c.bool(0)?;
        edit_bar(h, c, |bar, _| {
            bar.gradient = gradient;
            Ok(())
        })
    };
    getBorderColor "(): string" "The colour of the line around the bars, as #RRGGBB; empty text without a line." => |h, c| Ok(bar(h, c)?.0.border.map(color_text).unwrap_or_default().into());
    setBorderColor "(borderColor: string): void" "Draws a line around the bars; empty text removes it." => |h, c| {
        let color = color_arg(c, 0)?;
        edit_bar(h, c, |bar, _| {
            bar.border = color;
            Ok(())
        })
    };
});

pub(super) const BAR_NEGATIVE: ApiClass = class!("ConditionalDataBarNegativeFormat", "The look of the data bars of negative values.", {
    getFillColor "(): string" "The colour of the negative bars, as #RRGGBB (that of the positive bars when they match)." => |h, c| {
        let bar = bar(h, c)?.0;
        Ok(color_text(bar.negative_fill.unwrap_or(bar.fill)).into())
    };
    setFillColor "(fillColor: string): void" "Gives the negative bars a colour of their own." => |h, c| {
        let color = optional_color(c, 0)?;
        edit_bar(h, c, |bar, _| {
            bar.negative_fill = Some(color);
            Ok(())
        })
    };
    getMatchPositiveFillColor "(): boolean" "Whether negative bars have the colour of the positive ones." => |h, c| Ok(bar(h, c)?.0.negative_fill.is_none().into());
    setMatchPositiveFillColor "(matchPositiveFillColor: boolean): void" "Negative bars take the colour of the positive ones, or keep one of their own (red to start with)." => |h, c| {
        let same = c.bool(0)?;
        edit_bar(h, c, |bar, _| {
            bar.negative_fill = if same { None } else { Some(bar.negative_fill.unwrap_or(Color(0xFF0000))) };
            Ok(())
        })
    };
    getBorderColor "(): string" "The colour of the line around negative bars, as #RRGGBB; empty text when they have none of their own." => |h, c| {
        Ok(bar(h, c)?.0.negative_border.map(color_text).unwrap_or_default().into())
    };
    setBorderColor "(borderColor: string): void" "Gives the line around negative bars a colour of its own." => |h, c| {
        let color = optional_color(c, 0)?;
        edit_bar(h, c, |bar, _| {
            bar.negative_border = Some(color);
            Ok(())
        })
    };
    getMatchPositiveBorderColor "(): boolean" "Whether the line around negative bars is that of the positive ones." => |h, c| Ok(bar(h, c)?.0.negative_border.is_none().into());
    setMatchPositiveBorderColor "(matchPositiveBorderColor: boolean): void" "The line around negative bars takes the colour of the positive ones, or keeps one of its own (red to start with)." => |h, c| {
        let same = c.bool(0)?;
        edit_bar(h, c, |bar, _| {
            bar.negative_border = if same { None } else { Some(bar.negative_border.unwrap_or(Color(0xFF0000))) };
            Ok(())
        })
    };
});

pub(super) const ICON_SET: ApiClass = class!("IconSetConditionalFormat", "A rule that shows in each cell an icon of a set, by its value.", {
    getStyle "(): ExcelScript.IconSet" "The set of icons: ThreeArrows, FourRating, FiveBoxes..." => |h, c| Ok(format!("{:?}", icons(h, c)?.0.set).into());
    setStyle "(style: ExcelScript.IconSet): void" "Changes the set of icons; a set of another size starts from bands of equal width." => |h, c| {
        let set = icon_set(c, enum_value(c, 0, "IconSet")?.ok_or_else(|| c.invalid())?)?;
        edit_icons(h, c, |icons, _| {
            if set.len() == icons.set.len() {
                icons.set = set;
            } else {
                *icons = IconRule {
                    reverse: icons.reverse,
                    show_value: icons.show_value,
                    ..IconRule::preset(set)
                };
            }
            Ok(())
        })
    };
    getReverseIconOrder "(): boolean" "Whether the icons are given in the reverse order." => |h, c| Ok(icons(h, c)?.0.reverse.into());
    setReverseIconOrder "(reverseIconOrder: boolean): void" "Gives the icons in the reverse order: the last one to the lowest values." => |h, c| {
        let reverse = c.bool(0)?;
        edit_icons(h, c, |icons, _| {
            icons.reverse = reverse;
            Ok(())
        })
    };
    getShowIconOnly "(): boolean" "Whether the cells show their icon without their value." => |h, c| Ok((!icons(h, c)?.0.show_value).into());
    setShowIconOnly "(showIconOnly: boolean): void" "Shows the icon alone, or the icon with the value." => |h, c| {
        let only = c.bool(0)?;
        edit_icons(h, c, |icons, _| {
            icons.show_value = !only;
            Ok(())
        })
    };
    getCriteria "(): ExcelScript.ConditionalIconCriterion[]" "One criterion per icon, the icon of the lowest values first: { type, operator, formula, customIcon? }. The first has no threshold." => |h, c| {
        let (icons, anchor) = icons(h, c)?;
        let out: Vec<J> = icons.thresholds.iter().enumerate().map(|(band, threshold)| {
            let (kind, formula) = threshold_out(&threshold.value, anchor);
            let mut out = if band == 0 {
                json!({ "type": "Invalid", "operator": "Invalid", "formula": "" })
            } else {
                json!({
                    "type": kind,
                    "operator": if threshold.gte { "GreaterThanOrEqual" } else { "GreaterThan" },
                    "formula": formula.unwrap_or_default(),
                })
            };
            if let Some(Some((set, index))) = icons.custom.as_ref().and_then(|custom| custom.get(band)) {
                out["customIcon"] = json!({ "set": format!("{set:?}"), "index": index });
            }
            out
        }).collect();
        Ok(Ret::Value(J::from(out)))
    };
    setCriteria "(criteria: ExcelScript.ConditionalIconCriterion[]): void" "Sets the criterion of each icon, the icon of the lowest values first; the first one needs no threshold. customIcon gives a band the icon of another set." => |h, c| {
        let given = c.array(0)?;
        edit_icons(h, c, |icons, anchor| {
            if given.len() != icons.set.len() {
                return Err(c.fail(format!("This icon set needs {} criteria.", icons.set.len())));
            }
            let mut thresholds = Vec::new();
            let mut custom = Vec::new();
            for (band, criterion) in given.iter().enumerate() {
                thresholds.push(if band == 0 {
                    IconThreshold { value: Cfvo::percent(0.0), gte: true }
                } else {
                    let value = threshold(c, field(c, criterion, "type")?, criterion.get("formula"), anchor)?;
                    if matches!(value, Cfvo::Min | Cfvo::Max | Cfvo::Automatic) {
                        return Err(c.invalid());
                    }
                    IconThreshold { value, gte: field(c, criterion, "operator")? != "GreaterThan" }
                });
                custom.push(match criterion.get("customIcon").filter(|icon| !icon.is_null()) {
                    Some(icon) => {
                        let set = icon_set(c, field(c, icon, "set")?)?;
                        let index = icon.get("index").and_then(J::as_u64).filter(|i| (*i as usize) < set.len()).ok_or_else(|| c.invalid())?;
                        Some((set, index as u8))
                    }
                    None => None,
                });
            }
            icons.thresholds = thresholds;
            icons.custom = custom.iter().any(Option::is_some).then_some(custom);
            Ok(())
        })
    };
});

pub(super) const PRESET: ApiClass = class!("PresetCriteriaConditionalFormat", "A rule with a ready-made criterion: blanks, errors, dates, values above or below the average, duplicates.", {
    getFormat "(): ConditionalRangeFormat" "The format of the cells that match." => |h, c| format_object(h, c);
    getRule "(): ExcelScript.ConditionalPresetCriteriaRule" "The criterion: { criterion }." => |h, c| {
        let (_, _, cf) = rule(h, c)?;
        let name = presets().into_iter().find(|(_, rule)| *rule == cf.rule).map(|(name, _)| name).ok_or_else(|| c.gone())?;
        Ok(Ret::Value(json!({ "criterion": name })))
    };
    setRule "(rule: ExcelScript.ConditionalPresetCriteriaRule): void" "Sets the criterion, a member of ExcelScript.ConditionalFormatPresetCriterion." => |h, c| {
        let name = field(c, c.arg(0), "criterion")?;
        let new = presets().into_iter().find(|(n, _)| names_eq(n, name)).map(|(_, rule)| rule).ok_or_else(|| c.invalid())?;
        edit(h, c, |cf| {
            cf.rule = new;
            Ok(())
        })
    };
});

pub(super) const TEXT: ApiClass = class!("TextConditionalFormat", "A rule on the text of each cell: contains, does not contain, begins with, ends with.", {
    getFormat "(): ConditionalRangeFormat" "The format of the cells that match." => |h, c| format_object(h, c);
    getRule "(): ExcelScript.ConditionalTextComparisonRule" "The comparison: { operator, text }." => |h, c| {
        let (_, _, cf) = rule(h, c)?;
        let Rule::Text { op, text } = &cf.rule else {
            return Err(c.gone());
        };
        let operator = TEXT_OPS.iter().find(|(_, o)| o == op).map_or("Invalid", |(name, _)| name);
        let text = operand_text(text, Locale::en(), cf.anchor(), cf.anchor(), true);
        Ok(Ret::Value(json!({ "operator": operator, "text": text })))
    };
    setRule "(rule: ExcelScript.ConditionalTextComparisonRule): void" "Sets the comparison: an operator and the text looked for." => |h, c| {
        let given = c.arg(0);
        let name = field(c, given, "operator")?;
        let op = TEXT_OPS.iter().find(|(n, _)| names_eq(n, name)).map(|(_, op)| *op).ok_or_else(|| c.invalid())?;
        let text = Expr::Text(Arc::from(field(c, given, "text")?));
        edit(h, c, |cf| {
            cf.rule = Rule::Text { op, text };
            Ok(())
        })
    };
});

pub(super) const TOP_BOTTOM: ApiClass = class!("TopBottomConditionalFormat", "A rule on the highest or the lowest values of the range.", {
    getFormat "(): ConditionalRangeFormat" "The format of the cells that match." => |h, c| format_object(h, c);
    getRule "(): ExcelScript.ConditionalTopBottomRule" "The criterion: { rank, type }." => |h, c| {
        let Rule::TopBottom { rank, percent, bottom } = rule(h, c)?.2.rule else {
            return Err(c.gone());
        };
        let kind = match (bottom, percent) {
            (false, false) => "TopItems",
            (false, true) => "TopPercent",
            (true, false) => "BottomItems",
            (true, true) => "BottomPercent",
        };
        Ok(Ret::Value(json!({ "rank": rank, "type": kind })))
    };
    setRule "(rule: ExcelScript.ConditionalTopBottomRule): void" "Sets the criterion: how many (1 to 1000, or 1 to 100 percent) and TopItems, TopPercent, BottomItems or BottomPercent." => |h, c| {
        let given = c.arg(0);
        let (bottom, percent) = match field(c, given, "type")? {
            "TopItems" => (false, false),
            "TopPercent" => (false, true),
            "BottomItems" => (true, false),
            "BottomPercent" => (true, true),
            _ => return Err(c.invalid()),
        };
        let most = if percent { 100 } else { 1000 };
        let rank = given.get("rank").and_then(J::as_u64).filter(|rank| (1..=most).contains(rank)).ok_or_else(|| c.invalid())? as u32;
        edit(h, c, |cf| {
            cf.rule = Rule::TopBottom { rank, percent, bottom };
            Ok(())
        })
    };
});

pub(super) const CUSTOM: ApiClass = class!("CustomConditionalFormat", "A rule whose test is a formula.", {
    getFormat "(): ConditionalRangeFormat" "The format of the cells for which the formula is true." => |h, c| format_object(h, c);
    getRule "(): ConditionalFormatRule" "The formula of the rule." => |h, c| {
        rule(h, c)?;
        Ok(Ret::Object(handle_object("ConditionalFormatRule", c.this_int(0))))
    };
});

/// The formula of a custom rule, in a language.
fn formula_out(h: &Host, call: &Call, locale: &'static Locale) -> Res<Ret> {
    let (_, _, cf) = rule(h, call)?;
    match &cf.rule {
        Rule::Formula(expr) => Ok(format!("={}", expr.to_text(locale)).into()),
        _ => Err(call.gone()),
    }
}

fn set_formula(h: &mut Host, call: &Call, locale: &'static Locale) -> Res<Ret> {
    let text = call.str(0)?;
    if !text.trim_start().starts_with('=') {
        return Err(call.invalid());
    }
    edit(h, call, |cf| {
        let anchor = cf.anchor();
        cf.rule = Rule::Formula(parse_operand(text, locale, anchor, anchor).map_err(|_| call.invalid())?);
        Ok(())
    })
}

pub(super) const FORMULA_RULE: ApiClass = class!("ConditionalFormatRule", "The formula of a custom rule, written for the first cell of its range.", {
    getFormula "(): string" "The formula, in English (=A1>10)." => |h, c| formula_out(h, c, Locale::en());
    setFormula "(formula: string): void" "Sets the formula, in English; it starts with =." => |h, c| set_formula(h, c, Locale::en());
    getFormulaLocal "(): string" "The formula in the language of the workbook." => |h, c| formula_out(h, c, h.wb.locale);
    setFormulaLocal "(formulaLocal: string): void" "Sets the formula, written in the language of the workbook." => |h, c| {
        let locale = h.wb.locale;
        set_formula(h, c, locale)
    };
});

pub(super) const RANGE_FORMAT: ApiClass = class!("ConditionalRangeFormat", "The format a rule gives the cells that match: fill, font, borders, number format.", {
    getFill "(): ConditionalRangeFill" "The fill." => |h, c| {
        rule(h, c)?;
        Ok(Ret::Object(handle_object("ConditionalRangeFill", c.this_int(0))))
    };
    getFont "(): ConditionalRangeFont" "The font: bold, italic, underline, strikethrough, colour." => |h, c| {
        rule(h, c)?;
        Ok(Ret::Object(handle_object("ConditionalRangeFont", c.this_int(0))))
    };
    getBorders "(): ConditionalRangeBorder[]" "The four borders: top, bottom, left, right." => |h, c| {
        rule(h, c)?;
        Ok(Ret::Object(J::from((0..4).map(|side| border_object(c, side)).collect::<Vec<J>>())))
    };
    getConditionalRangeBorder "(index: ExcelScript.ConditionalRangeBorderIndex): ConditionalRangeBorder" "One border: EdgeTop, EdgeBottom, EdgeLeft or EdgeRight." => |h, c| {
        rule(h, c)?;
        let name = enum_value(c, 0, "ConditionalRangeBorderIndex")?.ok_or_else(|| c.invalid())?;
        let side = BORDER_SIDES.iter().position(|s| *s == name).ok_or_else(|| c.invalid())?;
        Ok(Ret::Object(border_object(c, side)))
    };
    getConditionalRangeBorderTop "(): ConditionalRangeBorder" "The top border." => |h, c| {
        rule(h, c)?;
        Ok(Ret::Object(border_object(c, 0)))
    };
    getConditionalRangeBorderBottom "(): ConditionalRangeBorder" "The bottom border." => |h, c| {
        rule(h, c)?;
        Ok(Ret::Object(border_object(c, 1)))
    };
    getConditionalRangeBorderLeft "(): ConditionalRangeBorder" "The left border." => |h, c| {
        rule(h, c)?;
        Ok(Ret::Object(border_object(c, 2)))
    };
    getConditionalRangeBorderRight "(): ConditionalRangeBorder" "The right border." => |h, c| {
        rule(h, c)?;
        Ok(Ret::Object(border_object(c, 3)))
    };
    getNumberFormat "(): string" "The number format code the rule gives, in English; empty text when it leaves the cells' own." => |h, c| {
        Ok(patch(h, c)?.num_fmt.as_deref().unwrap_or_default().into())
    };
    setNumberFormat "(numberFormat: string): void" "Gives the cells that match a number format code, in English; empty text leaves them their own." => |h, c| {
        let code = c.str(0)?;
        edit_patch(h, c, |patch| {
            patch.num_fmt = (!code.is_empty()).then(|| Arc::from(code));
            Ok(())
        })
    };
});

pub(super) const RANGE_FILL: ApiClass = class!("ConditionalRangeFill", "The fill a rule gives the cells that match.", {
    getColor "(): string" "The fill colour as #RRGGBB; empty text when the rule sets none." => |h, c| {
        Ok(patch(h, c)?.fill.flatten().map(color_text).unwrap_or_default().into())
    };
    setColor "(color: string): void" "Fills the cells that match (#RRGGBB or a colour name)." => |h, c| {
        let color = optional_color(c, 0)?;
        edit_patch(h, c, |patch| {
            patch.fill = Some(Some(color));
            Ok(())
        })
    };
    clear "(): void" "The rule no longer changes the fill." => |h, c| {
        edit_patch(h, c, |patch| {
            patch.fill = None;
            Ok(())
        })
    };
});

pub(super) const RANGE_FONT: ApiClass = class!("ConditionalRangeFont", "The font a rule gives the cells that match.", {
    getBold "(): boolean" "Whether the rule makes the text bold." => |h, c| Ok(patch(h, c)?.bold.unwrap_or(false).into());
    setBold "(bold: boolean): void" "Bold or not for the cells that match." => |h, c| {
        let on = c.bool(0)?;
        edit_patch(h, c, |patch| {
            patch.bold = Some(on);
            Ok(())
        })
    };
    getItalic "(): boolean" "Whether the rule makes the text italic." => |h, c| Ok(patch(h, c)?.italic.unwrap_or(false).into());
    setItalic "(italic: boolean): void" "Italic or not for the cells that match." => |h, c| {
        let on = c.bool(0)?;
        edit_patch(h, c, |patch| {
            patch.italic = Some(on);
            Ok(())
        })
    };
    getStrikethrough "(): boolean" "Whether the rule strikes the text through." => |h, c| Ok(patch(h, c)?.strike.unwrap_or(false).into());
    setStrikethrough "(strikethrough: boolean): void" "Struck through or not for the cells that match." => |h, c| {
        let on = c.bool(0)?;
        edit_patch(h, c, |patch| {
            patch.strike = Some(on);
            Ok(())
        })
    };
    getUnderline "(): ExcelScript.ConditionalRangeFontUnderlineStyle" "The underline the rule gives: None, Single or Double." => |h, c| {
        Ok(match patch(h, c)?.underline.unwrap_or_default() {
            Underline::None => "None",
            Underline::Single => "Single",
            Underline::Double => "Double",
        }.into())
    };
    setUnderline "(underline: ExcelScript.ConditionalRangeFontUnderlineStyle): void" "Underlines the text of the cells that match: None, Single or Double." => |h, c| {
        let underline = match enum_value(c, 0, "ConditionalRangeFontUnderlineStyle")?.ok_or_else(|| c.invalid())? {
            "Single" => Underline::Single,
            "Double" => Underline::Double,
            _ => Underline::None,
        };
        edit_patch(h, c, |patch| {
            patch.underline = Some(underline);
            Ok(())
        })
    };
    getColor "(): string" "The colour of the text as #RRGGBB; empty text when the rule sets none." => |h, c| {
        Ok(patch(h, c)?.font_color.flatten().map(color_text).unwrap_or_default().into())
    };
    setColor "(color: string): void" "Colours the text of the cells that match (#RRGGBB or a colour name)." => |h, c| {
        let color = optional_color(c, 0)?;
        edit_patch(h, c, |patch| {
            patch.font_color = Some(Some(color));
            Ok(())
        })
    };
    clear "(): void" "The rule no longer changes the font." => |h, c| {
        edit_patch(h, c, |patch| {
            (patch.bold, patch.italic, patch.strike, patch.underline, patch.font_color) = (None, None, None, None, None);
            Ok(())
        })
    };
});

pub(super) const RANGE_BORDER: ApiClass = class!("ConditionalRangeBorder", "One border a rule gives the cells that match.", {
    getSideIndex "(): ExcelScript.ConditionalRangeBorderIndex" "Which border it is: EdgeTop, EdgeBottom, EdgeLeft or EdgeRight." => |h, c| {
        rule(h, c)?;
        Ok(BORDER_SIDES.get(c.this_int(1) as usize).copied().unwrap_or("EdgeRight").into())
    };
    getStyle "(): ExcelScript.ConditionalRangeBorderLineStyle" "The line: None, Continuous, Dash or Dot." => |h, c| {
        Ok(match border_of(h, c)?.map(|border| border.style) {
            None | Some(BorderStyle::None) => "None",
            Some(BorderStyle::Dashed) => "Dash",
            Some(BorderStyle::Dotted) => "Dot",
            Some(_) => "Continuous",
        }.into())
    };
    setStyle "(style: ExcelScript.ConditionalRangeBorderLineStyle): void" "Draws the border: Continuous, Dash or Dot; None removes it. Rules have thin lines only." => |h, c| {
        let style = match enum_value(c, 0, "ConditionalRangeBorderLineStyle")?.ok_or_else(|| c.invalid())? {
            "None" => BorderStyle::None,
            "Continuous" => BorderStyle::Thin,
            "Dash" => BorderStyle::Dashed,
            "Dot" => BorderStyle::Dotted,
            other => return Err(c.unsupported(format!("ExcelScript.ConditionalRangeBorderLineStyle.{}{}", other[..1].to_lowercase(), &other[1..]))),
        };
        let side = c.this_int(1);
        edit_patch(h, c, |patch| {
            let slot = border_slot(patch, side);
            *slot = Some(Border { style, color: slot.and_then(|border| border.color) });
            Ok(())
        })
    };
    getColor "(): string" "The colour of the border as #RRGGBB; empty text for the automatic colour." => |h, c| {
        Ok(border_of(h, c)?.and_then(|border| border.color).map(color_text).unwrap_or_default().into())
    };
    setColor "(color: string): void" "Colours the border (a thin line when there was none); empty text gives the automatic colour." => |h, c| {
        let color = color_arg(c, 0)?;
        let side = c.this_int(1);
        edit_patch(h, c, |patch| {
            let slot = border_slot(patch, side);
            let style = slot.map(|border| border.style).filter(|style| *style != BorderStyle::None).unwrap_or(BorderStyle::Thin);
            *slot = Some(Border { style, color });
            Ok(())
        })
    };
});
