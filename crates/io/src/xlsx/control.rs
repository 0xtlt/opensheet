//! Form controls (buttons, check boxes, lists...) in `.xlsx` and `.xlsm`
//! files. Excel stores each control three times over:
//!
//! * a shape of the legacy VML drawing of the sheet (`<legacyDrawing>`,
//!   the part that also holds the boxes of the notes), whose
//!   `<x:ClientData ObjectType="Button">` says what the control is, where
//!   it sits and how it is set — the only form Excel 2007 wrote, and the
//!   one every version reads;
//! * a `ctrlProp` part (`xl/ctrlProps/ctrlPropN.xml`), the same settings
//!   as attributes of a `<formControlPr>`;
//! * a `<control>` element of the worksheet, inside
//!   `<mc:AlternateContent>` (older readers skip it), naming the shape by
//!   its number and the `ctrlProp` part by a relationship, with the
//!   anchor of the control.
//!
//! All three are written here, from the model, the way Excel writes them
//! for a control just inserted. A control that came from a file and was
//! not changed since is written back with the very pieces it was read
//! from (see `ControlSource`), renumbered: what the model does not hold —
//! the font of a caption, colours — goes through untouched.
//!
//! Not written: the hidden twin Excel also puts in the DrawingML part of
//! the sheet (an `xdr:sp` carrying `a14:compatExt`); a twin found in a
//! file is recognised by the drawing reader and left out, so that it does
//! not come back as a rectangle. ActiveX controls (shapes of type `Pict`,
//! `control` relationships) are not read.

use super::parts::{Extras, SheetLinks};
use super::read::{Package, relationships};
use super::write::NS_DOC_REL;
use super::xml::{Event, Reader, XML_DECL, push_attr, push_text};
use opensheet_engine::ast::{Expr, RefExpr};
use opensheet_engine::control::{
    CheckState, ControlKind, ControlSource, FormControl, SelectionType,
};
use opensheet_engine::parser::parse_formula;
use opensheet_engine::{Locale, Sheet, Workbook};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{Read, Seek};
use std::sync::Arc;

const CT_CTRL_PROP: &str = "application/vnd.ms-excel.controlproperties+xml";
const NS_X14: &str = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
const NS_MC: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";
const NS_XDR: &str = "http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing";
/// Shape ids come in blocks of this many; a VML drawing lists its blocks.
const SHAPE_ID_BLOCK: usize = 1024;
const EMU_PER_PX: f64 = 9525.0;

/// `ObjectType` of the VML shape and `objectType` of the `ctrlProp` part.
fn type_names(kind: ControlKind) -> (&'static str, &'static str) {
    match kind {
        ControlKind::Button => ("Button", "Button"),
        ControlKind::CheckBox => ("Checkbox", "CheckBox"),
        ControlKind::OptionButton => ("Radio", "Radio"),
        ControlKind::ComboBox => ("Drop", "Drop"),
        ControlKind::ListBox => ("List", "List"),
        ControlKind::SpinButton => ("Spin", "Spin"),
        ControlKind::ScrollBar => ("Scroll", "Scroll"),
        ControlKind::GroupBox => ("GBox", "GBox"),
        ControlKind::Label => ("Label", "Label"),
    }
}

fn kind_of(object_type: &str) -> Option<ControlKind> {
    ControlKind::ALL
        .into_iter()
        .find(|kind| type_names(*kind).0.eq_ignore_ascii_case(object_type))
}

/// How many blocks of shape ids `shapes` shapes take.
pub(crate) fn blocks(shapes: usize) -> usize {
    shapes / SHAPE_ID_BLOCK + 1
}

/// The shapes of the VML drawing of a sheet: its notes, then its controls.
fn shape_count(sheet: &Sheet) -> usize {
    sheet.comments.len() + sheet.controls.len()
}

/// The number of the first control shape of sheet `index`: the VML
/// drawings take their blocks of ids in the order of the sheets, and in
/// each the notes come first.
fn first_shape_id(wb: &Workbook, index: usize) -> usize {
    let block: usize = 1 + wb.sheets[..index]
        .iter()
        .filter(|sheet| shape_count(sheet) > 0)
        .map(|sheet| blocks(shape_count(sheet)))
        .sum::<usize>();
    block * SHAPE_ID_BLOCK + wb.sheets[index].comments.len() + 1
}

// ---- writing ---------------------------------------------------------------

