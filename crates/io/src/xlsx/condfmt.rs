//! Conditional formatting: `<conditionalFormatting>` in a worksheet, and
//! what Excel 2010 added to it in the worksheet's extension list.
//!
//! A rule is stored in one of three ways:
//!
//! * most rules, as Excel 2007 knew them: a `<cfRule>` under the
//!   `<conditionalFormatting>` of its cells, its format a `dxfId`;
//! * a data bar with any of the newer options (border, solid fill,
//!   negative colours, axis, direction, automatic ends), twice: the 2007
//!   rule, and an `<x14:cfRule>` in the extension list holding the rest.
//!   An id ties the two together;
//! * what 2007 could not express at all — the newer icon sets, icons
//!   picked one by one, and rules whose formulas refer to another sheet —
//!   only in the extension list, format inline (`<x14:dxf>`).
//!
//! The rules of a sheet have one order, their priority, whichever place
//! each is stored in.

use super::color::{Palette, RawColor};
use super::dxf::{Dxfs, push_color, read_dxf};
use super::formula::{from_file_text, to_file_text};
use super::validation::{names_a_sheet, parse_sqref};
use super::write::push_range;
use super::xml::{Event, Reader, Tag, decode_cell_text, push_text, push_xstring_attr};
use opensheet_engine::ast::Expr;
use opensheet_engine::condfmt::{
    BarAxis, BarDirection, CellOp, Cfvo, ColorScale, ColorStop, CondFormat, DataBar, IconRule,
    IconSet, IconThreshold, Rule, TextOp, TimePeriod, ooxml_formulas, text_operand_from_ooxml,
};
use opensheet_engine::refs::Rect;
use opensheet_engine::style::StylePatch;
use opensheet_engine::{Color, Locale, Sheet, Workbook};
use std::collections::HashMap;
use std::fmt::Write as _;

/// The extension of a worksheet that holds `x14:conditionalFormattings`.
const EXT_URI: &str = "{78C0D931-6437-407d-A8EE-F0AAD7539E65}";
/// The extension of a 2007 rule that holds the id of its other half.
const ID_EXT_URI: &str = "{B025F937-C7B1-47D3-B67F-A62EFF666E3E}";
const NS_X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
const NS_XM: &str = "http://schemas.microsoft.com/office/excel/2006/main";

/// The colours Excel gives the negative bars and the axis of a data bar
/// when a file does not say.
const NEGATIVE_RED: Color = Color(0xFF0000);

// ---- writing -------------------------------------------------------------------

/// Where a rule is stored.
#[derive(Clone, Copy, PartialEq)]
enum Form {
    /// As a 2007 rule only.
    Plain,
    /// As a 2007 rule and an x14 rule tied by an id.
    Linked,
    /// In the extension list only.
    Extension,
}

/// A data bar that says nothing 2007 could not: it needs no x14 half.
fn is_2007_bar(bar: &DataBar) -> bool {
    bar.gradient
        && bar.border.is_none()
        && bar.negative_fill.is_none()
        && bar.negative_border.is_none()
        && bar.axis == BarAxis::None
        && bar.direction == BarDirection::Context
        && bar.min != Cfvo::Automatic
        && bar.max != Cfvo::Automatic
}

/// Whether the rule can be written at all: the numbers of thresholds and
/// colours are what the file format (and Excel) insist on.
fn is_sound(cf: &CondFormat) -> bool {
    if cf.ranges.is_empty() {
        return false;
    }
    match &cf.rule {
        Rule::ColorScale(scale) => matches!(scale.stops.len(), 2 | 3),
        Rule::IconSet(icons) => icons.thresholds.len() == icons.set.len(),
        _ => true,
    }
}

fn form(cf: &CondFormat, wb: &Workbook) -> Form {
    let formulas = ooxml_formulas(cf);
    let other_sheet = formulas
        .iter()
        .chain(cf.rule.exprs())
        .any(|expr| names_a_sheet(expr, wb));
    match &cf.rule {
        _ if other_sheet => Form::Extension,
        Rule::DataBar(bar) if !is_2007_bar(bar) => Form::Linked,
        Rule::IconSet(icons) if icons.set.is_x14() || custom_icons(icons).is_some() => {
            Form::Extension
        }
        _ => Form::Plain,
    }
}

/// The icons picked one by one, when there is one per band.
fn custom_icons(icons: &IconRule) -> Option<&[Option<(IconSet, u8)>]> {
    icons
        .custom
        .as_deref()
        .filter(|custom| custom.len() == icons.set.len())
}

