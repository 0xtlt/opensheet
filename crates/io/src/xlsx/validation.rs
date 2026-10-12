//! Data validation: the `<dataValidations>` of a worksheet.
//!
//! A rule is stored in one of two places. Excel 2007 could not refer to
//! another sheet in a rule; when later versions allowed it, the rules that
//! do went to the extension list of the worksheet, where Excel 2007 does
//! not look:
//!
//! ```xml
//! <dataValidations count="1">
//!   <dataValidation type="list" allowBlank="1" sqref="A1:A9">
//!     <formula1>$D$1:$D$3</formula1>
//!   </dataValidation>
//! </dataValidations>
//! ...
//! <extLst><ext uri="{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}">
//!   <x14:dataValidations count="1">
//!     <x14:dataValidation type="list" allowBlank="1">
//!       <x14:formula1><xm:f>Lists!$A$1:$A$3</xm:f></x14:formula1>
//!       <xm:sqref>B1:B9</xm:sqref>
//!     </x14:dataValidation>
//!   </x14:dataValidations>
//! </ext></extLst>
//! ```
//!
//! The reader takes both forms from wherever they are; the writer puts
//! each rule where Excel would.

use super::formula::{from_file_text, to_file_text};
use super::write::push_range;
use super::xml::{Event, Reader, Tag, decode_cell_text, push_text, push_xstring_attr};
use opensheet_engine::ast::Expr;
use opensheet_engine::refs::Rect;
use opensheet_engine::validation::{AlertStyle, ImeMode, Operator, Validation, ValidationKind};
use opensheet_engine::{Sheet, Workbook};
use std::collections::HashSet;
use std::fmt::Write as _;
use std::sync::Arc;

/// Longest title, message or typed list Excel takes from a file. Checked
/// with Excel 16 for Mac: 255 characters open, with 256 the file is
/// reported as damaged. (Its dialog stops titles at 32 characters, but
/// longer ones in a file are read and shown.)
const MAX_TEXT_LEN: usize = 255;

const EXT_URI: &str = "{CCE6A557-97BC-4b89-ADB6-D9C93CAAB3DF}";
const NS_X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
const NS_XM: &str = "http://schemas.microsoft.com/office/excel/2006/main";

// ---- reading -------------------------------------------------------------------

/// The ranges of a `sqref`: references separated by spaces. A reference
/// that makes no sense is left out, and so is one that was already given.
fn parse_sqref(text: &str) -> Vec<Rect> {
    let mut seen = HashSet::new();
    text.split_whitespace()
        .filter_map(Rect::parse)
        .filter(|range| seen.insert(*range))
        .collect()
}

/// One `<dataValidation>` (either form), consumed to its end tag. `None`
/// when the rule is damaged or means nothing the engine can check.
fn read_rule<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Option<Validation> {
    let mut rule = Validation::file_default();
    let mut sound = true;
    match tag.attr("type") {
        None => {}
        Some(name) => match ValidationKind::from_ooxml_name(&name) {
            Some(kind) => rule.kind = kind,
            None => sound = false,
        },
    }
    match tag.attr("operator") {
        None => {}
        Some(name) => match Operator::from_ooxml_name(&name) {
            Some(operator) => rule.operator = operator,
            None => sound = false,
        },
    }
    // What the alert looks like is not worth a rule: an unknown style is
    // the default one.
    rule.error_style = tag
        .attr("errorStyle")
        .and_then(|name| AlertStyle::from_ooxml_name(&name))
        .unwrap_or_default();
    rule.ime_mode = tag
        .attr("imeMode")
        .and_then(|name| ImeMode::from_ooxml_name(&name))
        .unwrap_or_default();
    rule.allow_blank = tag.attr_bool("allowBlank").unwrap_or(false);
    // The attribute hides the arrow, whatever its name says.
    rule.in_cell_dropdown = !tag.attr_bool("showDropDown").unwrap_or(false);
    rule.show_input = tag.attr_bool("showInputMessage").unwrap_or(false);
    rule.show_error = tag.attr_bool("showErrorMessage").unwrap_or(false);
    let text = |name: &str| {
        tag.attr(name)
            .map(|v| decode_cell_text(&v).into_owned())
            .unwrap_or_default()
    };
    rule.error_title = text("errorTitle");
    rule.error_message = text("error");
    rule.input_title = text("promptTitle");
    rule.input_message = text("prompt");
    let mut sqref = tag.attr("sqref").map(|v| v.into_owned());

    let mut formulas: [Option<String>; 2] = [None, None];
    if !tag.empty {
        // Newer files wrap the formula of a typed list in an
        // `AlternateContent` whose fallback is the plain `<formula1>`.
        let mut wrappers = 0usize;
        loop {
            match r.next() {
                Event::Open(t) => match t.name {
                    "formula1" => formulas[0] = Some(r.text(&t).into_owned()),
                    "formula2" => formulas[1] = Some(r.text(&t).into_owned()),
                    "sqref" => sqref = Some(r.text(&t).into_owned()),
                    "AlternateContent" | "Fallback" if !t.empty => wrappers += 1,
                    _ => r.skip(&t),
                },
                Event::Close(_) => {
                    if wrappers == 0 {
                        break;
                    }
                    wrappers -= 1;
                }
                Event::Eof => break,
                _ => {}
            }
        }
    }

    rule.ranges = parse_sqref(sqref.as_deref().unwrap_or(""));
    if !sound || rule.ranges.is_empty() {
        return None;
    }
    if rule.kind == ValidationKind::Any {
        return (!rule.is_noop()).then_some(rule);
    }
    // The operator of a list or of a custom formula says nothing.
    if !rule.kind.has_operator() {
        rule.operator = Operator::default();
    }
    let [first, second] = formulas.map(|text| text.filter(|t| !t.trim().is_empty()));
    let two = rule.kind.has_operator() && rule.operator.has_two_operands();
    // A rule whose formula cannot be read (a link to another workbook) or
    // is missing would refuse everything, or nothing: it is dropped.
    rule.formula1 = Some(Arc::new(from_file_text(&first?)?));
    if two {
        rule.formula2 = Some(Arc::new(from_file_text(&second?)?));
    }
    Some(rule)
}

