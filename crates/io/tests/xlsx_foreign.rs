//! Reading packages laid out the way other applications write them:
//! shared strings, theme and indexed colours, built-in number formats,
//! shared and array formulas, parts the engine has no model for.

mod common;

use common::{at, formula, package, value};
use opensheet_engine::condfmt::Rule;
use opensheet_engine::page::{Margins, PageSetup};
use opensheet_engine::style::{Border, BorderStyle, HAlign, Underline, VAlign};
use opensheet_engine::workbook::Content;
use opensheet_engine::{Color, ErrorKind, Expr, Locale, Rect, StyleId, Value, Workbook};
use opensheet_io::read_xlsx;
use std::io::Cursor;

const MAIN: &str = "xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/></Types>"#;

fn root_rels() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId3" Type="{REL}/extended-properties" Target="docProps/app.xml"/><Relationship Id="rId1" Type="{REL}/officeDocument" Target="xl/workbook.xml"/></Relationships>"#
    )
}

fn workbook_xml() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook {MAIN} xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="x15">
<fileVersion appName="xl" lastEdited="7" lowestEdited="7" rupBuild="27425"/>
<workbookPr defaultThemeVersion="166925"/>
<mc:AlternateContent><mc:Choice Requires="x15"><x15ac:absPath url="C:\tmp\" xmlns:x15ac="x"/></mc:Choice></mc:AlternateContent>
<bookViews><workbookView xWindow="-120" yWindow="-120" windowWidth="29040" windowHeight="15840" activeTab="1"/></bookViews>
<sheets>
  <sheet name="First" sheetId="1" r:id="rId1"/>
  <sheet name="Chart1" sheetId="3" r:id="rId2"/>
  <sheet name="Second &amp; Third" sheetId="2" state="hidden" r:id="rId3"/>
</sheets>
<definedNames>
  <definedName name="_xlnm.Print_Area" localSheetId="0">First!$A$1:$F$12</definedName>
  <definedName name="_xlnm._FilterDatabase" localSheetId="0" hidden="1">First!$A$1:$B$5</definedName>
  <definedName name="Local" localSheetId="1">'Second &amp; Third'!$A$1</definedName>
  <definedName name="Rate">0.2</definedName>
  <definedName name="rate" localSheetId="0">99</definedName>
  <definedName name="Total">SUM(First!$A$3:$A$5)</definedName>
  <definedName name="Broken">[1]Ext!$A$1</definedName>
</definedNames>
<calcPr calcId="191029"/>
<extLst><ext uri="x"><sheets><sheet name="Ghost" sheetId="9" r:id="rId1"/></sheets></ext></extLst>
</workbook>"#
    )
}

fn workbook_rels() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId6" Type="{REL}/sharedStrings" Target="sharedStrings.xml"/>
<Relationship Id="rId5" Type="{REL}/styles" Target="/xl/styles.xml"/>
<Relationship Id="rId4" Type="{REL}/theme" Target="theme/theme1.xml"/>
<Relationship Id="rId3" Type="{REL}/worksheet" Target="/xl/worksheets/sheet2.xml"/>
<Relationship Id="rId2" Type="{REL}/chartsheet" Target="chartsheets/sheet1.xml"/>
<Relationship Id="rId1" Type="{REL}/worksheet" Target="worksheets/sheet1.xml"/>
<Relationship Id="rId7" Type="{REL}/externalLink" Target="externalLinks/externalLink1.xml"/>
<Relationship Id="rId8" Type="{REL}/hyperlink" Target="https://example.com/worksheets/sheet1.xml" TargetMode="External"/>
</Relationships>"#
    )
}

/// The Office theme with accent 2 changed, to prove the part is read.
const THEME: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<a:theme xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" name="Office Theme"><a:themeElements><a:clrScheme name="Office">
<a:dk1><a:sysClr val="windowText" lastClr="000000"/></a:dk1><a:lt1><a:sysClr val="window" lastClr="FFFFFF"/></a:lt1>
<a:dk2><a:srgbClr val="44546A"/></a:dk2><a:lt2><a:srgbClr val="E7E6E6"/></a:lt2>
<a:accent1><a:srgbClr val="4472C4"/></a:accent1><a:accent2><a:srgbClr val="112233"/></a:accent2>
<a:accent3><a:srgbClr val="A5A5A5"/></a:accent3><a:accent4><a:srgbClr val="FFC000"/></a:accent4>
<a:accent5><a:srgbClr val="5B9BD5"/></a:accent5><a:accent6><a:srgbClr val="70AD47"/></a:accent6>
<a:hlink><a:srgbClr val="0563C1"/></a:hlink><a:folHlink><a:srgbClr val="954F72"/></a:folHlink>
</a:clrScheme><a:fontScheme name="Office"><a:majorFont><a:latin typeface="Calibri Light"/></a:majorFont></a:fontScheme></a:themeElements></a:theme>"#;