/// A reference as files write the links of controls: `$A$1`, or
/// `Sheet2!$A$1:$A$9` for cells of another sheet.
fn formula(r: &RefExpr) -> String {
    r.to_text()
}

/// The two corners of a control, as (row, col, x offset, y offset) in
/// whole pixels.
fn corners(sheet: &Sheet, control: &FormControl) -> [(u32, u32, f64, f64); 2] {
    let (x, y, w, h) = sheet.control_frame(control);
    let corner = |x: f64, y: f64| {
        let ((row, col), (dx, dy)) = sheet.corner_at(x.round(), y.round());
        (row, col, (dx as f64).round(), (dy as f64).round())
    };
    [corner(x, y), corner(x + w, y + h)]
}

fn push_points(x: &mut String, px: f64) {
    let pt = (px * 0.75 * 100.0).round() / 100.0;
    let _ = write!(x, "{pt}pt");
}

/// The `<v:shape>` of a control, as Excel writes a new one of its kind.
/// `now` is the control with its state resolved against the cells.
fn shape_xml(sheet: &Sheet, now: &FormControl, id: usize, z: usize) -> String {
    let (left, top, width, height) = sheet.control_frame(now);
    let mut x = String::new();
    w!(
        x,
        "<v:shape id=\"_x0000_s{id}\" type=\"#_x0000_t201\" style=\"position:absolute;margin-left:"
    );
    push_points(&mut x, left.round());
    x.push_str(";margin-top:");
    push_points(&mut x, top.round());
    x.push_str(";width:");
    push_points(&mut x, width.round());
    x.push_str(";height:");
    push_points(&mut x, height.round());
    w!(x, ";z-index:{z};mso-wrap-style:tight\"");
    // What the shape looks like to an application that draws the VML.
    let caption = |x: &mut String, align: &str, font: &str| {
        w!(
            x,
            "<v:textbox style=\"mso-direction-alt:auto\" o:singleclick=\"f\"><div style=\"text-align:{align}\">{font}"
        );
        push_text(x, &now.text);
        x.push_str("</font></div></v:textbox>");
    };
    match now.kind {
        ControlKind::Button => {
            x.push_str(" o:button=\"t\" fillcolor=\"buttonFace [67]\" strokecolor=\"windowText [64]\" o:insetmode=\"auto\"><v:fill color2=\"buttonFace [67]\" o:detectmouseclick=\"t\"/><o:lock v:ext=\"edit\" rotation=\"t\"/>");
            caption(
                &mut x,
                "center",
                "<font face=\"Calibri\" size=\"220\" color=\"#000000\">",
            );
        }
        ControlKind::CheckBox
        | ControlKind::OptionButton
        | ControlKind::GroupBox
        | ControlKind::Label => {
            x.push_str(" filled=\"f\" fillcolor=\"window [65]\" stroked=\"f\" strokecolor=\"windowText [64]\" o:insetmode=\"auto\"><v:path shadowok=\"t\" strokeok=\"t\" fillok=\"t\"/><o:lock v:ext=\"edit\" rotation=\"t\"/>");
            caption(
                &mut x,
                "left",
                "<font face=\"Segoe UI\" size=\"160\" color=\"auto\">",
            );
        }
        ControlKind::ListBox => {
            x.push_str(" fillcolor=\"window [65]\" strokecolor=\"windowText [64]\" o:insetmode=\"auto\"><v:fill color2=\"window [65]\"/><o:lock v:ext=\"edit\" rotation=\"t\" text=\"t\"/>");
        }
        ControlKind::ComboBox | ControlKind::SpinButton | ControlKind::ScrollBar => {
            x.push_str(" stroked=\"f\" strokecolor=\"windowText [64]\" o:insetmode=\"auto\"><o:lock v:ext=\"edit\" rotation=\"t\" text=\"t\"/>");
        }
    }
    w!(
        x,
        "<x:ClientData ObjectType=\"{}\">",
        type_names(now.kind).0
    );
    // Present, these two say that the object does *not* follow its cells.
    if !now.move_with_cells {
        x.push_str("<x:MoveWithCells/>");
    }
    if !now.size_with_cells {
        x.push_str("<x:SizeWithCells/>");
    }
    let [from, to] = corners(sheet, now);
    w!(
        x,
        "<x:Anchor>{}, {}, {}, {}, {}, {}, {}, {}</x:Anchor>",
        from.1,
        from.2,
        from.0,
        from.3,
        to.1,
        to.2,
        to.0,
        to.3
    );
    if !now.locked {
        x.push_str("<x:Locked>False</x:Locked>");
    }
    if !now.print {
        x.push_str("<x:PrintObject>False</x:PrintObject>");
    }
    let captioned = now.kind.has_text();
    if captioned {
        x.push_str("<x:AutoFill>False</x:AutoFill>");
    }
    if now.kind != ControlKind::Button
        && now.kind != ControlKind::SpinButton
        && now.kind != ControlKind::ScrollBar
    {
        x.push_str("<x:AutoLine>False</x:AutoLine>");
    }
    if let Some(name) = &now.macro_name {
        x.push_str("<x:FmlaMacro>");
        push_text(&mut x, name);
        x.push_str("</x:FmlaMacro>");
    }
    match now.kind {
        ControlKind::Button => {
            x.push_str("<x:TextHAlign>Center</x:TextHAlign><x:TextVAlign>Center</x:TextVAlign>")
        }
        ControlKind::CheckBox | ControlKind::OptionButton => {
            x.push_str("<x:TextVAlign>Center</x:TextVAlign>")
        }
        _ => {}
    }
    if captioned && !now.lock_text {
        x.push_str("<x:LockText>False</x:LockText>");
    }
    let reference = |x: &mut String, element: &str, r: &Option<RefExpr>| {
        if let Some(r) = r {
            w!(x, "<x:{element}>");
            push_text(x, &formula(r));
            w!(x, "</x:{element}>");
        }
    };
    match now.kind {
        ControlKind::CheckBox | ControlKind::OptionButton => {
            match now.checked {
                CheckState::Unchecked => {}
                CheckState::Checked => x.push_str("<x:Checked>1</x:Checked>"),
                CheckState::Mixed => x.push_str("<x:Checked>2</x:Checked>"),
            }
            reference(&mut x, "FmlaLink", &now.link);
            if !now.shaded {
                x.push_str("<x:NoThreeD/>");
            }
            if now.kind == ControlKind::OptionButton && now.first_button {
                x.push_str("<x:FirstButton/>");
            }
        }
        ControlKind::ComboBox | ControlKind::ListBox => {
            reference(&mut x, "FmlaRange", &now.input);
            reference(&mut x, "FmlaLink", &now.link);
            // The scroll bar of the list: the lines it can reach.
            w!(
                x,
                "<x:Val>0</x:Val><x:Min>0</x:Min><x:Max>0</x:Max><x:Inc>1</x:Inc><x:Page>{}</x:Page><x:Dx>22</x:Dx>",
                now.drop_lines.max(1)
            );
            w!(x, "<x:Sel>{}</x:Sel>", now.selected);
            if !now.shaded {
                x.push_str("<x:NoThreeD2/>");
            }
            x.push_str(match now.selection {
                SelectionType::Single => "<x:SelType>Single</x:SelType>",
                SelectionType::Multi => "<x:SelType>Multi</x:SelType>",
                SelectionType::Extend => "<x:SelType>Extend</x:SelType>",
            });
            x.push_str("<x:LCT>Normal</x:LCT>");
            if now.kind == ControlKind::ComboBox {
                w!(
                    x,
                    "<x:DropStyle>Combo</x:DropStyle><x:DropLines>{}</x:DropLines>",
                    now.drop_lines
                );
            }
        }
        ControlKind::SpinButton | ControlKind::ScrollBar => {
            reference(&mut x, "FmlaLink", &now.link);
            w!(
                x,
                "<x:Val>{}</x:Val><x:Min>{}</x:Min><x:Max>{}</x:Max><x:Inc>{}</x:Inc><x:Page>{}</x:Page>",
                now.value,
                now.min,
                now.max,
                now.inc,
                now.page
            );
            if now.horizontal {
                x.push_str("<x:Horiz/>");
            }
            x.push_str("<x:Dx>22</x:Dx>");
            if !now.shaded {
                x.push_str("<x:NoThreeD/>");
            }
        }
        ControlKind::GroupBox => {
            if !now.shaded {
                x.push_str("<x:NoThreeD/>");
            }
        }
        ControlKind::Button | ControlKind::Label => {}
    }
    x.push_str("</x:ClientData></v:shape>");
    x
}