/// The id tying the two halves of a rule, and naming a rule of the
/// extension list: any GUID does, as long as it is one.
fn rule_id(sheet: usize, priority: usize) -> String {
    format!("{{0C0FD1F0-0000-4000-{:04X}-{:012X}}}", sheet + 1, priority)
}

/// How thresholds and formulas are spelled: `val` attributes and
/// `<formula>` in a 2007 rule, `<xm:f>` children in an x14 one.
struct Spelling<'a> {
    x14: bool,
    wb: &'a Workbook,
}

impl Spelling<'_> {
    fn prefix(&self) -> &'static str {
        if self.x14 { "x14:" } else { "" }
    }

    fn push_formula(&self, x: &mut String, expr: &Expr) {
        let (open, close) = if self.x14 {
            ("<xm:f>", "</xm:f>")
        } else {
            ("<formula>", "</formula>")
        };
        x.push_str(open);
        push_text(x, &to_file_text(expr, self.wb));
        x.push_str(close);
    }

    /// `low_end` and `bar` say what an automatic threshold is called:
    /// `autoMin` / `autoMax` exist for the data bars of x14 only.
    fn push_cfvo(&self, x: &mut String, cfvo: &Cfvo, low_end: bool, bar: bool, gte: bool) {
        let p = self.prefix();
        let kind = match cfvo {
            Cfvo::Automatic if !(self.x14 && bar) => {
                if low_end {
                    "min"
                } else {
                    "max"
                }
            }
            other => other.ooxml_type(low_end),
        };
        w!(x, "<{p}cfvo type=\"{kind}\"");
        // The value of a 2007 threshold is an attribute, before `gte`.
        if let (Some(value), false) = (cfvo.value(), self.x14) {
            x.push_str(" val=\"");
            push_xstring_attr(x, &to_file_text(value, self.wb));
            x.push('"');
        }
        if !gte {
            x.push_str(" gte=\"0\"");
        }
        match cfvo.value().filter(|_| self.x14) {
            Some(value) => {
                x.push('>');
                self.push_formula(x, value);
                x.push_str("</x14:cfvo>");
            }
            None => x.push_str("/>"),
        }
    }

    fn push_color_scale(&self, x: &mut String, scale: &ColorScale) {
        let p = self.prefix();
        w!(x, "<{p}colorScale>");
        for (i, stop) in scale.stops.iter().enumerate() {
            self.push_cfvo(x, &stop.at, i == 0, false, true);
        }
        for stop in &scale.stops {
            push_color(x, &format!("{p}color"), stop.color);
        }
        w!(x, "</{p}colorScale>");
    }

    fn push_icon_set(&self, x: &mut String, icons: &IconRule) {
        let p = self.prefix();
        w!(x, "<{p}iconSet");
        if icons.set != IconSet::ThreeTrafficLights1 {
            w!(x, " iconSet=\"{}\"", icons.set.ooxml());
        }
        if !icons.show_value {
            x.push_str(" showValue=\"0\"");
        }
        if icons.reverse {
            x.push_str(" reverse=\"1\"");
        }
        let custom = custom_icons(icons).filter(|_| self.x14);
        if custom.is_some() {
            x.push_str(" custom=\"1\"");
        }
        x.push('>');
        for (i, threshold) in icons.thresholds.iter().enumerate() {
            self.push_cfvo(x, &threshold.value, i == 0, false, threshold.gte);
        }
        for icon in custom.into_iter().flatten() {
            match icon {
                Some((set, index)) => {
                    let index = (*index as usize).min(set.len() - 1);
                    w!(
                        x,
                        "<x14:cfIcon iconSet=\"{}\" iconId=\"{index}\"/>",
                        set.ooxml()
                    );
                }
                None => x.push_str("<x14:cfIcon iconSet=\"NoIcons\" iconId=\"0\"/>"),
            }
        }
        w!(x, "</{p}iconSet>");
    }

    /// The 2007 half of a data bar: its two ends and its colour.
    fn push_bar_2007(&self, x: &mut String, bar: &DataBar, linked: bool) {
        x.push_str("<dataBar");
        // The lengths of the 2007 form are its own (10 and 90) once the
        // real ones are in the x14 half.
        if !linked && (bar.min_length, bar.max_length) != (10, 90) {
            w!(
                x,
                " minLength=\"{}\" maxLength=\"{}\"",
                bar.min_length.min(100),
                bar.max_length.min(100)
            );
        }
        if !bar.show_value {
            x.push_str(" showValue=\"0\"");
        }
        x.push('>');
        self.push_cfvo(x, &bar.min, true, true, true);
        self.push_cfvo(x, &bar.max, false, true, true);
        push_color(x, "color", bar.fill);
        x.push_str("</dataBar>");
    }

    /// The x14 half of a data bar. `fill` is written when the rule has no
    /// 2007 half to say it.
    fn push_bar_x14(&self, x: &mut String, bar: &DataBar, fill: bool) {
        w!(
            x,
            "<x14:dataBar minLength=\"{}\" maxLength=\"{}\"",
            bar.min_length.min(100),
            bar.max_length.min(100)
        );
        if !bar.show_value {
            x.push_str(" showValue=\"0\"");
        }
        if bar.border.is_some() {
            x.push_str(" border=\"1\"");
        }
        if !bar.gradient {
            x.push_str(" gradient=\"0\"");
        }
        if bar.direction != BarDirection::Context {
            w!(x, " direction=\"{}\"", bar.direction.ooxml());
        }
        if bar.negative_fill.is_none() {
            x.push_str(" negativeBarColorSameAsPositive=\"1\"");
        }
        let negative_border = bar.border.and(bar.negative_border);
        if negative_border.is_some() {
            x.push_str(" negativeBarBorderColorSameAsPositive=\"0\"");
        }
        if bar.axis != BarAxis::Automatic {
            w!(x, " axisPosition=\"{}\"", bar.axis.ooxml());
        }
        x.push('>');
        self.push_cfvo(x, &bar.min, true, true, true);
        self.push_cfvo(x, &bar.max, false, true, true);
        if fill {
            push_color(x, "x14:fillColor", bar.fill);
        }
        if let Some(color) = bar.border {
            push_color(x, "x14:borderColor", color);
        }
        if let Some(color) = bar.negative_fill {
            push_color(x, "x14:negativeFillColor", color);
        }
        if let Some(color) = negative_border {
            push_color(x, "x14:negativeBorderColor", color);
        }
        if bar.axis != BarAxis::None {
            push_color(x, "x14:axisColor", bar.axis_color);
        }
        x.push_str("</x14:dataBar>");
    }

    /// The attributes of a rule that say what it tests, in the order of
    /// the schema (after `type`, `dxfId`, `priority` and `stopIfTrue`).
    fn push_test_attributes(&self, x: &mut String, rule: &Rule) {
        match rule {
            Rule::CellIs { op, .. } => {
                w!(x, " operator=\"{}\"", op.ooxml());
            }
            Rule::TopBottom {
                rank,
                percent,
                bottom,
            } => {
                if *percent {
                    x.push_str(" percent=\"1\"");
                }
                if *bottom {
                    x.push_str(" bottom=\"1\"");
                }
                let most = if *percent { 100 } else { 1000 };
                w!(x, " rank=\"{}\"", (*rank).clamp(1, most));
            }
            Rule::Average {
                above,
                or_equal,
                std_dev,
            } => {
                if !*above {
                    x.push_str(" aboveAverage=\"0\"");
                }
                if *std_dev > 0 {
                    w!(x, " stdDev=\"{}\"", (*std_dev).min(3));
                }
                if *or_equal {
                    x.push_str(" equalAverage=\"1\"");
                }
            }
            Rule::Text { op, text } => {
                w!(x, " operator=\"{}\"", op.ooxml_operator());
                // The attribute holds a text typed as such; a text taken
                // from a cell is in the formula only.
                if let Expr::Text(text) = text {
                    x.push_str(" text=\"");
                    push_xstring_attr(x, text);
                    x.push('"');
                }
            }
            Rule::DateOccurring(period) => {
                w!(x, " timePeriod=\"{}\"", period.ooxml());
            }
            _ => {}
        }
    }
}