fn styles_xml() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet {MAIN}>
<numFmts count="2"><numFmt numFmtId="164" formatCode="0.000"/><numFmt numFmtId="165" formatCode="[$-409]d\-mmm\-yy;@"/></numFmts>
<fonts count="4" x14ac:knownFonts="1" xmlns:x14ac="x">
  <font><sz val="11"/><color theme="1"/><name val="Calibri"/><family val="2"/><scheme val="minor"/></font>
  <font><b/><i val="0"/><strike/><u val="double"/><sz val="14"/><color rgb="FFFF0000"/><name val="Arial"/><family val="2"/></font>
  <font><i/><u/><sz val="11"/><color theme="4" tint="-0.249977111117893"/><name val="Calibri"/></font>
  <font><sz val="10.5"/><color indexed="10"/><name val="Calibri"/></font>
</fonts>
<fills count="6">
  <fill><patternFill patternType="none"/></fill>
  <fill><patternFill patternType="gray125"/></fill>
  <fill><patternFill patternType="solid"><fgColor rgb="FFFFFF00"/><bgColor indexed="64"/></patternFill></fill>
  <fill><patternFill patternType="solid"><fgColor theme="4" tint="0.79998168889431442"/><bgColor indexed="64"/></patternFill></fill>
  <fill><patternFill patternType="solid"><fgColor theme="0" tint="-0.249977111117893"/></patternFill></fill>
  <fill><gradientFill degree="90"><stop position="0"><color rgb="FF00FF00"/></stop><stop position="1"><color theme="4"/></stop></gradientFill></fill>
</fills>
<borders count="2">
  <border><left/><right/><top/><bottom/><diagonal/></border>
  <border diagonalUp="1"><left style="thin"><color auto="1"/></left><right style="medium"><color rgb="FF00B050"/></right><top style="double"><color theme="5"/></top><bottom style="mediumDashed"><color indexed="64"/></bottom><diagonal style="thin"><color rgb="FFFF0000"/></diagonal></border>
</borders>
<cellStyleXfs count="2"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/><xf numFmtId="9" fontId="1" fillId="2" borderId="1"/></cellStyleXfs>
<cellXfs count="10">
  <xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/>
  <xf numFmtId="14" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/>
  <xf numFmtId="9" fontId="0" fillId="0" borderId="0" xfId="1" applyFont="1"/>
  <xf numFmtId="10" fontId="0" fillId="0" borderId="0" xfId="0"/>
  <xf numFmtId="4" fontId="0" fillId="0" borderId="0" xfId="0"/>
  <xf numFmtId="44" fontId="0" fillId="0" borderId="0" xfId="0"/>
  <xf numFmtId="164" fontId="1" fillId="2" borderId="1" xfId="0" applyAlignment="1" applyProtection="1"><alignment horizontal="center" vertical="top" wrapText="1" indent="2" textRotation="135"/><protection locked="0" hidden="1"/></xf>
  <xf numFmtId="165" fontId="2" fillId="3" borderId="0" xfId="0"/>
  <xf numFmtId="0" fontId="3" fillId="4" borderId="0" xfId="0"><alignment horizontal="centerContinuous" vertical="center" textRotation="45" shrinkToFit="1"/></xf>
  <xf numFmtId="22" fontId="0" fillId="5" borderId="0" xfId="0"><alignment textRotation="255" vertical="distributed" horizontal="distributed"/></xf>
</cellXfs>
<cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>
<dxfs count="1"><dxf><font><b/><color rgb="FF9C0006"/></font><numFmt numFmtId="170" formatCode="0.0"/><fill><patternFill><bgColor rgb="FFFFC7CE"/></patternFill></fill><border><left style="thick"/></border></dxf></dxfs>
<tableStyles count="0" defaultTableStyle="TableStyleMedium2" defaultPivotStyle="PivotStyleLight16"/>
<colors><indexedColors><rgbColor rgb="00000000"/><rgbColor rgb="00FFFFFF"/><rgbColor rgb="00FF0000"/><rgbColor rgb="0000FF00"/><rgbColor rgb="000000FF"/><rgbColor rgb="00FFFF00"/><rgbColor rgb="00FF00FF"/><rgbColor rgb="0000FFFF"/><rgbColor rgb="00000000"/><rgbColor rgb="00FFFFFF"/><rgbColor rgb="00ABCDEF"/></indexedColors><mruColors><color rgb="FF123456"/></mruColors></colors>
<extLst><ext uri="x"><fonts><font><name val="Ghost"/></font></fonts></ext></extLst>
</styleSheet>"#
    )
}