/// Read a `<dataValidations>` element, of the worksheet or of its
/// extension list, into the rules of the sheet.
pub(crate) fn read<'a>(r: &mut Reader<'a>, container: &Tag<'a>, sheet: &mut Sheet) {
    if container.empty {
        return;
    }
    loop {
        match r.next() {
            Event::Open(t) if t.name == "dataValidation" => {
                sheet.validations.extend(read_rule(r, &t));
            }
            Event::Open(t) => r.skip(&t),
            Event::Close(_) | Event::Eof => break,
            _ => {}
        }
    }
}

// ---- writing -------------------------------------------------------------------

/// Whether a formula names a sheet, which the plain form cannot. (A sheet
/// that is gone is not named: what pointed at it is written `#REF!`.)
fn names_a_sheet(expr: &Expr, wb: &Workbook) -> bool {
    let exists = |sheet: &str| wb.sheet_index(sheet).is_some();
    let mut found = false;
    expr.walk(&mut |e| {
        found |= match e {
            Expr::Ref(r) => r.sheet.as_deref().is_some_and(exists),
            Expr::Ref3D(r) => exists(&r.first) && exists(&r.last),
            Expr::SheetName(sheet, _) | Expr::RefError(sheet) => exists(sheet),
            _ => false,
        }
    });
    found
}

/// The formulas of a rule that are written: none where anything goes, the
/// second only between two bounds.
fn formulas(rule: &Validation) -> [Option<&Expr>; 2] {
    if rule.kind == ValidationKind::Any {
        return [None, None];
    }
    let two = rule.kind.has_operator() && rule.operator.has_two_operands();
    [
        rule.formula1.as_deref(),
        rule.formula2.as_deref().filter(|_| two),
    ]
}

fn in_extension(rule: &Validation, wb: &Workbook) -> bool {
    formulas(rule)
        .into_iter()
        .flatten()
        .any(|expr| names_a_sheet(expr, wb))
}

/// `text` cut to `max` characters, counted as Excel counts them: in
/// UTF-16 units.
fn fit(text: &str, max: usize) -> &str {
    let mut units = 0;
    for (i, c) in text.char_indices() {
        units += c.len_utf16();
        if units > max {
            return &text[..i];
        }
    }
    text
}

/// The text of a formula as written. A list typed in the dialog is one
/// text constant; Excel refuses a file where it is too long, so whatever
/// items do not fit are left out. (Formulas themselves may be longer.)
fn formula_text(expr: &Expr, wb: &Workbook) -> String {
    match expr {
        Expr::Text(list) if list.encode_utf16().count() > MAX_TEXT_LEN => {
            let head = fit(list, MAX_TEXT_LEN);
            let whole_items = match head.rfind(',') {
                // The cut fell inside an item: it goes too.
                Some(comma) if !list[head.len()..].starts_with(',') => &head[..comma],
                _ => head,
            };
            Expr::Text(Arc::from(whole_items)).to_file_text()
        }
        other => to_file_text(other, wb),
    }
}