/// One rule as a 2007 `<cfRule>`.
fn push_rule_2007(
    x: &mut String,
    cf: &CondFormat,
    priority: usize,
    linked_id: Option<&str>,
    wb: &Workbook,
    dxfs: &mut Dxfs,
) {
    let spell = Spelling { x14: false, wb };
    w!(x, "<cfRule type=\"{}\"", cf.rule.ooxml_type());
    let graded = cf.rule.is_graded();
    if !graded && !cf.format.is_empty() {
        w!(x, " dxfId=\"{}\"", dxfs.id(&cf.format));
    }
    w!(x, " priority=\"{priority}\"");
    if cf.stop_if_true && !graded {
        x.push_str(" stopIfTrue=\"1\"");
    }
    spell.push_test_attributes(x, &cf.rule);
    let formulas = ooxml_formulas(cf);
    if formulas.is_empty() && !graded {
        x.push_str("/>");
        return;
    }
    x.push('>');
    for formula in &formulas {
        spell.push_formula(x, formula);
    }
    match &cf.rule {
        Rule::ColorScale(scale) => spell.push_color_scale(x, scale),
        Rule::DataBar(bar) => spell.push_bar_2007(x, bar, linked_id.is_some()),
        Rule::IconSet(icons) => spell.push_icon_set(x, icons),
        _ => {}
    }
    if let Some(id) = linked_id {
        w!(
            x,
            "<extLst><ext xmlns:x14=\"{NS_X14}\" uri=\"{ID_EXT_URI}\"><x14:id>{id}</x14:id></ext></extLst>"
        );
    }
    x.push_str("</cfRule>");
}