const SHARED_STRINGS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count="4" uniqueCount="3"><si><t>Name</t></si><si><r><rPr><b/><sz val="11"/><rFont val="Calibri"/></rPr><t xml:space="preserve">bold </t></r><r><t>tail</t></r><rPh sb="0" eb="1"><t>phonetic</t></rPh><phoneticPr fontId="1"/></si><si><t xml:space="preserve"> second
sheet </t></si></sst>"#;

fn sheet1_xml() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet {MAIN} xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="x14ac" xmlns:x14ac="http://schemas.microsoft.com/office/spreadsheetml/2009/9/ac">
<sheetPr><tabColor theme="4" tint="-0.249977111117893"/><pageSetUpPr fitToPage="1"/></sheetPr>
<dimension ref="A1:F15"/>
<sheetViews><sheetView showGridLines="0" tabSelected="1" workbookViewId="0"><pane xSplit="1" ySplit="2" topLeftCell="B3" activePane="bottomRight" state="frozen"/><selection pane="topRight" activeCell="B1" sqref="B1"/><selection pane="bottomLeft" activeCell="A3" sqref="A3"/><selection pane="bottomRight" activeCell="C7" sqref="C7"/></sheetView><sheetView workbookViewId="1"><pane xSplit="5" ySplit="5" state="frozen"/></sheetView></sheetViews>
<sheetFormatPr defaultRowHeight="15" x14ac:dyDescent="0.25"/>
<cols><col min="1" max="1" width="20.7109375" customWidth="1"/><col min="3" max="4" width="9.140625" style="2" hidden="1"/><col min="6" max="16384" width="12" style="4"/></cols>
<sheetData>
<row r="1" spans="1:6" ht="30" customHeight="1" x14ac:dyDescent="0.25">
  <c r="A1" t="s"><v>0</v></c>
  <c r="B1" s="6" t="s"><v>1</v></c>
  <c r="C1" t="inlineStr"><is><t>inline</t></is></c>
  <c r="D1" t="b"><v>1</v></c>
  <c r="E1" t="e"><v>#DIV/0!</v></c>
  <c r="F1" s="1"><v>45351</v></c>
</row>
<row r="2" hidden="1"><c r="A2"><v>1.5</v></c><c r="B2"><v>2</v></c><c r="C2"><f>A2+B2</f><v>3.5</v></c><c r="D2" t="str"><f>A1&amp;"!"</f><v>Name!</v></c><c r="E2" s="7"/></row>
<row r="3" s="8" customFormat="1" ht="18.75"><c r="A3" s="8"><v>10</v></c><c r="B3" s="8"><f t="shared" ref="B3:B5" si="0">A3*$A$2</f><v>15</v></c><c r="C3" s="8"/></row>
<row r="4"><c r="A4"><v>20</v></c><c r="B4"><f t="shared" si="0"/><v>30</v></c></row>
<row r="5" ht="15"><c r="A5"><v>30</v></c><c r="B5"><f t="shared" si="0"/><v>45</v></c></row>
<row r="6"><c r="A6"><f>[1]Other!A1*2</f><v>99</v></c><c r="B6" t="str"><f>Table1[[#This Row],[x]]</f><v>kept</v></c><c r="C6"><f t="dataTable" ref="C6:C7" dt2D="0" dtr="0" r1="A2"/><v>7</v></c></row>
<row r="7"><c r="D7" cm="1"><f t="array" ref="D7:D9">A3:A5*2</f><v>20</v></c><c r="E7"><f>_xlfn.XLOOKUP(20,A3:A5,B3:B5)</f><v>30</v></c><c r="F7" t="e"><v>#GETTING_DATA</v></c></row>
<row r="8"><c r="D8" s="7"><v>40</v></c></row>
<row r="9"><c r="D9"><v>60</v></c><c r="E9" s="9"><v>1.25E-2</v></c></row>
<row r="11"><c><v>7</v></c><c><v>8</v></c><c s="3"/></row>
<row><c t="n"><v>9</v></c><c t="str"><v>literal_x000A_text</v></c></row>
</sheetData>
<sheetProtection sheet="1" objects="1" scenarios="1"/>
<autoFilter ref="A1:B5"/>
<mergeCells count="3"><mergeCell ref="A14:C15"/><mergeCell ref="B15:D16"/><mergeCell ref="E14"/></mergeCells>
<conditionalFormatting sqref="A1:A5"><cfRule type="cellIs" dxfId="0" priority="1" operator="greaterThan"><formula>5</formula></cfRule></conditionalFormatting>
<dataValidations count="1"><dataValidation type="list" sqref="B1"><formula1>"a,b"</formula1></dataValidation></dataValidations>
<hyperlinks><hyperlink ref="A1" r:id="rId1"/></hyperlinks>
<pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/>
<drawing r:id="rId2"/>
<extLst><ext uri="x"><x14:data xmlns:x14="x"><sheetData><row r="99"><c r="A99"><v>1</v></c></row></sheetData><mergeCells><mergeCell ref="X1:Y2"/></mergeCells></x14:data></ext></extLst>
</worksheet>"#
    )
}