fn push_rule(x: &mut String, rule: &Validation, wb: &Workbook, extension: bool) {
    let p = if extension { "x14:" } else { "" };
    w!(x, "<{p}dataValidation");
    if rule.kind != ValidationKind::Any {
        w!(x, " type=\"{}\"", rule.kind.ooxml_name());
    }
    if rule.error_style != AlertStyle::default() {
        w!(x, " errorStyle=\"{}\"", rule.error_style.ooxml_name());
    }
    if rule.ime_mode != ImeMode::default() {
        w!(x, " imeMode=\"{}\"", rule.ime_mode.ooxml_name());
    }
    if rule.kind.has_operator() && rule.operator != Operator::default() {
        w!(x, " operator=\"{}\"", rule.operator.ooxml_name());
    }
    for (name, on) in [
        ("allowBlank", rule.allow_blank),
        ("showDropDown", !rule.in_cell_dropdown),
        ("showInputMessage", rule.show_input),
        ("showErrorMessage", rule.show_error),
    ] {
        if on {
            w!(x, " {name}=\"1\"");
        }
    }
    for (name, text) in [
        ("errorTitle", &rule.error_title),
        ("error", &rule.error_message),
        ("promptTitle", &rule.input_title),
        ("prompt", &rule.input_message),
    ] {
        if !text.is_empty() {
            w!(x, " {name}=\"");
            push_xstring_attr(x, fit(text, MAX_TEXT_LEN));
            x.push('"');
        }
    }
    let push_sqref = |x: &mut String| {
        for (i, range) in rule.ranges.iter().enumerate() {
            if i > 0 {
                x.push(' ');
            }
            push_range(x, *range);
        }
    };
    if !extension {
        x.push_str(" sqref=\"");
        push_sqref(x);
        x.push('"');
        if formulas(rule) == [None, None] {
            x.push_str("/>");
            return;
        }
    }
    x.push('>');
    for (expr, name) in formulas(rule).into_iter().zip(["formula1", "formula2"]) {
        let Some(expr) = expr else { continue };
        w!(x, "<{p}{name}>");
        if extension {
            x.push_str("<xm:f>");
        }
        push_text(x, &formula_text(expr, wb));
        if extension {
            x.push_str("</xm:f>");
        }
        w!(x, "</{p}{name}>");
    }
    if extension {
        x.push_str("<xm:sqref>");
        push_sqref(x);
        x.push_str("</xm:sqref>");
    }
    w!(x, "</{p}dataValidation>");
}

/// The rules of a sheet that go to the file, in the given form.
fn rules<'a>(sheet: &'a Sheet, wb: &Workbook, extension: bool) -> Vec<&'a Validation> {
    sheet
        .validations
        .iter()
        .filter(|rule| !rule.ranges.is_empty() && !rule.is_noop())
        .filter(|rule| in_extension(rule, wb) == extension)
        .collect()
}

/// Append the `<dataValidations>` of a sheet: the rules that do not refer
/// to another sheet.
pub(crate) fn write(x: &mut String, sheet: &Sheet, wb: &Workbook) {
    let rules = rules(sheet, wb, false);
    if rules.is_empty() {
        return;
    }
    w!(x, "<dataValidations count=\"{}\">", rules.len());
    for rule in rules {
        push_rule(x, rule, wb, false);
    }
    x.push_str("</dataValidations>");
}