/// The `ctrlProp` part of a control, attributes in the order of the
/// schema, the defaults left out, as Excel writes it.
fn properties_xml(now: &FormControl) -> String {
    let mut x = String::from(XML_DECL);
    w!(
        x,
        "<formControlPr xmlns=\"{NS_X14}\" objectType=\"{}\"",
        type_names(now.kind).1
    );
    let listed = now.kind.has_list();
    let ticked = matches!(now.kind, ControlKind::CheckBox | ControlKind::OptionButton);
    if ticked {
        match now.checked {
            CheckState::Unchecked => {}
            CheckState::Checked => x.push_str(" checked=\"Checked\""),
            CheckState::Mixed => x.push_str(" checked=\"Mixed\""),
        }
    }
    if now.kind == ControlKind::ComboBox {
        w!(x, " dropLines=\"{}\" dropStyle=\"combo\"", now.drop_lines);
    }
    if listed || now.kind.has_value() {
        x.push_str(" dx=\"22\"");
    }
    if now.kind == ControlKind::OptionButton && now.first_button {
        x.push_str(" firstButton=\"1\"");
    }
    if let Some(link) = now.link.as_ref().filter(|_| now.kind.has_link()) {
        x.push_str(" fmlaLink=\"");
        push_attr(&mut x, &formula(link));
        x.push('"');
    }
    if let Some(input) = now.input.as_ref().filter(|_| listed) {
        x.push_str(" fmlaRange=\"");
        push_attr(&mut x, &formula(input));
        x.push('"');
    }
    if now.kind.has_value() {
        if now.horizontal {
            x.push_str(" horiz=\"1\"");
        }
        if now.inc != 1 {
            w!(x, " inc=\"{}\"", now.inc);
        }
    }
    if now.kind.has_text() && now.kind != ControlKind::GroupBox && now.lock_text {
        x.push_str(" lockText=\"1\"");
    }
    if now.kind.has_value() {
        // The schema's maximum is 0 (and its page change 0): Excel says
        // what differs.
        if now.max != 0 {
            w!(x, " max=\"{}\"", now.max);
        }
        if now.min != 0 {
            w!(x, " min=\"{}\"", now.min);
        }
    }
    if now.kind == ControlKind::ListBox && now.selection != SelectionType::Single {
        x.push_str(match now.selection {
            SelectionType::Multi => " seltype=\"multi\"",
            _ => " seltype=\"extended\"",
        });
    }
    if !now.shaded && now.kind != ControlKind::Button && now.kind != ControlKind::Label {
        x.push_str(" noThreeD=\"1\"");
    }
    if now.kind.has_value() && now.page != 0 {
        w!(x, " page=\"{}\"", now.page);
    }
    if listed {
        w!(x, " sel=\"{}\" val=\"0\"", now.selected);
    }
    if now.kind.has_value() {
        w!(x, " val=\"{}\"", now.value);
    }
    x.push_str("/>");
    x
}