const SHEET2: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<x:worksheet xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<x:sheetViews><x:sheetView showRowColHeaders="0" workbookViewId="0"><x:pane xSplit="2000" ySplit="1000" state="split"/></x:sheetView></x:sheetViews>
<x:sheetFormatPr defaultColWidth="12" defaultRowHeight="15"/>
<x:cols><x:col min="2" max="2" width="9.140625"/><x:col min="5" max="5" width="0" hidden="1" customWidth="1"/></x:cols>
<x:sheetData>
<x:row r="1"><x:c r="A1" t="s"><x:v>2</x:v></x:c><x:c r="C1"><x:f>First!A3+Rate</x:f><x:v>10.2</x:v></x:c></x:row>
<x:row r="2"><x:c r="B2" t="d"><x:v>2024-02-29T12:00:00Z</x:v></x:c><x:c r="F2"><x:f>Total</x:f><x:v>60</x:v></x:c></x:row>
</x:sheetData>
</x:worksheet>"#;

fn excel_like() -> Vec<u8> {
    package(&[
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", &root_rels()),
        ("docProps/app.xml", "<Properties/>"),
        ("xl/workbook.xml", &workbook_xml()),
        ("xl/_rels/workbook.xml.rels", &workbook_rels()),
        ("xl/theme/theme1.xml", THEME),
        ("xl/styles.xml", &styles_xml()),
        ("xl/sharedStrings.xml", SHARED_STRINGS),
        ("xl/worksheets/sheet1.xml", &sheet1_xml()),
        ("xl/worksheets/sheet2.xml", SHEET2),
        ("xl/chartsheets/sheet1.xml", "<chartsheet/>"),
    ])
}

fn load(bytes: &[u8], locale: &'static Locale) -> Workbook {
    read_xlsx(Cursor::new(bytes), locale).expect("the package loads")
}

#[test]
fn sheets_names_and_views() {
    let wb = load(&excel_like(), Locale::en());
    assert!(!wb.modified && !wb.can_undo());
    // The chart sheet is dropped; a hidden sheet is shown (no engine model).
    let names: Vec<&str> = wb.sheets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["First", "Second & Third"]);

    let first = &wb.sheets[0];
    assert_eq!(
        first.tab_color.map(Color::plain),
        Some(Color(0x305496)),
        "accent 1, darker 25 %"
    );
    assert!(!first.show_gridlines && first.show_headers);
    assert_eq!(
        (first.frozen_rows, first.frozen_cols),
        (2, 1),
        "only the first view counts"
    );
    assert_eq!(
        first.merges,
        vec![Rect::parse("A14:C15").unwrap()],
        "overlapping and 1x1 merges are dropped"
    );
    let second = &wb.sheets[1];
    assert!(second.show_gridlines && !second.show_headers);
    assert_eq!(
        (second.frozen_rows, second.frozen_cols),
        (0, 0),
        "a split is not a freeze"
    );
    assert_eq!(second.tab_color, None);

    // Workbook-scoped names win; built-in and unparseable names are dropped.
    let keys: Vec<&str> = wb.names.keys().map(|k| k.as_str()).collect();
    assert_eq!(keys, ["LOCAL", "RATE", "TOTAL"]);
    assert_eq!(wb.names["RATE"].name, "Rate");
    assert_eq!(wb.names["RATE"].expr.to_text(Locale::en()), "0.2");
    assert_eq!(
        wb.names["TOTAL"].expr.to_text(Locale::en()),
        "SUM(First!$A$3:$A$5)"
    );
    assert_eq!(
        wb.names["LOCAL"].expr.to_text(Locale::en()),
        "'Second & Third'!$A$1"
    );
}

