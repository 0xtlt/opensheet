//! Form controls in `.xlsx` and `.xlsm` files: the shapes of the legacy
//! VML drawing, the `ctrlProp` parts and the `<controls>` block of the
//! worksheet, written as Excel writes them and kept as they were when a
//! control read from a file did not change.

mod common;

use common::{assert_well_formed, enter, keep_sample, part, part_names};
use opensheet_engine::ast::{Expr, RefExpr};
use opensheet_engine::control::{CheckState, ControlKind, FormControl};
use opensheet_engine::parser::parse_formula;
use opensheet_engine::{Locale, Value, Workbook};
use opensheet_io::vba::{ModuleKind, build_project};
use std::io::{Cursor, Write};

const CT: &str = "http://schemas.openxmlformats.org/package/2006/content-types";
const PKG_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const DOC_REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const MS_REL: &str = "http://schemas.microsoft.com/office/2006/relationships";
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const CT_PROP: &str = "application/vnd.ms-excel.controlproperties+xml";

fn range(text: &str) -> RefExpr {
    match parse_formula(text, Locale::en()) {
        Ok(Expr::Ref(r)) => r,
        other => panic!("{text}: {other:?}"),
    }
}

fn load(bytes: &[u8]) -> Workbook {
    opensheet_io::read_xlsx(Cursor::new(bytes), Locale::en()).expect("a workbook")
}

fn save_xlsm(wb: &Workbook) -> Vec<u8> {
    opensheet_io::write_xlsm(wb, Cursor::new(Vec::new())).expect("a file").into_inner()
}

fn save_xlsx(wb: &Workbook) -> Vec<u8> {
    opensheet_io::write_xlsx(wb, Cursor::new(Vec::new())).expect("a file").into_inner()
}