/// What a `<control>` element holds (`<controlPr>` and the anchor), as
/// Excel writes it for a new control.
fn control_pr_xml(sheet: &Sheet, now: &FormControl) -> String {
    let mut x = String::from("<controlPr defaultSize=\"0\"");
    if !now.locked {
        x.push_str(" locked=\"0\"");
    }
    if !now.print {
        x.push_str(" print=\"0\"");
    }
    if now.kind.has_text() {
        x.push_str(" autoFill=\"0\"");
    }
    if now.kind != ControlKind::Button
        && now.kind != ControlKind::SpinButton
        && now.kind != ControlKind::ScrollBar
    {
        x.push_str(" autoLine=\"0\"");
    }
    x.push_str(" autoPict=\"0\"");
    if let Some(name) = &now.macro_name {
        x.push_str(" macro=\"");
        push_attr(&mut x, name);
        x.push('"');
    }
    x.push_str("><anchor");
    if now.move_with_cells {
        x.push_str(" moveWithCells=\"1\"");
    }
    if now.size_with_cells {
        x.push_str(" sizeWithCells=\"1\"");
    }
    x.push('>');
    for (name, (row, col, dx, dy)) in ["from", "to"].into_iter().zip(corners(sheet, now)) {
        w!(
            x,
            "<{name}><xdr:col>{col}</xdr:col><xdr:colOff>{}</xdr:colOff><xdr:row>{row}</xdr:row><xdr:rowOff>{}</xdr:rowOff></{name}>",
            (dx * EMU_PER_PX).round() as i64,
            (dy * EMU_PER_PX).round() as i64
        );
    }
    x.push_str("</anchor></controlPr>");
    x
}