#[test]
fn geometry() {
    let wb = load(&excel_like(), Locale::en());
    let first = &wb.sheets[0];
    // Calibri 11: columns of 64 pixels, rows of 20.
    assert_eq!(
        (first.default_col_width, first.default_row_height),
        (64.0, 20.0)
    );
    assert_eq!(first.col_widths.get(&0), Some(&145.0));
    assert_eq!(
        first.col_widths.get(&2),
        None,
        "a default-width column stays default"
    );
    assert!(first.hidden_cols.contains(&2) && first.hidden_cols.contains(&3));
    assert_eq!(first.hidden_cols.len(), 2);
    assert_eq!(first.col_widths.get(&5), Some(&84.0));
    assert_eq!(
        first.col_widths.get(&16383),
        Some(&84.0),
        "column ranges are expanded"
    );
    assert_eq!(first.col_widths.len(), 1 + 16379);
    assert_eq!(first.row_heights.get(&0), Some(&40.0));
    assert_eq!(
        first.row_heights.get(&2),
        Some(&25.0),
        "an auto-fitted height that is not the default"
    );
    assert_eq!(first.row_heights.len(), 2, "a 15 pt row is a default row");
    assert_eq!(first.hidden_rows.iter().copied().collect::<Vec<_>>(), [1]);

    // The second sheet declares its own default column width (12 → 84 px);
    // there a column of the usual 64 pixels is a column with a width.
    let second = &wb.sheets[1];
    assert_eq!(second.default_col_width, 84.0);
    let widths: Vec<(u32, f32)> = second.col_widths.iter().map(|(c, w)| (*c, *w)).collect();
    assert_eq!(widths, [(1, 64.0)]);
    assert_eq!(second.col_width(0), 84.0);
    assert_eq!(second.col_width(200), 84.0);
    assert_eq!(second.hidden_cols.iter().copied().collect::<Vec<_>>(), [4]);
    assert_eq!(second.col_width(4), 0.0);
}

#[test]
fn values_and_formulas() {
    let wb = load(&excel_like(), Locale::en());
    assert_eq!(value(&wb, 0, "A1"), Value::text("Name"));
    assert_eq!(
        value(&wb, 0, "B1"),
        Value::text("bold tail"),
        "rich text runs are joined, phonetics skipped"
    );
    assert_eq!(value(&wb, 0, "C1"), Value::text("inline"));
    assert_eq!(value(&wb, 0, "D1"), Value::Bool(true));
    assert_eq!(value(&wb, 0, "E1"), Value::Error(ErrorKind::Div0));
    assert_eq!(value(&wb, 0, "F1"), Value::Number(45351.0));
    assert_eq!(value(&wb, 0, "A2"), Value::Number(1.5));
    assert_eq!(formula(&wb, 0, "C2").unwrap(), "A2+B2");
    assert_eq!(value(&wb, 0, "C2"), Value::Number(3.5));
    assert_eq!(formula(&wb, 0, "D2").unwrap(), "A1&\"!\"");
    assert_eq!(value(&wb, 0, "D2"), Value::text("Name!"));

    // Shared formulas are expanded relative to their master cell.
    assert_eq!(formula(&wb, 0, "B3").unwrap(), "A3*$A$2");
    assert_eq!(formula(&wb, 0, "B4").unwrap(), "A4*$A$2");
    assert_eq!(formula(&wb, 0, "B5").unwrap(), "A5*$A$2");
    assert_eq!(value(&wb, 0, "B5"), Value::Number(45.0));

    // Formulas the engine cannot parse keep their cached result.
    assert_eq!(formula(&wb, 0, "A6"), None);
    assert_eq!(value(&wb, 0, "A6"), Value::Number(99.0));
    assert_eq!(value(&wb, 0, "B6"), Value::text("kept"));
    assert_eq!(value(&wb, 0, "C6"), Value::Number(7.0), "data table result");

    // An array formula spills again instead of colliding with its own
    // cached results.
    assert_eq!(formula(&wb, 0, "D7").unwrap(), "A3:A5*2");
    assert_eq!(value(&wb, 0, "D7"), Value::Number(20.0));
    assert_eq!(value(&wb, 0, "D8"), Value::Number(40.0));
    assert_eq!(value(&wb, 0, "D9"), Value::Number(60.0));
    // The cached results are gone as constants; what remains of those cells
    // is their formatting (which is not that of their column).
    for name in ["D8", "D9"] {
        let (r, c) = at(name);
        assert!(
            matches!(wb.cell(0, r, c).unwrap().content, Content::Empty),
            "{name} is not a constant"
        );
    }
    assert_eq!(
        wb.style_at(0, 7, 3).fill.map(Color::plain),
        Some(Color(0xD9E1F2))
    );
    assert_eq!(wb.sheets[0].spills.len(), 1);

    assert_eq!(formula(&wb, 0, "E7").unwrap(), "XLOOKUP(20,A3:A5,B3:B5)");
    assert_eq!(
        value(&wb, 0, "F7"),
        Value::Error(ErrorKind::Value),
        "unknown error codes degrade to #VALUE!"
    );
    assert_eq!(value(&wb, 0, "E9"), Value::Number(0.0125));

    // Cells and rows without an address follow their predecessor.
    assert_eq!(value(&wb, 0, "A11"), Value::Number(7.0));
    assert_eq!(value(&wb, 0, "B11"), Value::Number(8.0));
    assert_eq!(value(&wb, 0, "A12"), Value::Number(9.0));
    assert_eq!(value(&wb, 0, "B12"), Value::text("literal\ntext"));
    // Nothing leaks out of extension lists.
    assert_eq!(value(&wb, 0, "A99"), Value::Empty);

    // Second sheet: prefixed element names, cross-sheet formula, names, ISO dates.
    assert_eq!(value(&wb, 1, "A1"), Value::text(" second\nsheet "));
    assert_eq!(formula(&wb, 1, "C1").unwrap(), "First!A3+Rate");
    assert_eq!(value(&wb, 1, "C1"), Value::Number(10.2));
    assert_eq!(value(&wb, 1, "B2"), Value::Number(45351.5));
    assert_eq!(value(&wb, 1, "F2"), Value::Number(60.0));
}