fn zip(parts: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, content) in parts {
        zip.start_file(*name, options).unwrap();
        zip.write_all(content).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

/// Every XML part of the package parses.
fn all_well_formed(bytes: &[u8]) {
    for name in part_names(bytes) {
        if name.ends_with(".xml") || name.ends_with(".rels") {
            assert_well_formed(&name, &part(bytes, &name));
        } else if name.ends_with(".vml") {
            // A VML drawing has no declaration, in Excel's files either.
            assert_well_formed(&name, &format!("<?xml version=\"1.0\"?>{}", part(bytes, &name)));
        }
    }
}

fn add(wb: &mut Workbook, kind: ControlKind, at: (u32, u32), edit: impl FnOnce(&mut FormControl)) -> u32 {
    let mut control = FormControl::new(kind);
    control.anchor = at;
    edit(&mut control);
    wb.add_control(0, control).expect("a sheet")
}

/// One control of each kind on a sheet with the cells they are linked to,
/// a note beside them, and a second sheet with a button of its own.
fn book() -> Workbook {
    let mut wb = Workbook::new(Locale::en());
    for (cell, text) in [
        ("A1", "Linked cells"),
        ("A2", "TRUE"),
        ("A3", "2"),
        ("A4", "3"),
        ("A5", "1"),
        ("A6", "12"),
        ("A7", "40"),
        ("H1", "Colours"),
        ("H2", "Red"),
        ("H3", "Green"),
        ("H4", "Blue"),
        ("H5", "Black"),
    ] {
        enter(&mut wb, 0, cell, text);
    }
    wb.set_note(0, 0, 0, "Me", "What each control writes");
    add(&mut wb, ControlKind::Button, (1, 2), |c| {
        c.text = "Format report".to_string();
        c.macro_name = Some("[0]!FormatReport".to_string());
    });
    add(&mut wb, ControlKind::CheckBox, (3, 2), |c| {
        c.text = "Show totals".to_string();
        c.link = Some(range("$A$2"));
        c.checked = CheckState::Checked;
    });
    add(&mut wb, ControlKind::GroupBox, (5, 2), |c| {
        c.text = "Size".to_string();
        c.size = (150.0, 70.0);
    });
    add(&mut wb, ControlKind::OptionButton, (6, 2), |c| {
        c.text = "Small".to_string();
        c.offset = (8.0, 2.0);
        c.link = Some(range("$A$3"));
    });
    add(&mut wb, ControlKind::OptionButton, (7, 2), |c| {
        c.text = "Large".to_string();
        c.offset = (8.0, 2.0);
        c.link = Some(range("$A$3"));
        c.checked = CheckState::Checked;
    });
    add(&mut wb, ControlKind::ComboBox, (10, 2), |c| {
        c.link = Some(range("$A$4"));
        c.input = Some(range("$H$2:$H$5"));
        c.selected = 3;
        c.drop_lines = 6;
    });
    add(&mut wb, ControlKind::ListBox, (12, 2), |c| {
        c.link = Some(range("$A$5"));
        c.input = Some(range("$H$2:$H$5"));
        c.selected = 1;
    });
    add(&mut wb, ControlKind::SpinButton, (1, 5), |c| {
        c.link = Some(range("$A$6"));
        c.value = 12;
        c.min = 10;
        c.max = 20;
        c.inc = 2;
    });
    add(&mut wb, ControlKind::ScrollBar, (1, 6), |c| {
        c.link = Some(range("$A$7"));
        c.value = 40;
        c.page = 20;
    });
    add(&mut wb, ControlKind::Label, (18, 2), |c| c.text = "A label".to_string());
    wb.add_sheet(1);
    let mut second = FormControl::new(ControlKind::Button);
    second.anchor = (2, 1);
    second.text = "Clear".to_string();
    second.macro_name = Some("[0]!ClearReport".to_string());
    wb.add_control(1, second).unwrap();
    wb.commit("controls");
    wb
}

/// The controls of a sheet without what a trip through a file may not keep
/// to the last bit (ids, what they remember of the file).
fn contents(wb: &Workbook, sheet: usize) -> Vec<FormControl> {
    wb.sheets[sheet].controls.iter().map(|c| wb.control_state(sheet, c).content()).collect()
}

#[test]
fn one_control_of_each_kind_goes_through_a_macro_workbook() {
    let mut wb = book();
    wb.vba.workbook_code_name = Some("ThisWorkbook".to_string());
    wb.sheets[0].code_name = Some("Sheet1".to_string());
    wb.sheets[1].code_name = Some("Sheet2".to_string());
    let project = build_project(&[
        ("ThisWorkbook", ModuleKind::Document, ""),
        ("Sheet1", ModuleKind::Document, ""),
        ("Sheet2", ModuleKind::Document, ""),
        (
            "Module1",
            ModuleKind::Standard,
            "Sub FormatReport()\n    Range(\"A1\").Font.Bold = True\nEnd Sub\n\nSub ClearReport()\n    Range(\"A2:A7\").ClearContents\nEnd Sub\n",
        ),
    ]);
    wb.vba.project = Some(opensheet_engine::vba::VbaProject {
        bin: project.into(),
        signatures: Vec::new(),
    });
    let bytes = save_xlsm(&wb);
    keep_sample("form-controls.xlsm", &bytes);
    all_well_formed(&bytes);
    let names = part_names(&bytes);
    // Ten controls on the first sheet and one on the second: a part each.
    for n in 1..=11 {
        assert!(names.contains(&format!("xl/ctrlProps/ctrlProp{n}.xml")), "{names:?}");
    }
    assert!(names.contains(&"xl/drawings/vmlDrawing1.vml".to_string()) && names.contains(&"xl/drawings/vmlDrawing2.vml".to_string()));
    let types = part(&bytes, "[Content_Types].xml");
    assert!(types.contains(&format!("<Override PartName=\"/xl/ctrlProps/ctrlProp11.xml\" ContentType=\"{CT_PROP}\"/>")), "{types}");
    assert!(types.contains("Extension=\"vml\""));

    // The worksheet: the legacy drawing, then the controls in a choice
    // older readers skip, each naming its shape and its properties.
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    let rels = part(&bytes, "xl/worksheets/_rels/sheet1.xml.rels");
    assert!(sheet.contains("<legacyDrawing r:id=\"rId1\"/><mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"><mc:Choice xmlns:x14=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\" Requires=\"x14\"><controls xmlns:xdr=\"http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing\">"), "{sheet}");
    assert!(
        sheet.contains("<control shapeId=\"1026\" r:id=\"rId3\" name=\"Button 1\"><controlPr defaultSize=\"0\" print=\"0\" autoFill=\"0\" autoPict=\"0\" macro=\"[0]!FormatReport\"><anchor moveWithCells=\"1\" sizeWithCells=\"1\"><from><xdr:col>2</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></from><to><xdr:col>3</xdr:col>"),
        "{sheet}"
    );
    assert!(sheet.contains("<control shapeId=\"1027\" r:id=\"rId4\" name=\"Check Box 2\"><controlPr defaultSize=\"0\" autoFill=\"0\" autoLine=\"0\" autoPict=\"0\"><anchor moveWithCells=\"1\"><from>"), "{sheet}");
    assert_eq!(sheet.matches("<control ").count(), 10);
    assert!(sheet.ends_with("</controls></mc:Choice></mc:AlternateContent></worksheet>"), "{sheet}");
    assert!(rels.contains(&format!("<Relationship Id=\"rId1\" Type=\"{DOC_REL}/vmlDrawing\" Target=\"../drawings/vmlDrawing1.vml\"/>")), "{rels}");
    assert!(rels.contains(&format!("<Relationship Id=\"rId3\" Type=\"{DOC_REL}/ctrlProp\" Target=\"../ctrlProps/ctrlProp1.xml\"/>")), "{rels}");

    // The VML drawing: the note first, then the controls, numbered on.
    let vml = part(&bytes, "xl/drawings/vmlDrawing1.vml");
    assert!(vml.contains("<o:idmap v:ext=\"edit\" data=\"1\"/>"));
    assert!(vml.contains("<v:shape id=\"_x0000_s1025\" type=\"#_x0000_t202\""), "the note: {vml}");
    assert!(vml.contains("<v:shapetype id=\"_x0000_t201\" coordsize=\"21600,21600\" o:spt=\"201\""));
    assert!(
        vml.contains("<x:ClientData ObjectType=\"Button\"><x:Anchor>2, 0, 1, 0, 3, 32, 2, 10</x:Anchor><x:PrintObject>False</x:PrintObject><x:AutoFill>False</x:AutoFill><x:FmlaMacro>[0]!FormatReport</x:FmlaMacro><x:TextHAlign>Center</x:TextHAlign><x:TextVAlign>Center</x:TextVAlign></x:ClientData></v:shape>"),
        "{vml}"
    );
    assert!(vml.contains(">Format report</font></div></v:textbox>"));
    assert!(vml.contains("<x:ClientData ObjectType=\"Checkbox\"><x:SizeWithCells/><x:Anchor>"), "{vml}");
    assert!(vml.contains("<x:Checked>1</x:Checked><x:FmlaLink>$A$2</x:FmlaLink><x:NoThreeD/></x:ClientData>"), "{vml}");
    assert!(vml.contains("<x:FmlaLink>$A$3</x:FmlaLink><x:NoThreeD/><x:FirstButton/></x:ClientData>"), "{vml}");
    assert!(vml.contains("<x:ClientData ObjectType=\"Drop\">"), "{vml}");
    assert!(vml.contains("<x:FmlaRange>$H$2:$H$5</x:FmlaRange><x:FmlaLink>$A$4</x:FmlaLink>"), "{vml}");
    assert!(vml.contains("<x:Sel>3</x:Sel><x:NoThreeD2/><x:SelType>Single</x:SelType><x:LCT>Normal</x:LCT><x:DropStyle>Combo</x:DropStyle><x:DropLines>6</x:DropLines></x:ClientData>"), "{vml}");
    assert!(vml.contains("<x:ClientData ObjectType=\"Spin\"><x:Anchor>"), "{vml}");
    assert!(vml.contains("<x:FmlaLink>$A$6</x:FmlaLink><x:Val>12</x:Val><x:Min>10</x:Min><x:Max>20</x:Max><x:Inc>2</x:Inc><x:Page>10</x:Page><x:Dx>22</x:Dx><x:NoThreeD/></x:ClientData>"), "{vml}");
    for kind in ["Radio", "List", "Scroll", "GBox", "Label"] {
        assert!(vml.contains(&format!("<x:ClientData ObjectType=\"{kind}\">")), "{kind}: {vml}");
    }
    // The second sheet has the next block of ids.
    let second = part(&bytes, "xl/drawings/vmlDrawing2.vml");
    assert!(second.contains("data=\"2\"") && second.contains("<v:shape id=\"_x0000_s2049\" type=\"#_x0000_t201\""), "{second}");
    assert!(part(&bytes, "xl/worksheets/sheet2.xml").contains("<control shapeId=\"2049\" r:id=\"rId2\" name=\"Button 1\">"));

    // The properties.
    let prop = |n: usize| part(&bytes, &format!("xl/ctrlProps/ctrlProp{n}.xml"));
    let x14 = "xmlns=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\"";
    assert!(prop(1).ends_with(&format!("<formControlPr {x14} objectType=\"Button\" lockText=\"1\"/>")), "{}", prop(1));
    assert!(prop(2).ends_with(&format!("<formControlPr {x14} objectType=\"CheckBox\" checked=\"Checked\" fmlaLink=\"$A$2\" lockText=\"1\" noThreeD=\"1\"/>")), "{}", prop(2));
    assert!(prop(3).ends_with(&format!("<formControlPr {x14} objectType=\"GBox\" noThreeD=\"1\"/>")), "{}", prop(3));
    assert!(prop(4).ends_with(&format!("<formControlPr {x14} objectType=\"Radio\" firstButton=\"1\" fmlaLink=\"$A$3\" lockText=\"1\" noThreeD=\"1\"/>")), "{}", prop(4));
    assert!(prop(5).ends_with(&format!("<formControlPr {x14} objectType=\"Radio\" checked=\"Checked\" fmlaLink=\"$A$3\" lockText=\"1\" noThreeD=\"1\"/>")), "{}", prop(5));
    assert!(prop(6).ends_with(&format!("<formControlPr {x14} objectType=\"Drop\" dropLines=\"6\" dropStyle=\"combo\" dx=\"22\" fmlaLink=\"$A$4\" fmlaRange=\"$H$2:$H$5\" noThreeD=\"1\" sel=\"3\" val=\"0\"/>")), "{}", prop(6));
    assert!(prop(8).ends_with(&format!("<formControlPr {x14} objectType=\"Spin\" dx=\"22\" fmlaLink=\"$A$6\" inc=\"2\" max=\"20\" min=\"10\" noThreeD=\"1\" page=\"10\" val=\"12\"/>")), "{}", prop(8));
    assert!(prop(9).ends_with(&format!("<formControlPr {x14} objectType=\"Scroll\" dx=\"22\" fmlaLink=\"$A$7\" max=\"100\" noThreeD=\"1\" page=\"20\" val=\"40\"/>")), "{}", prop(9));

    // Read back: every control, its place, its settings, its macro.
    let back = load(&bytes);
    for (read, written) in contents(&back, 0).iter().zip(contents(&wb, 0)) {
        assert_eq!(*read, written);
    }
    assert_eq!(contents(&back, 0).len(), 10);
    assert_eq!(contents(&back, 1), contents(&wb, 1));
    assert_eq!(back.sheets[0].controls[0].macro_name.as_deref(), Some("[0]!FormatReport"));
    assert!(back.sheets[0].drawings.is_empty() && back.sheets[0].comments.len() == 1);
    // A second trip changes nothing, as a macro workbook or not.
    assert_eq!(save_xlsm(&back), bytes);
    let plain = save_xlsx(&back);
    assert_eq!(contents(&load(&plain), 0), contents(&wb, 0), "controls do not need the project");
}

/// The states written are those the linked cells show.
#[test]
fn the_file_says_what_the_linked_cells_say() {
    let mut wb = book();
    enter(&mut wb, 0, "A2", "FALSE");
    enter(&mut wb, 0, "A3", "1");
    enter(&mut wb, 0, "A6", "18");
    wb.commit("cells");
    let bytes = save_xlsx(&wb);
    assert!(part(&bytes, "xl/ctrlProps/ctrlProp2.xml").contains("objectType=\"CheckBox\" fmlaLink=\"$A$2\""));
    assert!(part(&bytes, "xl/ctrlProps/ctrlProp4.xml").contains("objectType=\"Radio\" checked=\"Checked\" firstButton=\"1\""));
    assert!(part(&bytes, "xl/ctrlProps/ctrlProp8.xml").contains("val=\"18\""));
    let vml = part(&bytes, "xl/drawings/vmlDrawing1.vml");
    assert!(vml.contains("<x:Val>18</x:Val>") && !vml.contains("<x:Val>12</x:Val>"));
}

const BUTTON_SHAPE: &str = "<v:shape id=\"_x0000_s1025\" type=\"#_x0000_t201\" style='position:absolute;\n  margin-left:48pt;margin-top:15pt;width:96pt;height:30pt;z-index:1;\n  mso-wrap-style:tight' o:button=\"t\" fillcolor=\"buttonFace [67]\"\n  strokecolor=\"windowText [64]\" o:insetmode=\"auto\">\n  <v:fill color2=\"buttonFace [67]\" o:detectmouseclick=\"t\"/>\n  <o:lock v:ext=\"edit\" rotation=\"t\"/>\n  <v:textbox style='mso-direction-alt:auto' o:singleclick=\"f\">\n   <div style='text-align:center'><font face=\"Arial Black\" size=\"280\" color=\"#C00000\">Run it</font></div>\n  </v:textbox>\n  <x:ClientData ObjectType=\"Button\">\n   <x:Anchor>\n    1, 0, 1, 0, 3, 0, 3, 0</x:Anchor>\n   <x:PrintObject>False</x:PrintObject>\n   <x:AutoFill>False</x:AutoFill>\n   <x:FmlaMacro>[0]!Sheet1.Recalculate</x:FmlaMacro>\n   <x:TextHAlign>Center</x:TextHAlign>\n   <x:TextVAlign>Center</x:TextVAlign>\n  </x:ClientData>\n </v:shape>";

const CHECK_SHAPE: &str = "<v:shape id=\"_x0000_s1026\" type=\"#_x0000_t201\" style='position:absolute;\n  margin-left:48pt;margin-top:60pt;width:99pt;height:16.5pt;z-index:2;\n  mso-wrap-style:tight' filled=\"f\" fillcolor=\"window [65]\" stroked=\"f\"\n  strokecolor=\"windowText [64]\" o:insetmode=\"auto\">\n  <v:path shadowok=\"t\" strokeok=\"t\" fillok=\"t\"/>\n  <o:lock v:ext=\"edit\" rotation=\"t\"/>\n  <v:textbox style='mso-direction-alt:auto' o:singleclick=\"f\">\n   <div style='text-align:left'><font face=\"Segoe UI\" size=\"160\" color=\"auto\">Urgent &amp; new</font></div>\n  </v:textbox>\n  <x:ClientData ObjectType=\"Checkbox\">\n   <x:SizeWithCells/>\n   <x:Anchor>\n    1, 0, 4, 0, 2, 68, 5, 2</x:Anchor>\n   <x:AutoFill>False</x:AutoFill>\n   <x:AutoLine>False</x:AutoLine>\n   <x:TextVAlign>Center</x:TextVAlign>\n   <x:Checked>1</x:Checked>\n   <x:FmlaLink>$A$1</x:FmlaLink>\n   <x:NoThreeD/>\n  </x:ClientData>\n </v:shape>";

/// An ActiveX control beside them: its shape is a picture.
const ACTIVEX_SHAPE: &str = "<v:shape id=\"_x0000_s1027\" type=\"#_x0000_t201\" style='position:absolute;margin-left:200pt;margin-top:15pt;width:72pt;height:24pt;z-index:3' stroked=\"f\" o:insetmode=\"auto\"><x:ClientData ObjectType=\"Pict\"><x:SizeWithCells/><x:Anchor>4, 0, 1, 0, 5, 32, 2, 12</x:Anchor><x:CF>Pict</x:CF><x:AutoPict/></x:ClientData></v:shape>";

/// A workbook as Excel lays one out: a button assigned to a macro and a
/// ticked check box, with a caption font the model does not hold, the
/// twins of the controls in the DrawingML part, and an ActiveX control.
fn excel_workbook() -> Vec<u8> {
    let types = format!(
        "<?xml version=\"1.0\"?><Types xmlns=\"{CT}\"><Default Extension=\"bin\" ContentType=\"application/vnd.ms-office.vbaProject\"/><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"vml\" ContentType=\"application/vnd.openxmlformats-officedocument.vmlDrawing\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.ms-excel.sheet.macroEnabled.main+xml\"/><Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/><Override PartName=\"/xl/drawings/drawing1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.drawing+xml\"/><Override PartName=\"/xl/ctrlProps/ctrlProp1.xml\" ContentType=\"{CT_PROP}\"/><Override PartName=\"/xl/ctrlProps/ctrlProp2.xml\" ContentType=\"{CT_PROP}\"/><Override PartName=\"/xl/activeX/activeX1.xml\" ContentType=\"application/vnd.ms-office.activeX+xml\"/></Types>"
    );
    let root = format!("<Relationships xmlns=\"{PKG_REL}\"><Relationship Id=\"rId1\" Type=\"{DOC_REL}/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>");
    let workbook = format!(
        "<workbook xmlns=\"{MAIN}\" xmlns:r=\"{DOC_REL}\"><workbookPr codeName=\"ThisWorkbook\"/><sheets><sheet name=\"Data\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>"
    );
    let workbook_rels = format!(
        "<Relationships xmlns=\"{PKG_REL}\"><Relationship Id=\"rId1\" Type=\"{DOC_REL}/worksheet\" Target=\"worksheets/sheet1.xml\"/><Relationship Id=\"rId2\" Type=\"{MS_REL}/vbaProject\" Target=\"vbaProject.bin\"/></Relationships>"
    );
    let anchor = |from: &str, to: &str| format!("<from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>{from}</xdr:row><xdr:rowOff>0</xdr:rowOff></from><to><xdr:col>{to}</xdr:col></to>");
    let sheet = format!(
        "<worksheet xmlns=\"{MAIN}\" xmlns:r=\"{DOC_REL}\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" xmlns:x14=\"http://schemas.microsoft.com/office/spreadsheetml/2009/9/main\" xmlns:xdr=\"http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing\" mc:Ignorable=\"x14\"><sheetPr codeName=\"Sheet1\"/><sheetData><row r=\"1\"><c r=\"A1\" t=\"b\"><v>1</v></c></row></sheetData><drawing r:id=\"rId1\"/><legacyDrawing r:id=\"rId2\"/><mc:AlternateContent><mc:Choice Requires=\"x14\"><controls><mc:AlternateContent><mc:Choice Requires=\"x14\"><control shapeId=\"1025\" r:id=\"rId3\" name=\"Button 1\"><controlPr defaultSize=\"0\" print=\"0\" autoFill=\"0\" autoPict=\"0\" macro=\"[0]!Sheet1.Recalculate\" altText=\"Recalculate the sheet\"><anchor moveWithCells=\"1\" sizeWithCells=\"1\">{}</anchor></controlPr></control></mc:Choice></mc:AlternateContent><mc:AlternateContent><mc:Choice Requires=\"x14\"><control shapeId=\"1026\" r:id=\"rId4\" name=\"Check Box 2\"><controlPr defaultSize=\"0\" autoFill=\"0\" autoLine=\"0\" autoPict=\"0\"><anchor moveWithCells=\"1\">{}</anchor></controlPr></control></mc:Choice></mc:AlternateContent><mc:AlternateContent><mc:Choice Requires=\"x14\"><control shapeId=\"1027\" r:id=\"rId5\" name=\"CommandButton1\"><controlPr defaultSize=\"0\" autoLine=\"0\" r:id=\"rId6\"/></control></mc:Choice><mc:Fallback><control shapeId=\"1027\" r:id=\"rId5\" name=\"CommandButton1\"/></mc:Fallback></mc:AlternateContent></controls></mc:Choice></mc:AlternateContent></worksheet>",
        anchor("1", "3"),
        anchor("4", "2"),
    );
    let sheet_rels = format!(
        "<Relationships xmlns=\"{PKG_REL}\"><Relationship Id=\"rId1\" Type=\"{DOC_REL}/drawing\" Target=\"../drawings/drawing1.xml\"/><Relationship Id=\"rId2\" Type=\"{DOC_REL}/vmlDrawing\" Target=\"../drawings/vmlDrawing1.vml\"/><Relationship Id=\"rId3\" Type=\"{DOC_REL}/ctrlProp\" Target=\"../ctrlProps/ctrlProp1.xml\"/><Relationship Id=\"rId4\" Type=\"{DOC_REL}/ctrlProp\" Target=\"../ctrlProps/ctrlProp2.xml\"/><Relationship Id=\"rId5\" Type=\"{DOC_REL}/control\" Target=\"../activeX/activeX1.xml\"/></Relationships>"
    );
    let twin = |id: u32, name: &str| {
        format!(
            "<mc:AlternateContent xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"><mc:Choice xmlns:a14=\"http://schemas.microsoft.com/office/drawing/2010/main\" Requires=\"a14\"><xdr:twoCellAnchor editAs=\"oneCell\"><xdr:from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>3</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>3</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:sp macro=\"\" textlink=\"\"><xdr:nvSpPr><xdr:cNvPr id=\"{id}\" name=\"{name}\" hidden=\"1\"><a:extLst><a:ext uri=\"{{63B3BB69-23CF-44E3-9099-C40C66FF867C}}\"><a14:compatExt spid=\"_x0000_s{id}\"/></a:ext></a:extLst></xdr:cNvPr><xdr:cNvSpPr/></xdr:nvSpPr><xdr:spPr bwMode=\"auto\"><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"0\" cy=\"0\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom><a:noFill/></xdr:spPr></xdr:sp><xdr:clientData fPrintsWithSheet=\"0\"/></xdr:twoCellAnchor></mc:Choice><mc:Fallback/></mc:AlternateContent>"
        )
    };
    let drawing = format!(
        "<xdr:wsDr xmlns:xdr=\"http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\">{}{}</xdr:wsDr>",
        twin(1025, "Button 1"),
        twin(1026, "Check Box 2")
    );
    let vml = format!(
        "<xml xmlns:v=\"urn:schemas-microsoft-com:vml\"\n xmlns:o=\"urn:schemas-microsoft-com:office:office\"\n xmlns:x=\"urn:schemas-microsoft-com:office:excel\">\n <o:shapelayout v:ext=\"edit\">\n  <o:idmap v:ext=\"edit\" data=\"1\"/>\n </o:shapelayout><v:shapetype id=\"_x0000_t201\" coordsize=\"21600,21600\" o:spt=\"201\"\n  path=\"m,l,21600r21600,l21600,xe\">\n  <v:stroke joinstyle=\"miter\"/>\n  <v:path shadowok=\"f\" o:extrusionok=\"f\" strokeok=\"f\" fillok=\"f\" o:connecttype=\"rect\"/>\n  <o:lock v:ext=\"edit\" shapetype=\"t\"/>\n </v:shapetype>{BUTTON_SHAPE}{CHECK_SHAPE}{ACTIVEX_SHAPE}</xml>"
    );
    let x14 = "http://schemas.microsoft.com/office/spreadsheetml/2009/9/main";
    let project = build_project(&[
        ("ThisWorkbook", ModuleKind::Document, ""),
        ("Sheet1", ModuleKind::Document, "Sub Recalculate()\n    Me.Calculate\nEnd Sub\n"),
    ]);
    zip(&[
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", root.as_bytes()),
        ("xl/workbook.xml", workbook.as_bytes()),
        ("xl/_rels/workbook.xml.rels", workbook_rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
        ("xl/worksheets/_rels/sheet1.xml.rels", sheet_rels.as_bytes()),
        ("xl/drawings/drawing1.xml", drawing.as_bytes()),
        ("xl/drawings/vmlDrawing1.vml", vml.as_bytes()),
        (
            "xl/ctrlProps/ctrlProp1.xml",
            format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<formControlPr xmlns=\"{x14}\" objectType=\"Button\" lockText=\"1\"/>").as_bytes(),
        ),
        (
            "xl/ctrlProps/ctrlProp2.xml",
            format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<formControlPr xmlns=\"{x14}\" objectType=\"CheckBox\" checked=\"Checked\" fmlaLink=\"$A$1\" lockText=\"1\" noThreeD=\"1\"/>").as_bytes(),
        ),
        ("xl/activeX/activeX1.xml", b"<ax:ocx xmlns:ax=\"http://schemas.microsoft.com/office/2006/activeX\"/>"),
        ("xl/vbaProject.bin", &project),
    ])
}