/// The pieces of every control of a sheet, in the order of the sheet:
/// (shape, `ctrlProp` part, inside of `<control>`, name, shape id).
struct Written {
    shape: String,
    properties: String,
    control: String,
    name: String,
    id: usize,
}

fn written(wb: &Workbook, index: usize) -> Vec<Written> {
    let sheet = &wb.sheets[index];
    let first = first_shape_id(wb, index);
    sheet
        .controls
        .iter()
        .enumerate()
        .map(|(k, control)| {
            let id = first + k;
            // What the control shows now is what the file says of it.
            let now = wb.control_state(index, control);
            let (shape, properties, inside) = match FormControl::unchanged_source(&now) {
                Some(source) => (
                    // The shape under the number it has in this file.
                    source
                        .shape
                        .replace(
                            &format!("_x0000_s{}\"", source.shape_id),
                            &format!("_x0000_s{id}\""),
                        )
                        .replace(
                            &format!("_x0000_s{}'", source.shape_id),
                            &format!("_x0000_s{id}'"),
                        ),
                    source.properties.clone(),
                    source.control.clone(),
                ),
                None => (
                    shape_xml(sheet, &now, id, sheet.comments.len() + k + 1),
                    properties_xml(&now),
                    control_pr_xml(sheet, &now),
                ),
            };
            Written {
                shape,
                properties: if properties.is_empty() {
                    properties_xml(&now)
                } else {
                    properties
                },
                control: if inside.is_empty() {
                    control_pr_xml(sheet, &now)
                } else {
                    inside
                },
                name: now.name.clone(),
                id,
            }
        })
        .collect()
}

/// The control shapes of the VML drawing of sheet `index`, after its
/// type of shape; empty for a sheet without controls.
pub(crate) fn vml_shapes(wb: &Workbook, index: usize) -> String {
    let shapes = written(wb, index);
    if shapes.is_empty() {
        return String::new();
    }
    let mut x = String::from(
        "<v:shapetype id=\"_x0000_t201\" coordsize=\"21600,21600\" o:spt=\"201\" path=\"m,l,21600r21600,l21600,xe\"><v:stroke joinstyle=\"miter\"/><v:path shadowok=\"f\" o:extrusionok=\"f\" strokeok=\"f\" fillok=\"f\" o:connecttype=\"rect\"/><o:lock v:ext=\"edit\" shapetype=\"t\"/></v:shapetype>",
    );
    for shape in shapes {
        x.push_str(&shape.shape);
    }
    x
}

/// The `<controls>` block of the worksheet of sheet `index`, which comes
/// after `<picture>` and before `<tableParts>`; empty without controls.
pub(crate) fn sheet_xml(wb: &Workbook, index: usize, links: &SheetLinks) -> String {
    let mut x = String::new();
    if links.controls.is_empty() {
        return x;
    }
    // The prefixes are declared here, where they are used: the worksheet
    // element of this writer does not carry them.
    w!(
        x,
        "<mc:AlternateContent xmlns:mc=\"{NS_MC}\"><mc:Choice xmlns:x14=\"{NS_X14}\" Requires=\"x14\"><controls xmlns:xdr=\"{NS_XDR}\">"
    );
    for (control, rel) in written(wb, index).iter().zip(&links.controls) {
        w!(
            x,
            "<mc:AlternateContent xmlns:mc=\"{NS_MC}\"><mc:Choice Requires=\"x14\"><control shapeId=\"{}\" r:id=\"rId{rel}\" name=\"",
            control.id
        );
        push_attr(&mut x, &control.name);
        w!(
            x,
            "\">{}</control></mc:Choice></mc:AlternateContent>",
            control.control
        );
    }
    x.push_str("</controls></mc:Choice></mc:AlternateContent>");
    x
}

/// The `ctrlProp` parts of the workbook, numbered through its sheets.
pub(crate) fn add_parts(wb: &Workbook, extras: &mut Extras) {
    let mut number = 0;
    for (index, sheet) in wb.sheets.iter().enumerate() {
        if sheet.controls.is_empty() {
            continue;
        }
        let links = SheetLinks::of(sheet);
        for (control, rel) in written(wb, index).into_iter().zip(&links.controls) {
            number += 1;
            extras.add_part(
                format!("xl/ctrlProps/ctrlProp{number}.xml"),
                Some(CT_CTRL_PROP),
                control.properties,
            );
            extras.relate_sheet(
                index,
                *rel,
                &format!("{NS_DOC_REL}/ctrlProp"),
                &format!("../ctrlProps/ctrlProp{number}.xml"),
            );
        }
    }
}