#[test]
fn styles() {
    let wb = load(&excel_like(), Locale::en());
    let style = |name: &str| {
        let (r, c) = at(name);
        wb.style_at(0, r, c).clone()
    };

    // The workbook's default font is kept; plain "text 1" is automatic.
    let plain = style("A1");
    assert_eq!(
        (plain.font.name.as_ref(), plain.font.size, plain.font.color),
        ("Calibri", 22, None)
    );
    // It is the font of the cells nobody formatted, used or not.
    let (r, c) = at("A1");
    assert_eq!(wb.cell(0, r, c).unwrap().style, StyleId(0));
    assert_eq!(wb.styles.default_style(), &plain);
    assert_eq!(wb.style_at(1, 500, 500), &plain);
    assert_eq!(plain.num_fmt.as_ref(), "General");
    assert!(plain.locked && plain.fill.is_none() && plain.borders.is_none());

    // Built-in number format ids.
    assert_eq!(style("F1").num_fmt.as_ref(), "m/d/yyyy");
    assert_eq!(wb.style_at(0, 50, 2).num_fmt.as_ref(), "0%", "column style");
    assert!(wb.sheets[0].col_styles.contains_key(&3));
    assert_eq!(style("C11").num_fmt.as_ref(), "0.00%");
    assert_eq!(
        wb.style_at(0, 50, 5).num_fmt.as_ref(),
        "#,##0.00",
        "style of columns F:XFD"
    );
    assert_eq!(wb.style_at(0, 50, 16383).num_fmt.as_ref(), "#,##0.00");
    assert_eq!(style("E9").num_fmt.as_ref(), "m/d/yyyy h:mm");
    let french = load(&excel_like(), Locale::fr());
    assert_eq!(
        french.style_at(0, 0, 5).num_fmt.as_ref(),
        "dd/mm/yyyy",
        "id 14 is the reader's short date"
    );

    let b1 = style("B1");
    assert_eq!(b1.num_fmt.as_ref(), "0.000");
    assert_eq!((b1.font.name.as_ref(), b1.font.size), ("Arial", 28));
    assert!(b1.font.bold && !b1.font.italic && b1.font.strike);
    assert_eq!(b1.font.underline, Underline::Double);
    assert_eq!(b1.font.color, Some(Color(0xFF0000)));
    assert_eq!(b1.fill, Some(Color(0xFFFF00)));
    assert_eq!(
        (b1.h_align, b1.v_align, b1.wrap, b1.indent),
        (HAlign::Center, VAlign::Top, true, 2)
    );
    assert_eq!(b1.rotation, -45, "135 is 45 degrees clockwise");
    assert!(!b1.locked);
    assert_eq!(b1.borders.left, Border::thin());
    assert_eq!(
        b1.borders.right,
        Border {
            style: BorderStyle::Medium,
            color: Some(Color(0x00B050))
        }
    );
    assert_eq!(
        b1.borders.top,
        Border {
            style: BorderStyle::Double,
            color: Some(wb.theme().accent(2, 0.0))
        },
        "theme colour 5 is accent 2 of this file's theme"
    );
    assert_eq!(b1.borders.top.color.map(|c| c.value()), Some(0x112233));
    assert_eq!(
        b1.borders.bottom,
        Border {
            style: BorderStyle::Dashed,
            color: None
        },
        "indexed 64 is automatic"
    );

    // Theme colours with tints.
    let e2 = style("E2");
    assert_eq!(e2.font.color.map(Color::plain), Some(Color(0x305496)));
    assert!(e2.font.italic);
    assert_eq!(e2.font.underline, Underline::Single);
    assert_eq!(
        e2.fill.map(Color::plain),
        Some(Color(0xD9E1F2)),
        "accent 1, lighter 80 %, as Excel shades it"
    );
    assert_eq!(e2.num_fmt.as_ref(), "[$-409]d\\-mmm\\-yy;@");
    let (r, c) = at("E2");
    assert!(
        matches!(wb.cell(0, r, c).unwrap().content, Content::Empty),
        "formatted empty cell"
    );

    // Row style, custom indexed palette, alignment variants.
    let a3 = style("A3");
    assert_eq!(
        wb.sheets[0].row_styles.get(&2).map(|id| wb.styles.get(*id)),
        Some(&a3)
    );
    assert_eq!(a3.font.size, 21);
    assert_eq!(
        a3.font.color,
        Some(Color(0xABCDEF)),
        "indexed 10 is redefined by the file"
    );
    assert_eq!(
        a3.fill.map(Color::plain),
        Some(Color(0xBFBFBF)),
        "white, darker 25 %"
    );
    assert_eq!(
        (a3.h_align, a3.v_align),
        (HAlign::CenterAcross, VAlign::Center)
    );
    assert_eq!((a3.rotation, a3.shrink_to_fit), (45, true));
    let (r, c) = at("C3");
    assert!(
        wb.cell(0, r, c).is_none(),
        "an empty cell repeating its row style is not stored"
    );
    let e9 = style("E9");
    assert_eq!(e9.fill, Some(Color(0x00FF00)), "first stop of a gradient");
    assert_eq!(
        (e9.rotation, e9.h_align, e9.v_align),
        (0, HAlign::Justify, VAlign::Center)
    );
}