#[test]
fn the_controls_of_a_file_from_excel_are_kept_as_they_were() {
    let original = excel_workbook();
    let wb = load(&original);
    let sheet = &wb.sheets[0];
    // The two form controls; not their twins of the drawing, not the
    // ActiveX control.
    assert!(sheet.drawings.is_empty(), "{:?}", sheet.drawings.len());
    let kinds: Vec<ControlKind> = sheet.controls.iter().map(|c| c.kind).collect();
    assert_eq!(kinds, [ControlKind::Button, ControlKind::CheckBox]);
    let (button, check) = (&sheet.controls[0], &sheet.controls[1]);
    assert_eq!((button.name.as_str(), button.text.as_str()), ("Button 1", "Run it"));
    assert_eq!(button.macro_name.as_deref(), Some("[0]!Sheet1.Recalculate"));
    assert_eq!(button.macro_short_name(), Some("Sheet1.Recalculate"));
    assert_eq!((button.anchor, button.offset, button.size), ((1, 1), (0.0, 0.0), (128.0, 40.0)));
    assert!(button.move_with_cells && button.size_with_cells && !button.print);
    assert_eq!((check.name.as_str(), check.text.as_str()), ("Check Box 2", "Urgent & new"));
    assert_eq!(check.link.as_ref().map(RefExpr::to_text).as_deref(), Some("$A$1"));
    assert_eq!(check.checked, CheckState::Checked);
    assert!(check.move_with_cells && !check.size_with_cells && !check.shaded && check.print);

    // Saved untouched: the shapes, the properties and the control
    // elements are those of the file, under the numbers of this file.
    let saved = save_xlsm(&wb);
    all_well_formed(&saved);
    let vml = part(&saved, "xl/drawings/vmlDrawing1.vml");
    assert!(vml.contains(BUTTON_SHAPE), "{vml}");
    assert!(vml.contains(CHECK_SHAPE), "{vml}");
    assert!(!vml.contains("Pict"), "the ActiveX shape is not kept");
    assert_eq!(part(&saved, "xl/ctrlProps/ctrlProp2.xml"), part(&original, "xl/ctrlProps/ctrlProp2.xml"));
    let xml = part(&saved, "xl/worksheets/sheet1.xml");
    assert!(
        xml.contains("<control shapeId=\"1025\" r:id=\"rId2\" name=\"Button 1\"><controlPr defaultSize=\"0\" print=\"0\" autoFill=\"0\" autoPict=\"0\" macro=\"[0]!Sheet1.Recalculate\" altText=\"Recalculate the sheet\"><anchor moveWithCells=\"1\" sizeWithCells=\"1\"><from><xdr:col>1</xdr:col>"),
        "{xml}"
    );
    assert_eq!(xml.matches("<control ").count(), 2);
    let names = part_names(&saved);
    assert!(names.iter().all(|name| !name.contains("activeX")), "{names:?}");
    assert!(names.iter().all(|name| !name.starts_with("xl/drawings/drawing")), "no twin is written: {names:?}");
    let rels = part(&saved, "xl/worksheets/_rels/sheet1.xml.rels");
    assert_eq!(rels.matches("<Relationship ").count(), 3, "{rels}");
    let back = load(&saved);
    assert_eq!(contents(&back, 0), contents(&wb, 0));
    assert_eq!(save_xlsm(&back), saved);

    // The linked cell changes: the check box is written again from the
    // model, the button is still the button of the file.
    let mut changed = load(&original);
    enter(&mut changed, 0, "A1", "FALSE");
    changed.commit("untick");
    let saved = save_xlsm(&changed);
    let vml = part(&saved, "xl/drawings/vmlDrawing1.vml");
    assert!(vml.contains(BUTTON_SHAPE) && !vml.contains("<x:Checked>"), "{vml}");
    assert!(vml.contains(">Urgent &amp; new</font>"));
    assert!(part(&saved, "xl/ctrlProps/ctrlProp2.xml").contains("objectType=\"CheckBox\" fmlaLink=\"$A$1\" lockText=\"1\" noThreeD=\"1\"/>"));
    assert_eq!(load(&saved).control_state(0, &load(&saved).sheets[0].controls[1]).checked, CheckState::Unchecked);

    // A row inserted above: both follow, and say where they are now.
    let mut moved = load(&original);
    moved.insert_rows(0, 0, 2);
    moved.commit("insert");
    let saved = save_xlsm(&moved);
    let vml = part(&saved, "xl/drawings/vmlDrawing1.vml");
    assert!(vml.contains("<x:Anchor>1, 0, 3, 0, 3, 0, 5, 0</x:Anchor>"), "{vml}");
    assert!(vml.contains("<x:FmlaMacro>[0]!Sheet1.Recalculate</x:FmlaMacro>") && vml.contains("<x:FmlaLink>$A$3</x:FmlaLink>"), "{vml}");
    let back = load(&saved);
    assert_eq!(back.sheets[0].controls[0].anchor, (3, 1));
    assert_eq!(back.value_at(0, 2, 0), Value::Bool(true));
}