// ---- reading ---------------------------------------------------------------

/// A `<control>` element of a worksheet.
struct Listed {
    name: String,
    /// The relationship to its properties.
    rel: String,
    /// What the element holds, as written.
    inside: String,
}

/// The `<control>` elements of a worksheet part, by the number of their
/// shape.
fn listed_controls(sheet_xml: &str) -> HashMap<u32, Listed> {
    let mut out = HashMap::new();
    let Some(at) = sheet_xml.find("<controls") else {
        return out;
    };
    let mut r = Reader::new(&sheet_xml[at..]);
    loop {
        match r.next() {
            Event::Open(t) if t.name == "control" => {
                let (id, rel, name) = (t.attr_u32("shapeId"), t.attr_local("id"), t.attr("name"));
                let inside = if t.empty {
                    String::new()
                } else {
                    let from = r.rest();
                    r.skip(&t);
                    let held = &from[..from.len() - r.rest().len()];
                    held.rfind("</").map_or("", |end| &held[..end]).to_string()
                };
                // The fallback of a choice repeats the element, bare.
                if let (Some(id), Some(rel)) = (id, rel)
                    && !out.contains_key(&id)
                {
                    out.insert(
                        id,
                        Listed {
                            name: name.unwrap_or_default().into_owned(),
                            rel: rel.into_owned(),
                            inside,
                        },
                    );
                }
            }
            Event::Close("controls") | Event::Eof => break,
            _ => {}
        }
    }
    out
}

/// A reference as files write it, when it is one.
fn parse_reference(text: &str) -> Option<RefExpr> {
    match parse_formula(text.trim().trim_start_matches('='), Locale::en()) {
        Ok(Expr::Ref(r)) => Some(r),
        _ => None,
    }
}