#[test]
fn conditional_formats_and_page_setup() {
    let wb = load(&excel_like(), Locale::en());
    let first = &wb.sheets[0];
    // The rule takes its format from the differential formats of the
    // style sheet.
    assert_eq!(first.cond_formats.len(), 1);
    let cf = &first.cond_formats[0];
    assert_eq!(cf.ranges, [Rect::parse("A1:A5").unwrap()]);
    assert_eq!(cf.rule, Rule::greater_than(Expr::Number(5.0)));
    assert!(!cf.stop_if_true);
    assert_eq!(cf.format.bold, Some(true));
    assert_eq!(cf.format.font_color, Some(Some(Color(0x9C0006))));
    assert_eq!(cf.format.fill, Some(Some(Color(0xFFC7CE))));
    assert_eq!(cf.format.num_fmt.as_deref(), Some("0.0"));
    assert_eq!(
        cf.format.border_left,
        Some(Border {
            style: BorderStyle::Thick,
            color: None
        })
    );
    assert_eq!(
        cf.format.border_right, None,
        "a side the format says nothing about"
    );

    // The page: one flag of the sheet properties, the margins, and the
    // print area, which the file keeps among the names.
    let page = PageSetup {
        fit_to_page: true,
        print_area: vec![Rect::parse("A1:F12").unwrap()],
        ..PageSetup::default()
    };
    assert_eq!(first.page, page);
    assert_eq!(first.page.margins, Margins::NORMAL);
    assert_eq!(wb.sheets[1].page, PageSetup::default());
    assert!(wb.sheets[1].cond_formats.is_empty());
}

#[test]
fn minimal_package_without_relationships() {
    // No content types, no relationship parts, no styles, no strings.
    let bytes = package(&[
        (
            "xl/workbook.xml",
            r#"<workbook><sheets><sheet name="Only" sheetId="1" r:id="rId1"/><sheet name="only" sheetId="2"/><sheet name="" sheetId="3"/></sheets></workbook>"#,
        ),
        (
            "xl/worksheets/sheet1.xml",
            r#"<worksheet><sheetData><row r="2"><c r="B2" s="5"><v>3</v></c><c r="C2" t="s"><v>0</v></c><c r="D2"><v>oops</v></c></row></sheetData></worksheet>"#,
        ),
        (
            "XL/Worksheets/Sheet2.xml",
            r#"<worksheet><sheetData><row><c><f>Only!B2*2</f></c></row></sheetData></worksheet>"#,
        ),
        (
            "xl/worksheets/sheet3.xml",
            "<worksheet><sheetData/></worksheet>",
        ),
    ]);
    let wb = load(&bytes, Locale::en());
    let names: Vec<&str> = wb.sheets.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(
        names,
        ["Only", "only (2)", "Sheet3"],
        "names are made unique and non-empty"
    );
    assert_eq!(value(&wb, 0, "B2"), Value::Number(3.0));
    assert_eq!(
        value(&wb, 0, "C2"),
        Value::Empty,
        "a dangling shared string index"
    );
    assert_eq!(value(&wb, 0, "D2"), Value::text("oops"));
    assert_eq!(value(&wb, 1, "A1"), Value::Number(6.0));
    assert_eq!(wb.style_at(0, 1, 1), wb.styles.get(Default::default()));
}