/// One rule as an `<x14:cfRule>`: the x14 half of a data bar (`linked`),
/// or a rule that is stored nowhere else.
fn push_rule_x14(
    x: &mut String,
    cf: &CondFormat,
    priority: usize,
    id: &str,
    linked: bool,
    wb: &Workbook,
    dxfs: &mut Dxfs,
) {
    let spell = Spelling { x14: true, wb };
    w!(x, "<x14:cfRule type=\"{}\"", cf.rule.ooxml_type());
    let graded = cf.rule.is_graded();
    if !linked {
        w!(x, " priority=\"{priority}\"");
        if cf.stop_if_true && !graded {
            x.push_str(" stopIfTrue=\"1\"");
        }
        spell.push_test_attributes(x, &cf.rule);
    }
    w!(x, " id=\"{id}\">");
    for formula in &ooxml_formulas(cf) {
        spell.push_formula(x, formula);
    }
    match &cf.rule {
        Rule::ColorScale(scale) => spell.push_color_scale(x, scale),
        Rule::DataBar(bar) => spell.push_bar_x14(x, bar, !linked),
        Rule::IconSet(icons) => spell.push_icon_set(x, icons),
        _ => {}
    }
    if !graded && !cf.format.is_empty() {
        dxfs.push_dxf(x, "x14:dxf", &cf.format);
    }
    x.push_str("</x14:cfRule>");
}

fn push_sqref(x: &mut String, ranges: &[Rect]) {
    for (i, range) in ranges.iter().enumerate() {
        if i > 0 {
            x.push(' ');
        }
        push_range(x, *range);
    }
}

/// Rules gathered by the cells they apply to, as Excel stores them: one
/// element per distinct list of ranges, in the order the lists first come
/// up.
#[derive(Default)]
struct Groups<'a> {
    index: HashMap<&'a [Rect], usize>,
    list: Vec<(&'a [Rect], String)>,
}

impl<'a> Groups<'a> {
    fn rules_of(&mut self, ranges: &'a [Rect]) -> &mut String {
        let next = self.list.len();
        let at = *self.index.entry(ranges).or_insert(next);
        if at == next {
            self.list.push((ranges, String::new()));
        }
        &mut self.list[at].1
    }
}

/// Append the `<conditionalFormatting>` elements of a sheet: the rules,
/// and the 2007 halves of rules, that Excel 2007 could read.
pub(crate) fn write(x: &mut String, sheet: &Sheet, index: usize, wb: &Workbook, dxfs: &mut Dxfs) {
    let mut groups = Groups::default();
    for (i, cf) in sheet.cond_formats.iter().enumerate() {
        if !is_sound(cf) {
            continue;
        }
        let priority = i + 1;
        let id = match form(cf, wb) {
            Form::Plain => None,
            Form::Linked => Some(rule_id(index, priority)),
            Form::Extension => continue,
        };
        let rules = groups.rules_of(&cf.ranges);
        push_rule_2007(rules, cf, priority, id.as_deref(), wb, dxfs);
    }
    for (ranges, rules) in &groups.list {
        x.push_str("<conditionalFormatting sqref=\"");
        push_sqref(x, ranges);
        w!(x, "\">{rules}</conditionalFormatting>");
    }
}