/// Append the `<ext>` element (a child of the worksheet's `<extLst>`)
/// carrying the rules that refer to another sheet, when there are any.
pub(crate) fn write_ext(x: &mut String, sheet: &Sheet, wb: &Workbook) {
    let rules = rules(sheet, wb, true);
    if rules.is_empty() {
        return;
    }
    w!(
        x,
        "<ext xmlns:x14=\"{NS_X14}\" uri=\"{EXT_URI}\"><x14:dataValidations xmlns:xm=\"{NS_XM}\" count=\"{}\">",
        rules.len()
    );
    for rule in rules {
        push_rule(x, rule, wb, true);
    }
    x.push_str("</x14:dataValidations></ext>");
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::Locale;

    fn parse(xml: &str) -> Vec<Validation> {
        let mut sheet = Sheet::new("S");
        let mut r = Reader::new(xml);
        let Event::Open(tag) = r.next() else {
            panic!("no element in {xml}");
        };
        read(&mut r, &tag, &mut sheet);
        sheet.validations
    }

    fn text(expr: &Option<Arc<Expr>>) -> String {
        expr.as_ref()
            .map(|e| e.to_text(Locale::en()))
            .unwrap_or_default()
    }

    #[test]
    fn sqref() {
        let names =
            |text: &str| -> Vec<String> { parse_sqref(text).iter().map(Rect::name).collect() };
        assert_eq!(names("A1"), ["A1"]);
        assert_eq!(
            names(" B2:C3  $D$4\tA1:A1048576\nXFD1 "),
            ["B2:C3", "D4", "A:A", "XFD1"]
        );
        assert_eq!(names("A1 nonsense ZZZ9 B2 A1 $A$1"), ["A1", "B2"]);
        assert!(names("").is_empty());
    }

    #[test]
    fn lengths_fit_excel() {
        assert_eq!(fit("abc", 3), "abc");
        assert_eq!(fit("abcd", 3), "abc");
        assert_eq!(fit("é😀é", 3), "é😀", "an emoji counts for two");
        assert_eq!(fit("é😀é", 2), "é");
        let wb = Workbook::new(Locale::en());
        let long: String = (0..100).map(|i| format!("item{i},")).collect();
        let written = formula_text(&Expr::Text(Arc::from(long.trim_end_matches(','))), &wb);
        assert!(written.len() <= MAX_TEXT_LEN + 2, "{}", written.len());
        assert!(written.starts_with("\"item0,item1,") && written.ends_with(",item37\""));
        // One item more would not fit: the cut falls inside it.
        let longer = format!("x{}", long.trim_end_matches(','));
        let written = formula_text(&Expr::Text(Arc::from(longer)), &wb);
        assert!(written.ends_with(",item36\""), "{written}");
        assert_eq!(formula_text(&Expr::Text(Arc::from("a,b")), &wb), "\"a,b\"");
    }

    #[test]
    fn damaged_rules_are_dropped_alone() {
        let rules = parse(concat!(
            "<dataValidations>",
            // Unknown type, unknown operator.
            r#"<dataValidation type="telepathy" sqref="A1"><formula1>1</formula1></dataValidation>"#,
            r#"<dataValidation type="whole" operator="roughly" sqref="A2"><formula1>1</formula1></dataValidation>"#,
            // No cells, or none that exist.
            r#"<dataValidation type="whole" operator="equal"><formula1>1</formula1></dataValidation>"#,
            r#"<dataValidation type="whole" operator="equal" sqref="ZZZZ1 A0"><formula1>1</formula1></dataValidation>"#,
            // Formulas missing, empty, unreadable or pointing outside.
            r#"<dataValidation type="whole" sqref="A3"><formula1>1</formula1></dataValidation>"#,
            r#"<dataValidation type="list" sqref="A4"/>"#,
            r#"<dataValidation type="list" sqref="A5"><formula1> </formula1></dataValidation>"#,
            r#"<dataValidation type="custom" sqref="A6"><formula1>SUM((</formula1></dataValidation>"#,
            r#"<dataValidation type="list" sqref="A7"><formula1>[1]Sheet1!$A$1:$A$3</formula1></dataValidation>"#,
            // Says nothing at all.
            r#"<dataValidation sqref="A8"/>"#,
            r#"<dataValidation type="none" showInputMessage="1" sqref="A9"/>"#,
            // Sound, among the wreckage.
            r#"<dataValidation type="whole" operator="equal" errorStyle="loud" imeMode="telex" sqref="A10 junk"><formula1>1</formula1><formula2>((</formula2><bogus><formula1>2</formula1></bogus></dataValidation>"#,
            r#"<dataValidation promptTitle="Hint" sqref="A11"/>"#,
            // Cut short.
            r#"<dataValidation type="list" sqref="A12"><formula1>"a,b"</formula1>"#,
        ));
        let cells: Vec<String> = rules.iter().map(|r| r.ranges[0].name()).collect();
        assert_eq!(cells, ["A10", "A11", "A12"]);
        assert_eq!(text(&rules[0].formula1), "1");
        assert_eq!(rules[0].formula2, None);
        assert_eq!(rules[0].error_style, AlertStyle::Stop);
        assert_eq!(rules[0].ime_mode, ImeMode::NoControl);
        assert_eq!(rules[1].input_title, "Hint");
        assert_eq!(text(&rules[2].formula1), "\"a,b\"");
    }
}