#[test]
fn date_1904_workbooks_are_shifted() {
    let workbook = r#"<workbook><workbookPr date1904="1"/><sheets><sheet name="S" sheetId="1" r:id="rId1"/></sheets></workbook>"#;
    let styles = r#"<styleSheet><cellXfs count="3"><xf numFmtId="0"/><xf numFmtId="14"/><xf numFmtId="20"/></cellXfs></styleSheet>"#;
    let sheet = r#"<worksheet><sheetData><row r="1"><c r="A1" s="1"><v>0</v></c><c r="B1"><v>5</v></c><c r="C1" s="2"><v>0.5</v></c><c r="D1" s="1"><f>A1+1</f><v>1</v></c></row></sheetData></worksheet>"#;
    let bytes = package(&[
        ("xl/workbook.xml", workbook),
        ("xl/styles.xml", styles),
        ("xl/worksheets/sheet1.xml", sheet),
    ]);
    let wb = load(&bytes, Locale::en());
    assert_eq!(
        value(&wb, 0, "A1"),
        Value::Number(1462.0),
        "1904-01-01 in the 1900 system"
    );
    assert_eq!(
        value(&wb, 0, "B1"),
        Value::Number(5.0),
        "plain numbers are untouched"
    );
    assert_eq!(
        value(&wb, 0, "C1"),
        Value::Number(0.5),
        "so are times of day"
    );
    assert_eq!(
        value(&wb, 0, "D1"),
        Value::Number(1463.0),
        "formulas are recalculated"
    );
}

#[test]
fn damaged_parts_do_not_panic() {
    let cases = [
        "<worksheet><sheetData><row r=\"1\"><c r=\"A1\"><v>1</v>",
        "<worksheet><sheetData><row r=\"0\"><c r=\"ZZZZ1\"><v>1</v></c><c r=\"A99999999\"><v>2</v></c></row></sheetData>",
        "<worksheet><cols><col min=\"0\" max=\"99999\" width=\"-3\"/><col min=\"9\" max=\"2\"/></cols><sheetData><row r=\"9999999\"><c><v>1</v></c></row></sheetData></worksheet>",
        "<worksheet><sheetData><row><c t=\"s\"><v>abc</v></c><c t=\"e\"><v></v></c><c t=\"b\"/><c><f t=\"shared\" si=\"7\"/></c><c><f t=\"array\" ref=\"nonsense\">1</f></c></row></sheetData><mergeCells><mergeCell ref=\"??\"/><mergeCell/></mergeCells></worksheet>",
        "<worksheet><sheetViews><sheetView><pane xSplit=\"-5\" ySplit=\"1e99\" state=\"frozen\"/></sheetView></sheetViews></worksheet>",
        "",
        "not xml at all <<<< >>>",
    ];
    for sheet in cases {
        let bytes = package(&[
            (
                "xl/workbook.xml",
                r#"<workbook><sheets><sheet name="S" r:id="rId1"/></sheets><definedNames><definedName name="X">((((</definedName><definedName>1</definedName></definedNames></workbook>"#,
            ),
            (
                "xl/styles.xml",
                "<styleSheet><fonts><font><sz val=\"x\"/><color rgb=\"zz\" theme=\"99\" tint=\"q\"/></font></fonts><cellXfs><xf fontId=\"42\" fillId=\"42\" borderId=\"42\" numFmtId=\"4000000000\"><alignment textRotation=\"999\" indent=\"100000\"/></xf>",
            ),
            ("xl/sharedStrings.xml", "<sst><si><t>unterminated"),
            ("xl/worksheets/sheet1.xml", sheet),
        ]);
        let wb = load(&bytes, Locale::en());
        assert_eq!(wb.sheets.len(), 1);
        assert!(wb.names.is_empty());
    }
}