/// Append the `<ext>` element (a child of the worksheet's `<extLst>`)
/// carrying the x14 rules of a sheet, when there are any.
pub(crate) fn write_ext(
    x: &mut String,
    sheet: &Sheet,
    index: usize,
    wb: &Workbook,
    dxfs: &mut Dxfs,
) {
    let mut groups = Groups::default();
    for (i, cf) in sheet.cond_formats.iter().enumerate() {
        if !is_sound(cf) {
            continue;
        }
        let priority = i + 1;
        let linked = match form(cf, wb) {
            Form::Plain => continue,
            Form::Linked => true,
            Form::Extension => false,
        };
        let rules = groups.rules_of(&cf.ranges);
        let id = rule_id(index, priority);
        push_rule_x14(rules, cf, priority, &id, linked, wb, dxfs);
    }
    if groups.list.is_empty() {
        return;
    }
    w!(
        x,
        "<ext xmlns:x14=\"{NS_X14}\" uri=\"{EXT_URI}\"><x14:conditionalFormattings>"
    );
    for (ranges, rules) in &groups.list {
        w!(
            x,
            "<x14:conditionalFormatting xmlns:xm=\"{NS_XM}\">{rules}<xm:sqref>"
        );
        push_sqref(x, ranges);
        x.push_str("</xm:sqref></x14:conditionalFormatting>");
    }
    x.push_str("</x14:conditionalFormattings></ext>");
}

// ---- reading -------------------------------------------------------------------

/// What reading a rule takes from the rest of the file.
pub(crate) struct Context<'a> {
    pub palette: &'a Palette,
    /// The differential formats of `styles.xml`, by `dxfId`.
    pub dxfs: &'a [StylePatch],
    pub locale: &'a Locale,
}

/// One `<cfvo>` as read: its threshold, and whether reaching it is enough.
struct Threshold {
    value: Option<Cfvo>,
    gte: bool,
}

/// A `<cfRule>` (either form) as read, before it is known whether it
/// stands alone or is half of a rule.
struct Parsed {
    /// `None` for a rule that is damaged or of a kind the engine has not.
    rule: Option<Rule>,
    format: StylePatch,
    stop_if_true: bool,
    priority: Option<u32>,
    /// What ties the two halves of a rule: the `x14:id` inside a 2007
    /// rule, the `id` attribute of an x14 one.
    id: Option<String>,
    /// An x14 data bar that leaves the fill to its 2007 half.
    fill_missing: bool,
}

/// The children of an element, each handed to `child` (which consumes it).
fn each_child<'a>(
    r: &mut Reader<'a>,
    tag: &Tag<'a>,
    mut child: impl FnMut(&mut Reader<'a>, &Tag<'a>),
) {
    if tag.empty {
        return;
    }
    loop {
        match r.next() {
            Event::Open(t) => child(r, &t),
            Event::Close(_) | Event::Eof => break,
            _ => {}
        }
    }
}

/// A threshold: its value is the `val` attribute, or the `<xm:f>` child.
fn read_cfvo<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Threshold {
    let kind = tag.attr("type").unwrap_or_default();
    let gte = tag.attr_bool("gte").unwrap_or(true);
    let mut text = tag.attr("val").map(|v| v.into_owned());
    each_child(r, tag, |r, t| {
        if t.name == "f" {
            text = Some(r.text(t).into_owned());
        } else {
            r.skip(t);
        }
    });
    // A plain number is a number, sign included (as a formula `-5` would
    // be a negation).
    let value = text.filter(|t| !t.trim().is_empty()).and_then(|t| {
        match t.trim().parse::<f64>() {
            Ok(n) if n.is_finite() => Some(Expr::Number(n)),
            // Too large to be a number at all.
            Ok(_) => None,
            Err(_) => from_file_text(&t),
        }
    });
    Threshold {
        value: Cfvo::from_ooxml(&kind, value),
        gte,
    }
}

fn read_color(tag: &Tag, palette: &Palette) -> Option<Color> {
    RawColor::from_tag(tag).resolve(palette)
}