/// Read a control out of its VML shape (`raw`, from `<v:shape` to
/// `</v:shape>`); `None` for a shape that is no form control (the box of
/// a note, an ActiveX control, a picture).
fn parse_shape(raw: &str, sheet: &Sheet) -> Option<(u32, FormControl)> {
    let mut r = Reader::new(raw);
    let Event::Open(shape) = r.next() else {
        return None;
    };
    let id = shape.attr("id")?;
    let shape_id: u32 = id
        .rsplit_once("_s")
        .and_then(|(_, digits)| digits.parse().ok())?;
    let mut control: Option<FormControl> = None;
    let mut text = String::new();
    let mut anchor: Option<Vec<f64>> = None;
    // Present and empty, or "True", says yes.
    let yes = |text: &str| {
        !matches!(
            text.trim().to_ascii_lowercase().as_str(),
            "false" | "f" | "0"
        )
    };
    let (mut moves, mut sizes, mut flat, mut list_lines) = (true, true, false, None);
    loop {
        match r.next() {
            Event::Open(t) => match (t.name, &mut control) {
                ("textbox", _) => text = r.text(&t).trim().to_string(),
                ("ClientData", None) => {
                    control = Some(FormControl::new(kind_of(&t.attr("ObjectType")?)?));
                    if t.empty {
                        break;
                    }
                }
                (name, Some(c)) => {
                    let value = r.text(&t);
                    let number = || value.trim().parse::<i32>().ok();
                    match name {
                        "MoveWithCells" => moves = !yes(&value),
                        "SizeWithCells" => sizes = !yes(&value),
                        "Anchor" => {
                            anchor = Some(
                                value
                                    .split(',')
                                    .map(|n| n.trim().parse::<f64>().unwrap_or(0.0))
                                    .collect(),
                            )
                        }
                        "Locked" => c.locked = yes(&value),
                        "PrintObject" => c.print = yes(&value),
                        "LockText" => c.lock_text = yes(&value),
                        "FmlaMacro" => {
                            c.macro_name =
                                Some(value.trim().to_string()).filter(|name| !name.is_empty())
                        }
                        "FmlaLink" => c.link = parse_reference(&value),
                        "FmlaRange" => c.input = parse_reference(&value),
                        "Checked" => {
                            c.checked = match number() {
                                Some(1) => CheckState::Checked,
                                Some(2) => CheckState::Mixed,
                                _ => CheckState::Unchecked,
                            }
                        }
                        "FirstButton" => c.first_button = yes(&value),
                        "NoThreeD" | "NoThreeD2" => flat = yes(&value),
                        "Sel" => c.selected = number().unwrap_or(0).max(0) as u32,
                        "Val" if c.kind.has_value() => c.value = number().unwrap_or(0),
                        "Min" if c.kind.has_value() => c.min = number().unwrap_or(0),
                        "Max" if c.kind.has_value() => c.max = number().unwrap_or(0),
                        "Inc" if c.kind.has_value() => c.inc = number().unwrap_or(1),
                        "Page" if c.kind.has_value() => c.page = number().unwrap_or(0),
                        "Horiz" => c.horizontal = yes(&value),
                        "DropLines" => list_lines = number(),
                        "SelType" => {
                            c.selection = match value.trim().to_ascii_lowercase().as_str() {
                                "multi" => SelectionType::Multi,
                                "extend" => SelectionType::Extend,
                                _ => SelectionType::Single,
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            },
            Event::Eof => break,
            _ => {}
        }
    }
    let mut control = control?;
    control.move_with_cells = moves;
    control.size_with_cells = sizes;
    // A button and a label have no shading to speak of.
    control.shaded = !flat && !matches!(control.kind, ControlKind::Button | ControlKind::Label);
    if let Some(lines) = list_lines {
        control.drop_lines = lines.max(0) as u32;
    }
    if control.kind.has_text() {
        control.text = text;
    }
    // col, dx, row, dy of the two corners, in pixels.
    let a = anchor.filter(|a| a.len() == 8)?;
    let corner = |k: usize| {
        sheet.corner_point((
            (a[k + 2] as u32, a[k] as u32),
            (a[k + 1] as f32, a[k + 3] as f32),
        ))
    };
    let ((x0, y0), (x1, y1)) = (corner(0), corner(4));
    let (anchor, offset) = sheet.corner_at(x0, y0);
    control.anchor = anchor;
    control.offset = offset;
    control.size = (((x1 - x0) as f32).max(1.0), ((y1 - y0) as f32).max(1.0));
    Some((shape_id, control))
}

/// Read the form controls of a sheet into it: the shapes of its VML
/// drawing that are controls, each with its `ctrlProp` part and its
/// `<control>` element when the file has them (a file of Excel 2007 has
/// the shapes alone). The sheet must already have its columns and rows.
/// `next_id` is the id the next control of the workbook gets.
pub(crate) fn read_sheet<R: Read + Seek>(
    pkg: &mut Package<R>,
    sheet_part: &str,
    sheet_xml: &str,
    sheet: &mut Sheet,
    next_id: &mut u32,
) {
    let rels = relationships(pkg, sheet_part);
    let listed = listed_controls(sheet_xml);
    for rel in rels.iter().filter(|rel| rel.kind == "vmlDrawing") {
        let Ok(Some(vml)) = pkg.read(&rel.target) else {
            continue;
        };
        let mut rest = vml.as_str();
        // Shapes are picked out by their tags: the drawing is not always
        // tidy XML.
        while let Some(start) = rest.find("<v:shape ") {
            let Some(length) = rest[start..].find("</v:shape>") else {
                break;
            };
            let raw = &rest[start..start + length + "</v:shape>".len()];
            rest = &rest[start + raw.len()..];
            let Some((shape_id, mut control)) = parse_shape(raw, sheet) else {
                continue;
            };
            let entry = listed.get(&shape_id);
            // A shape the worksheet lists as something else than a form
            // control (an ActiveX control) is not one.
            let properties = match entry {
                Some(entry) => match rels.iter().find(|rel| rel.id == entry.rel) {
                    Some(rel) if rel.kind == "ctrlProp" => {
                        pkg.read(&rel.target).ok().flatten().unwrap_or_default()
                    }
                    _ => continue,
                },
                None => String::new(),
            };
            control.name = match entry {
                Some(entry) if !entry.name.is_empty() => entry.name.clone(),
                _ => sheet.new_control_name(control.kind),
            };
            control.source = Some(Arc::new(ControlSource {
                shape: raw.to_string(),
                shape_id,
                properties,
                control: entry.map(|entry| entry.inside.clone()).unwrap_or_default(),
                as_read: Box::new(control.content()),
            }));
            control.id = *next_id;
            *next_id += 1;
            sheet.controls.push(control);
        }
    }
}