/// A file of Excel 2007 (or of a tool that writes the shapes alone): the
/// controls are read from the VML drawing.
#[test]
fn controls_without_a_controls_block_are_read_from_their_shapes() {
    let types = format!(
        "<?xml version=\"1.0\"?><Types xmlns=\"{CT}\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Default Extension=\"vml\" ContentType=\"application/vnd.openxmlformats-officedocument.vmlDrawing\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/></Types>"
    );
    let root = format!("<Relationships xmlns=\"{PKG_REL}\"><Relationship Id=\"rId1\" Type=\"{DOC_REL}/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>");
    let workbook = format!("<workbook xmlns=\"{MAIN}\" xmlns:r=\"{DOC_REL}\"><sheets><sheet name=\"Data\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>");
    let workbook_rels = format!("<Relationships xmlns=\"{PKG_REL}\"><Relationship Id=\"rId1\" Type=\"{DOC_REL}/worksheet\" Target=\"worksheets/sheet1.xml\"/></Relationships>");
    let sheet = format!("<worksheet xmlns=\"{MAIN}\" xmlns:r=\"{DOC_REL}\"><sheetData/><legacyDrawing r:id=\"rId1\"/></worksheet>");
    let sheet_rels = format!("<Relationships xmlns=\"{PKG_REL}\"><Relationship Id=\"rId1\" Type=\"{DOC_REL}/vmlDrawing\" Target=\"../drawings/vmlDrawing1.vml\"/></Relationships>");
    let vml = format!("<xml xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:o=\"urn:schemas-microsoft-com:office:office\" xmlns:x=\"urn:schemas-microsoft-com:office:excel\">{BUTTON_SHAPE}{CHECK_SHAPE}</xml>");
    let original = zip(&[
        ("[Content_Types].xml", types.as_bytes()),
        ("_rels/.rels", root.as_bytes()),
        ("xl/workbook.xml", workbook.as_bytes()),
        ("xl/_rels/workbook.xml.rels", workbook_rels.as_bytes()),
        ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
        ("xl/worksheets/_rels/sheet1.xml.rels", sheet_rels.as_bytes()),
        ("xl/drawings/vmlDrawing1.vml", vml.as_bytes()),
    ]);
    let wb = load(&original);
    let names: Vec<&str> = wb.sheets[0].controls.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Button 1", "Check Box 2"]);
    assert_eq!(wb.sheets[0].controls[0].macro_name.as_deref(), Some("[0]!Sheet1.Recalculate"));
    // Saved, they get the parts Excel gives them today, and keep their shapes.
    let saved = save_xlsx(&wb);
    all_well_formed(&saved);
    assert!(part(&saved, "xl/drawings/vmlDrawing1.vml").contains(BUTTON_SHAPE));
    assert!(part(&saved, "xl/ctrlProps/ctrlProp1.xml").contains("objectType=\"Button\" lockText=\"1\"/>"));
    assert!(part(&saved, "xl/worksheets/sheet1.xml").contains("<control shapeId=\"1025\" r:id=\"rId2\" name=\"Button 1\"><controlPr defaultSize=\"0\" print=\"0\" autoFill=\"0\" autoPict=\"0\" macro=\"[0]!Sheet1.Recalculate\">"));
    assert_eq!(contents(&load(&saved), 0), contents(&wb, 0));
}