fn read_color_scale<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, ctx: &Context) -> Option<Rule> {
    let mut thresholds = Vec::new();
    let mut colors = Vec::new();
    each_child(r, tag, |r, t| match t.name {
        "cfvo" => thresholds.push(read_cfvo(r, t)),
        "color" => {
            colors.push(read_color(t, ctx.palette));
            r.skip(t);
        }
        _ => r.skip(t),
    });
    if !matches!(thresholds.len(), 2 | 3) || thresholds.len() != colors.len() {
        return None;
    }
    let stops = thresholds
        .into_iter()
        .zip(colors)
        .map(|(threshold, color)| {
            Some(ColorStop {
                // Only data bars have automatic ends.
                at: threshold.value.filter(|at| *at != Cfvo::Automatic)?,
                color: color?,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Rule::ColorScale(ColorScale { stops }))
}

/// A data bar of either form, and whether it left its fill unsaid.
fn read_data_bar<'a>(
    r: &mut Reader<'a>,
    tag: &Tag<'a>,
    ctx: &Context,
    x14: bool,
) -> Option<(DataBar, bool)> {
    let length = |name: &str, default: u8| tag.attr_u32(name).map_or(default, |n| n.min(100) as u8);
    let mut bar = DataBar::legacy(Color::BLACK);
    bar.min_length = length("minLength", 10);
    bar.max_length = length("maxLength", 90);
    bar.show_value = tag.attr_bool("showValue").unwrap_or(true);
    let mut same_negative_fill = true;
    let mut same_negative_border = true;
    let mut border = false;
    if x14 {
        // What the attributes of the x14 form mean when they are absent.
        bar.gradient = tag.attr_bool("gradient").unwrap_or(true);
        border = tag.attr_bool("border").unwrap_or(false);
        bar.direction = tag
            .attr("direction")
            .and_then(|d| BarDirection::from_ooxml(&d))
            .unwrap_or_default();
        bar.axis = tag
            .attr("axisPosition")
            .and_then(|a| BarAxis::from_ooxml(&a))
            .unwrap_or_default();
        same_negative_fill = tag
            .attr_bool("negativeBarColorSameAsPositive")
            .unwrap_or(false);
        same_negative_border = tag
            .attr_bool("negativeBarBorderColorSameAsPositive")
            .unwrap_or(true);
    }
    let mut ends = Vec::new();
    let mut fill = None;
    let mut border_color = None;
    let mut negative_fill = None;
    let mut negative_border = None;
    each_child(r, tag, |r, t| {
        match t.name {
            "cfvo" => {
                ends.push(read_cfvo(r, t));
                return;
            }
            "color" | "fillColor" => fill = read_color(t, ctx.palette),
            "borderColor" => border_color = read_color(t, ctx.palette),
            "negativeFillColor" => negative_fill = read_color(t, ctx.palette),
            "negativeBorderColor" => negative_border = read_color(t, ctx.palette),
            "axisColor" => bar.axis_color = read_color(t, ctx.palette).unwrap_or(Color::BLACK),
            _ => {}
        }
        r.skip(t);
    });
    let [min, max]: [Threshold; 2] = ends.try_into().ok()?;
    bar.min = min.value?;
    bar.max = max.value?;
    let fill_missing = fill.is_none();
    if !x14 && fill_missing {
        return None;
    }
    bar.fill = fill.unwrap_or(Color::BLACK);
    if border {
        bar.border = Some(border_color.unwrap_or(bar.fill));
        if !same_negative_border {
            bar.negative_border = Some(negative_border.unwrap_or(NEGATIVE_RED));
        }
    }
    if x14 && !same_negative_fill {
        bar.negative_fill = Some(negative_fill.unwrap_or(NEGATIVE_RED));
    }
    Some((bar, fill_missing))
}

fn read_icon_set<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Option<Rule> {
    let set = match tag.attr("iconSet") {
        Some(name) => IconSet::from_ooxml(&name),
        None => Some(IconSet::ThreeTrafficLights1),
    };
    let custom = tag.attr_bool("custom").unwrap_or(false);
    let mut thresholds = Vec::new();
    let mut icons: Vec<Option<(IconSet, u8)>> = Vec::new();
    // An icon that does not exist spoils the choice of icons, not the
    // rule: it then shows the icons of its set.
    let mut sound = true;
    each_child(r, tag, |r, t| {
        match t.name {
            "cfvo" => {
                thresholds.push(read_cfvo(r, t));
                return;
            }
            "cfIcon" => {
                let name = t.attr("iconSet").unwrap_or_default();
                let index = t.attr_u32("iconId").unwrap_or(0);
                match IconSet::from_ooxml(&name) {
                    Some(set) if (index as usize) < set.len() => {
                        icons.push(Some((set, index as u8)))
                    }
                    None if name == "NoIcons" => icons.push(None),
                    _ => sound = false,
                }
            }
            _ => {}
        }
        r.skip(t);
    });
    // (Only now that the element is read to its end may the rule be given
    // up.)
    let set = set?;
    if thresholds.len() != set.len() {
        return None;
    }
    let thresholds = thresholds
        .into_iter()
        .map(|threshold| {
            Some(IconThreshold {
                value: threshold.value.filter(|v| *v != Cfvo::Automatic)?,
                gte: threshold.gte,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(Rule::IconSet(IconRule {
        set,
        thresholds,
        reverse: tag.attr_bool("reverse").unwrap_or(false),
        show_value: tag.attr_bool("showValue").unwrap_or(true),
        // Custom icons that do not come one per band are no custom icons.
        custom: (custom && sound && icons.len() == set.len()).then_some(icons),
    }))
}

/// The id inside the `<extLst>` of a 2007 rule.
fn read_linked_id<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Option<String> {
    let mut id = None;
    each_child(r, tag, |r, ext| {
        each_child(r, ext, |r, t| {
            if t.name == "id" {
                id = Some(r.text(t).trim().to_string());
            } else {
                r.skip(t);
            }
        });
    });
    id.filter(|id| !id.is_empty())
}

/// Read a `<cfRule>` of either form, to its end tag.
fn read_rule<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, ctx: &Context, x14: bool) -> Parsed {
    let kind = tag.attr("type").unwrap_or_default();
    let mut parsed = Parsed {
        rule: None,
        format: tag
            .attr_u32("dxfId")
            .and_then(|id| ctx.dxfs.get(id as usize))
            .cloned()
            .unwrap_or_default(),
        stop_if_true: tag.attr_bool("stopIfTrue").unwrap_or(false),
        priority: tag.attr_u32("priority").filter(|p| *p > 0),
        id: tag
            .attr("id")
            .map(|id| id.trim().to_string())
            .filter(|_| x14),
        fill_missing: false,
    };
    let mut formulas: Vec<Option<Expr>> = Vec::new();
    let mut graded: Option<Rule> = None;
    each_child(r, tag, |r, t| match t.name {
        "formula" | "f" => formulas.push(from_file_text(&r.text(t))),
        "colorScale" => graded = read_color_scale(r, t, ctx),
        "dataBar" => {
            graded = read_data_bar(r, t, ctx, x14).map(|(bar, fill_missing)| {
                parsed.fill_missing = fill_missing;
                Rule::DataBar(bar)
            })
        }
        "iconSet" => graded = read_icon_set(r, t),
        "dxf" if x14 => parsed.format = read_dxf(r, t, ctx.palette, ctx.locale),
        "extLst" if !x14 => parsed.id = read_linked_id(r, t),
        _ => r.skip(t),
    });
    let mut formulas = formulas.into_iter();
    // A formula that is there but cannot be read (a link to another
    // workbook) spoils the rule; one that is not there may not be needed.
    let first = formulas.next();
    let second = formulas.next();
    let unreadable = matches!(first, Some(None)) || matches!(second, Some(None));
    let first = first.flatten();
    let second = second.flatten();
    let text_rule = |op: TextOp| {
        let text = tag.attr("text").map(|t| decode_cell_text(&t).into_owned());
        match text_operand_from_ooxml(op, text.as_deref(), first.as_ref()) {
            Some(text) => Some(Rule::Text { op, text }),
            // Not the formula Excel writes for this test: the formula is
            // what Excel would apply.
            None => first.clone().map(Rule::Formula),
        }
    };
    parsed.rule = match kind.as_ref() {
        _ if unreadable => None,
        "cellIs" => tag
            .attr("operator")
            .and_then(|op| CellOp::from_ooxml(&op))
            .and_then(|op| {
                Some(Rule::CellIs {
                    op,
                    first: first.clone()?,
                    second: if op.takes_two() {
                        Some(second.clone()?)
                    } else {
                        None
                    },
                })
            }),
        "expression" => first.clone().map(Rule::Formula),
        "top10" => Some(Rule::TopBottom {
            rank: tag.attr_u32("rank").unwrap_or(10).clamp(1, 1000),
            percent: tag.attr_bool("percent").unwrap_or(false),
            bottom: tag.attr_bool("bottom").unwrap_or(false),
        }),
        "aboveAverage" => Some(Rule::Average {
            above: tag.attr_bool("aboveAverage").unwrap_or(true),
            or_equal: tag.attr_bool("equalAverage").unwrap_or(false),
            std_dev: tag.attr_u32("stdDev").unwrap_or(0).min(3),
        }),
        "duplicateValues" => Some(Rule::DuplicateValues),
        "uniqueValues" => Some(Rule::UniqueValues),
        "containsText" => text_rule(TextOp::Contains),
        "notContainsText" => text_rule(TextOp::NotContains),
        "beginsWith" => text_rule(TextOp::BeginsWith),
        "endsWith" => text_rule(TextOp::EndsWith),
        "containsBlanks" => Some(Rule::Blanks),
        "notContainsBlanks" => Some(Rule::NoBlanks),
        "containsErrors" => Some(Rule::Errors),
        "notContainsErrors" => Some(Rule::NoErrors),
        "timePeriod" => tag
            .attr("timePeriod")
            .and_then(|period| TimePeriod::from_ooxml(&period))
            .map(Rule::DateOccurring),
        "colorScale" => graded.take().filter(|g| matches!(g, Rule::ColorScale(_))),
        "dataBar" => graded.take().filter(|g| matches!(g, Rule::DataBar(_))),
        "iconSet" => graded.take().filter(|g| matches!(g, Rule::IconSet(_))),
        _ => None,
    };
    parsed
}

/// A rule read from a 2007 element, or from the extension list with a
/// priority of its own.
struct Placed {
    ranges: Vec<Rect>,
    parsed: Parsed,
}

/// The conditional formats of a worksheet while it is read: the 2007
/// rules and those of the extension list come at different places of the
/// part, and only together do they give the rules of the sheet.
#[derive(Default)]
pub(crate) struct CondFormats {
    rules: Vec<Placed>,
    /// The x14 rules, which may be the second half of a 2007 rule.
    extension: Vec<Placed>,
}

impl CondFormats {
    /// Read a `<conditionalFormatting>` element of the worksheet.
    pub fn read<'a>(&mut self, r: &mut Reader<'a>, tag: &Tag<'a>, ctx: &Context) {
        let ranges = parse_sqref(&tag.attr("sqref").unwrap_or_default());
        each_child(r, tag, |r, t| {
            if t.name != "cfRule" {
                r.skip(t);
                return;
            }
            let parsed = read_rule(r, t, ctx, false);
            self.rules.push(Placed {
                ranges: ranges.clone(),
                parsed,
            });
        });
    }

    /// Read the `<x14:conditionalFormattings>` of the extension list.
    pub fn read_ext<'a>(&mut self, r: &mut Reader<'a>, tag: &Tag<'a>, ctx: &Context) {
        each_child(r, tag, |r, group| {
            if group.name != "conditionalFormatting" {
                r.skip(group);
                return;
            }
            // The cells come after the rules they are for.
            let mut rules = Vec::new();
            let mut ranges = Vec::new();
            each_child(r, group, |r, t| match t.name {
                "cfRule" => rules.push(read_rule(r, t, ctx, true)),
                "sqref" => ranges = parse_sqref(&r.text(t)),
                _ => r.skip(t),
            });
            self.extension
                .extend(rules.into_iter().map(|parsed| Placed {
                    ranges: ranges.clone(),
                    parsed,
                }));
        });
    }

    /// Give the sheet its rules, in priority order. A rule that could not
    /// be read is left out; the others are not affected.
    pub fn finish(mut self, sheet: &mut Sheet) {
        for x14 in std::mem::take(&mut self.extension) {
            let half = x14.parsed.id.as_ref().and_then(|id| {
                self.rules
                    .iter_mut()
                    .find(|base| base.parsed.id.as_ref() == Some(id))
            });
            match half {
                // The x14 half of a data bar says everything again but,
                // often, the fill.
                Some(base) => {
                    if let (Some(Rule::DataBar(old)), Some(Rule::DataBar(mut new))) =
                        (&base.parsed.rule, x14.parsed.rule)
                    {
                        if x14.parsed.fill_missing {
                            new.fill = old.fill;
                        }
                        base.parsed.rule = Some(Rule::DataBar(new));
                    }
                }
                // A rule of the extension list that stands alone says
                // where it comes among the others; one that does not is a
                // half whose other half is missing.
                None if x14.parsed.priority.is_some() => self.rules.push(x14),
                None => {}
            }
        }
        let mut rules: Vec<(u32, CondFormat)> = self
            .rules
            .into_iter()
            .filter(|placed| !placed.ranges.is_empty())
            .filter_map(|placed| {
                let parsed = placed.parsed;
                let rule = parsed.rule?;
                // A rule without a priority comes after those that have one.
                let priority = parsed.priority.unwrap_or(u32::MAX);
                let graded = rule.is_graded();
                let cf = CondFormat {
                    ranges: placed.ranges,
                    format: if graded {
                        StylePatch::default()
                    } else {
                        parsed.format
                    },
                    stop_if_true: parsed.stop_if_true && !graded,
                    rule,
                };
                Some((priority, cf))
            })
            .collect();
        rules.sort_by_key(|(priority, _)| *priority);
        sheet
            .cond_formats
            .extend(rules.into_iter().map(|(_, cf)| cf));
    }
}
