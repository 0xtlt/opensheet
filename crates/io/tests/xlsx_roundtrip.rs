//! Save → load round trips of workbooks built with the engine.

mod common;

use common::{assert_well_formed, at, enter, formula, part, part_names, value};
use opensheet_engine::parser::parse_formula;
use opensheet_engine::refs::{MAX_COLS, MAX_ROWS};
use opensheet_engine::style::{Border, BorderStyle, HAlign, Style, Underline, VAlign};
use opensheet_engine::workbook::Content;
use opensheet_engine::{Color, ErrorKind, Locale, Rect, Value, Workbook};
use opensheet_io::{load, load_xlsx, read_xlsx, save, save_xlsx, write_xlsx};
use std::io::Cursor;
use std::sync::Arc;

fn style(wb: &mut Workbook, sheet: usize, range: &str, f: &dyn Fn(&mut Style)) {
    wb.update_style(sheet, Rect::parse(range).unwrap(), f);
}

/// A workbook exercising everything the format carries.
fn sample() -> Workbook {
    let en = Locale::en();
    let mut wb = Workbook::new(en);
    wb.add_sheet(1);
    wb.add_sheet(2);
    assert!(wb.rename_sheet(0, "Summary"));
    assert!(wb.rename_sheet(1, "Data"));
    assert!(wb.rename_sheet(2, "Joe's & <Co>"));

    // Data
    for (i, (n, s)) in [
        (10.0, "apple"),
        (20.0, "pear"),
        (30.5, "fig"),
        (-4.0, "kiwi"),
        (2.5, "plum"),
    ]
    .into_iter()
    .enumerate()
    {
        wb.set_value(1, i as u32, 0, Value::Number(n));
        wb.set_value(1, i as u32, 1, Value::text(s));
    }
    // Numbers that need every digit, or an exponent, to survive.
    wb.set_value(1, 5, 3, Value::Number(123_456_789_012_345_680_000.0));
    wb.set_value(1, 6, 3, Value::Number(0.1 + 0.2));
    wb.set_value(1, 7, 3, Value::Number(1e-7));
    enter(&mut wb, 2, "A1", "7");
    enter(&mut wb, 2, "XFD1048576", "corner");

    // Constants of every type.
    enter(&mut wb, 0, "A1", "Title");
    enter(&mut wb, 0, "A2", "42");
    enter(&mut wb, 0, "A3", "TRUE");
    enter(&mut wb, 0, "A4", "#N/A");
    enter(&mut wb, 0, "A5", "3/12/2025");
    enter(&mut wb, 0, "A6", "50%");
    wb.set_value(
        0,
        6,
        0,
        Value::text("  <b> & \"q\"\nline 2\ttab\r_x000D_ \u{1} é€  "),
    );
    enter(&mut wb, 0, "A8", "'007");
    enter(&mut wb, 0, "A9", "apple");
    wb.set_value(0, 9, 0, Value::text(""));
    wb.set_value(0, 10, 0, Value::Bool(false));
    wb.set_value(0, 11, 0, Value::Error(ErrorKind::Div0));

    // Formulas: plain, cross-sheet, newer functions, every result type.
    enter(&mut wb, 0, "B1", "=A2*2");
    enter(&mut wb, 0, "B2", "=SUM(Data!A1:A5)");
    enter(&mut wb, 0, "B3", "=XLOOKUP(20,Data!A1:A5,Data!B1:B5)");
    enter(&mut wb, 0, "B4", "=IF(A3,\"yes & <no>\",\"no\")");
    enter(&mut wb, 0, "B5", "=1/0");
    enter(&mut wb, 0, "B6", "=A2>1");
    enter(&mut wb, 0, "B7", "='Joe''s & <Co>'!A1+Rate");
    enter(&mut wb, 0, "B8", "=SUM(Totals)+MAX($A$2,A$2,$A2)");
    enter(
        &mut wb,
        0,
        "B9",
        "=IFS(A2>1,\"big\",TRUE,\"small\")&TEXTJOIN(\",\",TRUE,Data!B1:B2)",
    );
    enter(
        &mut wb,
        0,
        "B10",
        "=SUM(FILTER(Data!A1:A5,Data!A1:A5>0))+STDEV.S(Data!A:A)",
    );
    enter(&mut wb, 0, "B11", "=-A2%+SUM({1,2;3,4})+SUM(1:1)");
    enter(&mut wb, 0, "B12", "=SUM(Data!A1:A5*(Data!A1:A5>15))");
    // A dynamic array formula spilling over D1:D3.
    enter(&mut wb, 0, "D1", "=Data!A1:A3*2");

    // Cell formatting.
    style(&mut wb, 0, "A1:B1", &|s| {
        s.font.name = Arc::from("Arial");
        s.font.set_size_pt(14.5);
        s.font.bold = true;
        s.font.italic = true;
        s.font.underline = Underline::Double;
        s.font.strike = true;
        s.font.color = Some(Color(0xC00000));
        s.fill = Some(Color(0xFFF2CC));
        s.h_align = HAlign::Center;
        s.v_align = VAlign::Top;
        s.wrap = true;
    });
    style(&mut wb, 0, "A2", &|s| {
        s.num_fmt = Arc::from("#,##0.00");
        s.h_align = HAlign::Right;
        s.indent = 2;
        s.font.underline = Underline::Single;
    });
    style(&mut wb, 0, "B2", &|s| {
        s.num_fmt = Arc::from("[Blue]0.000;[Red]-0.000;\"zero\"");
        s.rotation = 45;
        s.v_align = VAlign::Center;
        s.locked = false;
    });
    style(&mut wb, 0, "B5", &|s| {
        s.rotation = -30;
        s.shrink_to_fit = true;
        s.h_align = HAlign::CenterAcross;
    });
    style(&mut wb, 0, "A3:B4", &|s| {
        s.borders.left = Border::thin();
        s.borders.right = Border {
            style: BorderStyle::Medium,
            color: Some(Color(0x00B050)),
        };
        s.borders.top = Border {
            style: BorderStyle::Double,
            color: Some(Color(0x000000)),
        };
        s.borders.bottom = Border {
            style: BorderStyle::Dashed,
            color: None,
        };
    });
    style(&mut wb, 0, "C3", &|s| {
        s.borders.left = Border {
            style: BorderStyle::Hair,
            color: None,
        };
        s.borders.right = Border {
            style: BorderStyle::Thick,
            color: None,
        };
        s.borders.top = Border {
            style: BorderStyle::Dotted,
            color: Some(Color(0x123456)),
        };
        s.h_align = HAlign::Justify;
    });
    style(&mut wb, 0, "C4", &|s| {
        s.h_align = HAlign::Fill;
        s.num_fmt = Arc::from("@");
    });
    style(&mut wb, 0, "C5", &|s| s.h_align = HAlign::Left);
    // A formatted, empty cell under the spill of D1.
    style(&mut wb, 0, "D2", &|s| s.fill = Some(Color(0xDDEBF7)));
    // Whole column and whole row formatting.
    wb.update_style(0, Rect::new(0, 6, MAX_ROWS - 1, 7), &|s| s.font.bold = true);
    wb.update_style(0, Rect::new(14, 0, 14, MAX_COLS - 1), &|s| {
        s.fill = Some(Color(0xE2EFDA))
    });
    enter(&mut wb, 0, "G3", "in a bold column");

    // Geometry.
    wb.set_col_width(0, 0, Some(120.0));
    wb.set_col_width(0, 2, Some(33.0));
    wb.set_col_width(0, 9, Some(64.0));
    wb.set_col_width(0, 10, Some(64.0));
    wb.set_row_height(0, 0, Some(30.0));
    wb.set_row_height(0, 20, Some(41.0));
    wb.set_row_height(0, 21, Some(20.0));
    wb.sheets[0].hidden_cols.insert(4);
    wb.sheets[0].hidden_cols.insert(9);
    wb.sheets[0].hidden_rows.insert(7);
    wb.sheets[0].hidden_rows.insert(30);
    wb.sheets[0].merges.push(Rect::new(16, 0, 17, 2));
    wb.sheets[0].merges.push(Rect::new(18, 1, 18, 4));
    wb.set_frozen(0, 1, 2);
    wb.set_frozen(1, 3, 0);
    wb.set_frozen(2, 0, 1);
    wb.sheets[1].show_gridlines = false;
    wb.sheets[2].show_headers = false;
    wb.set_tab_color(0, Some(Color(0xFF6600)));
    wb.set_tab_color(2, Some(Color(0x0070C0)));

    wb.define_name("Rate", parse_formula("0.2", en).unwrap());
    wb.define_name("Totals", parse_formula("Data!$A$1:$A$5", en).unwrap());
    wb.define_name(
        "Newer",
        parse_formula("CONCAT(Data!$B$1,\"<&>\")", en).unwrap(),
    );
    wb.commit("setup");
    wb
}

/// Compare two workbooks cell by cell: content, calculated values,
/// formatting, geometry, names.
fn assert_same(a: &Workbook, b: &Workbook) {
    let en = Locale::en();
    assert_eq!(a.sheets.len(), b.sheets.len(), "sheet count");
    for (si, (sa, sb)) in a.sheets.iter().zip(&b.sheets).enumerate() {
        let name = &sa.name;
        assert_eq!(sa.name, sb.name);
        assert_eq!(sa.col_widths, sb.col_widths, "{name}: column widths");
        assert_eq!(sa.row_heights, sb.row_heights, "{name}: row heights");
        assert_eq!(sa.hidden_cols, sb.hidden_cols, "{name}: hidden columns");
        assert_eq!(sa.hidden_rows, sb.hidden_rows, "{name}: hidden rows");
        assert_eq!(sa.merges, sb.merges, "{name}: merges");
        assert_eq!(
            (sa.frozen_rows, sa.frozen_cols),
            (sb.frozen_rows, sb.frozen_cols),
            "{name}: frozen panes"
        );
        assert_eq!(sa.show_gridlines, sb.show_gridlines, "{name}: gridlines");
        assert_eq!(sa.show_headers, sb.show_headers, "{name}: headers");
        assert_eq!(sa.tab_color, sb.tab_color, "{name}: tab colour");

        assert_eq!(
            sa.col_styles.keys().collect::<Vec<_>>(),
            sb.col_styles.keys().collect::<Vec<_>>(),
            "{name}: styled columns"
        );
        for (c, id) in &sa.col_styles {
            assert_eq!(
                a.styles.get(*id),
                b.styles.get(sb.col_styles[c]),
                "{name}: style of column {c}"
            );
        }
        assert_eq!(
            sa.row_styles.keys().collect::<Vec<_>>(),
            sb.row_styles.keys().collect::<Vec<_>>(),
            "{name}: styled rows"
        );
        for (r, id) in &sa.row_styles {
            assert_eq!(
                a.styles.get(*id),
                b.styles.get(sb.row_styles[r]),
                "{name}: style of row {r}"
            );
        }

        assert_eq!(
            sa.cells.keys().collect::<Vec<_>>(),
            sb.cells.keys().collect::<Vec<_>>(),
            "{name}: stored cells"
        );
        for (pos, ca) in &sa.cells {
            let cb = &sb.cells[pos];
            let cell = format!("{name}!{}", Rect::cell(pos.0, pos.1).name());
            match (&ca.content, &cb.content) {
                (Content::Empty, Content::Empty) => {}
                (Content::Value(x), Content::Value(y)) => assert_eq!(x, y, "{cell}: constant"),
                (Content::Formula(x), Content::Formula(y)) => {
                    assert_eq!(x.to_text(en), y.to_text(en), "{cell}: formula");
                }
                (x, y) => panic!("{cell}: content {x:?} became {y:?}"),
            }
            assert_eq!(ca.value, cb.value, "{cell}: calculated value");
            assert_eq!(
                a.styles.get(ca.style),
                b.styles.get(cb.style),
                "{cell}: style"
            );
        }

        let spills = |wb: &Workbook| -> Vec<(u32, u32, usize, usize, Vec<Value>)> {
            let mut v: Vec<_> = wb.sheets[si]
                .spills
                .iter()
                .map(|sp| {
                    (
                        sp.row,
                        sp.col,
                        sp.array.rows,
                        sp.array.cols,
                        sp.array.data.clone(),
                    )
                })
                .collect();
            v.sort_by_key(|s| (s.0, s.1));
            v
        };
        assert_eq!(spills(a), spills(b), "{name}: spilled arrays");
    }
    assert_eq!(
        a.names.keys().collect::<Vec<_>>(),
        b.names.keys().collect::<Vec<_>>(),
        "names"
    );
    for (key, da) in &a.names {
        assert_eq!(da.name, b.names[key].name);
        assert_eq!(
            da.expr.to_text(en),
            b.names[key].expr.to_text(en),
            "definition of {key}"
        );
    }
}

fn to_bytes(wb: &Workbook) -> Vec<u8> {
    write_xlsx(wb, Cursor::new(Vec::new()))
        .unwrap()
        .into_inner()
}

#[test]
fn everything_round_trips() {
    let original = sample();
    // The sample is what it is meant to be before it goes anywhere.
    assert_eq!(value(&original, 0, "B1"), Value::Number(84.0));
    assert_eq!(value(&original, 0, "B2"), Value::Number(59.0));
    assert_eq!(value(&original, 0, "D3"), Value::Number(61.0));
    assert_eq!(original.sheets[0].spills.len(), 1);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("book.xlsx");
    save_xlsx(&original, &path).unwrap();
    let loaded = load_xlsx(&path, Locale::en()).unwrap();

    assert!(!loaded.modified, "a freshly loaded workbook is unmodified");
    assert!(
        !loaded.can_undo() && !loaded.can_redo(),
        "loading leaves no undo history"
    );
    assert_eq!(loaded.locale.id, "en");
    assert_same(&original, &loaded);

    // Spot checks against literal expectations.
    assert_eq!(value(&loaded, 0, "A2"), Value::Number(42.0));
    assert_eq!(value(&loaded, 0, "A3"), Value::Bool(true));
    assert_eq!(value(&loaded, 0, "A4"), Value::Error(ErrorKind::NA));
    assert_eq!(
        value(&loaded, 0, "A7"),
        Value::text("  <b> & \"q\"\nline 2\ttab\r_x000D_ \u{1} é€  ")
    );
    assert_eq!(value(&loaded, 0, "A8"), Value::text("007"));
    assert_eq!(value(&loaded, 0, "A10"), Value::text(""));
    assert_eq!(value(&loaded, 0, "B1"), Value::Number(84.0));
    assert_eq!(value(&loaded, 0, "B4"), Value::text("yes & <no>"));
    assert_eq!(value(&loaded, 0, "B5"), Value::Error(ErrorKind::Div0));
    assert_eq!(value(&loaded, 0, "B6"), Value::Bool(true));
    assert_eq!(value(&loaded, 0, "B7"), Value::Number(7.2));
    assert_eq!(value(&loaded, 0, "D2"), Value::Number(40.0));
    assert_eq!(
        value(&loaded, 1, "D6"),
        Value::Number(123_456_789_012_345_680_000.0)
    );
    assert_eq!(value(&loaded, 1, "D7"), Value::Number(0.1 + 0.2));
    assert_eq!(value(&loaded, 1, "D8"), Value::Number(1e-7));
    assert_eq!(
        formula(&loaded, 0, "B11").unwrap(),
        "-A2%+SUM({1,2;3,4})+SUM(1:1)"
    );
    assert_eq!(value(&loaded, 2, "XFD1048576"), Value::text("corner"));
    assert_eq!(
        formula(&loaded, 0, "B3").unwrap(),
        "XLOOKUP(20,Data!A1:A5,Data!B1:B5)"
    );
    assert_eq!(
        formula(&loaded, 0, "B7").unwrap(),
        "'Joe''s & <Co>'!A1+Rate"
    );
    assert_eq!(
        formula(&loaded, 0, "B10").unwrap(),
        "SUM(FILTER(Data!A1:A5,Data!A1:A5>0))+STDEV.S(Data!A:A)"
    );
    assert_eq!(formula(&loaded, 0, "D1").unwrap(), "Data!A1:A3*2");
    assert!(
        loaded.cell(0, 2, 3).is_none(),
        "spilled cells are not stored as constants"
    );
    let a1 = loaded.style_at(0, 0, 0);
    assert_eq!((a1.font.name.as_ref(), a1.font.size), ("Arial", 29));
    assert_eq!(a1.fill, Some(Color(0xFFF2CC)));
    assert_eq!(loaded.style_at(0, 4, 0).num_fmt.as_ref(), "m/d/yyyy");
    assert_eq!(loaded.style_at(0, 4, 1).rotation, -30);
    assert!(loaded.style_at(0, 100, 6).font.bold);
    assert_eq!(loaded.style_at(0, 14, 200).fill, Some(Color(0xE2EFDA)));
    assert_eq!(loaded.sheets[0].col_width(0), 120.0);
    assert_eq!(loaded.sheets[0].col_width(4), 0.0);
    assert_eq!(loaded.sheets[0].row_height(0), 30.0);

    // A second generation is identical to the first.
    let again = read_xlsx(Cursor::new(to_bytes(&loaded)), Locale::en()).unwrap();
    assert_same(&loaded, &again);
}

#[test]
fn package_is_a_valid_zip_of_well_formed_parts() {
    let bytes = to_bytes(&sample());
    let names = part_names(&bytes);
    assert_eq!(
        names[0], "[Content_Types].xml",
        "the content types part comes first"
    );
    for required in [
        "_rels/.rels",
        "xl/workbook.xml",
        "xl/_rels/workbook.xml.rels",
        "xl/worksheets/sheet1.xml",
        "xl/worksheets/sheet2.xml",
        "xl/worksheets/sheet3.xml",
        "xl/styles.xml",
        "xl/sharedStrings.xml",
        "xl/metadata.xml",
    ] {
        assert!(names.iter().any(|n| n == required), "missing {required}");
    }
    for name in &names {
        assert_well_formed(name, &part(&bytes, name));
    }
    // Every part is declared and every relationship target exists.
    let types = part(&bytes, "[Content_Types].xml");
    for name in names
        .iter()
        .filter(|n| n.ends_with(".xml") && !n.starts_with('['))
    {
        assert!(
            types.contains(&format!("PartName=\"/{name}\"")),
            "{name} has no content type"
        );
    }
    let rels = part(&bytes, "xl/_rels/workbook.xml.rels");
    for target in rels
        .split("Target=\"")
        .skip(1)
        .map(|s| s.split('"').next().unwrap())
    {
        assert!(
            names.iter().any(|n| *n == format!("xl/{target}")),
            "dangling relationship to {target}"
        );
    }

    let workbook = part(&bytes, "xl/workbook.xml");
    assert!(workbook.contains("<sheet name=\"Summary\" sheetId=\"1\" r:id=\"rId1\"/>"));
    assert!(
        workbook.contains("<sheet name=\"Joe's &amp; &lt;Co&gt;\" sheetId=\"3\" r:id=\"rId3\"/>")
    );
    assert!(workbook.contains("<definedName name=\"Totals\">Data!$A$1:$A$5</definedName>"));
    assert!(workbook.contains(
        "<definedName name=\"Newer\">_xlfn.CONCAT(Data!$B$1,\"&lt;&amp;&gt;\")</definedName>"
    ));

    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    // Newer functions carry their prefix; results are cached next to the formula.
    assert!(sheet.contains(">_xlfn.IFS(A2&gt;1,\"big\",TRUE,\"small\")&amp;_xlfn.TEXTJOIN(\",\",TRUE,Data!B1:B2)</f>"), "{sheet}");
    // Formulas using functions that can return arrays are dynamic array
    // formulas even when they give a single value, as in files from Excel.
    assert!(
        sheet.contains("<f t=\"array\" ref=\"B3\">_xlfn.XLOOKUP(20,Data!A1:A5,Data!B1:B5)</f>")
    );
    assert!(sheet.contains("<f t=\"array\" ref=\"B10\">SUM(_xlfn._xlws.FILTER(Data!A1:A5,Data!A1:A5&gt;0))+_xlfn.STDEV.S(Data!A:A)</f>"));
    assert!(
        sheet.contains(
            "<f t=\"array\" ref=\"B12\">SUM(Data!A1:A5*(Data!A1:A5&gt;15))</f><v>50.5</v>"
        ),
        "array arithmetic with one result"
    );
    assert!(sheet.contains("<f>A2*2</f><v>84</v></c>"));
    assert!(sheet.contains("<f>SUM(Data!A1:A5)</f><v>59</v></c>"));
    assert!(sheet.contains(
        " t=\"str\"><f>IF(A3,\"yes &amp; &lt;no&gt;\",\"no\")</f><v>yes &amp; &lt;no&gt;</v></c>"
    ));
    assert!(sheet.contains(" t=\"e\"><f>1/0</f><v>#DIV/0!</v></c>"));
    assert!(sheet.contains(" t=\"b\"><f>A2&gt;1</f><v>1</v></c>"));
    // The dynamic array is flagged, with its spilled results cached.
    assert!(
        sheet.contains(
            "<c r=\"D1\" cm=\"1\"><f t=\"array\" ref=\"D1:D3\">Data!A1:A3*2</f><v>20</v></c>"
        ),
        "{sheet}"
    );
    assert!(sheet.contains("<v>40</v></c>") && sheet.contains("<c r=\"D3\"><v>61</v></c>"));
    // Structure.
    assert!(sheet.contains("<sheetPr><tabColor rgb=\"FFFF6600\"/></sheetPr>"));
    assert!(sheet.contains("<pane xSplit=\"2\" ySplit=\"1\" topLeftCell=\"C2\" activePane=\"bottomRight\" state=\"frozen\"/>"));
    assert!(sheet.contains("<mergeCells count=\"2\"><mergeCell ref=\"A17:C18\"/><mergeCell ref=\"B19:E19\"/></mergeCells>"));
    assert!(sheet.contains("<row r=\"1\" ht=\"22.5\" customHeight=\"1\">"));
    assert!(sheet.contains("<row r=\"8\" hidden=\"1\">"));
    assert!(sheet.contains("<col min=\"1\" max=\"1\" width=\"17.14285714\" customWidth=\"1\"/>"));
    // The used range reaches the cells where the formatted row 15 crosses
    // the formatted columns G:H.
    assert!(sheet.contains("<dimension ref=\"A1:H15\"/>"), "{sheet}");
    let sheet2 = part(&bytes, "xl/worksheets/sheet2.xml");
    assert!(sheet2.contains("showGridLines=\"0\""));
    assert!(sheet2.contains(
        "<pane ySplit=\"3\" topLeftCell=\"A4\" activePane=\"bottomLeft\" state=\"frozen\"/>"
    ));
    assert!(sheet2.contains("<v>1e-7</v>") && sheet2.contains("<v>1.2345678901234568e20</v>"));
    assert!(sheet2.contains("<v>0.30000000000000004</v>"));
    let sheet3 = part(&bytes, "xl/worksheets/sheet3.xml");
    assert!(sheet3.contains("showRowColHeaders=\"0\""));
    assert!(sheet3.contains(
        "<pane xSplit=\"1\" topLeftCell=\"B1\" activePane=\"topRight\" state=\"frozen\"/>"
    ));

    let styles = part(&bytes, "xl/styles.xml");
    assert!(styles.contains("<font><b/><i/><strike/><u val=\"double\"/><sz val=\"14.5\"/><color rgb=\"FFC00000\"/><name val=\"Arial\"/></font>"));
    assert!(styles.contains("<fgColor rgb=\"FFFFF2CC\"/>"));
    assert!(styles.contains("formatCode=\"[Blue]0.000;[Red]-0.000;&quot;zero&quot;\""));
    assert!(styles.contains("<right style=\"medium\"><color rgb=\"FF00B050\"/></right>"));
    assert!(
        styles.contains("textRotation=\"120\""),
        "-30 degrees is stored as 90 + 30"
    );
    assert!(
        styles.contains("numFmtId=\"14\""),
        "the locale's short date is the built-in date format"
    );
    assert!(styles.contains("numFmtId=\"4\"") && styles.contains("numFmtId=\"49\""));

    // Each distinct string is stored once.
    let strings = part(&bytes, "xl/sharedStrings.xml");
    assert_eq!(strings.matches("<t>apple</t>").count(), 1);
    assert!(strings.contains("<t xml:space=\"preserve\">  &lt;b&gt; &amp; \"q\"\nline 2\ttab_x000D__x005F_x000D_ _x0001_ é€  </t>"));
}

#[test]
fn empty_workbook() {
    let wb = Workbook::new(Locale::fr());
    let loaded = read_xlsx(Cursor::new(to_bytes(&wb)), Locale::fr()).unwrap();
    assert_same(&wb, &loaded);
    assert_eq!(loaded.sheets[0].name, "Feuil1");
    assert!(loaded.sheets[0].cells.is_empty());
    assert!(
        !part_names(&to_bytes(&wb))
            .iter()
            .any(|n| n == "xl/metadata.xml")
    );
}

#[test]
fn file_contents_are_canonical_whatever_the_locale() {
    let fr = Locale::fr();
    let mut wb = Workbook::new(fr);
    enter(&mut wb, 0, "A1", "1,5");
    enter(&mut wb, 0, "A2", "12/03/2025");
    enter(&mut wb, 0, "A3", "VRAI");
    enter(&mut wb, 0, "B1", "=SOMME(A1:A2;2,5)");
    enter(&mut wb, 0, "B2", "=SI(A3;\"oui\";\"non\")");
    enter(&mut wb, 0, "B3", "=NB.SI(A:A;\">1\")");
    wb.commit("setup");
    let bytes = to_bytes(&wb);
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    assert!(sheet.contains("<f>SUM(A1:A2,2.5)</f>"), "{sheet}");
    assert!(sheet.contains("<f>IF(A3,\"oui\",\"non\")</f>"));
    assert!(sheet.contains(">COUNTIF(A:A,\"&gt;1\")</f>"));
    assert!(sheet.contains("<v>1.5</v>") && sheet.contains("t=\"b\"><v>1</v>"));

    // Read back in French: the user sees what was typed.
    let french = read_xlsx(Cursor::new(&bytes), fr).unwrap();
    assert_same(&wb, &french);
    assert_eq!(french.formula_text(0, 0, 1).unwrap(), "=SOMME(A1:A2;2,5)");
    assert_eq!(french.style_at(0, 1, 0).num_fmt.as_ref(), "dd/mm/yyyy");
    // Read back in English: same workbook, English spelling, and the
    // system short date follows the reader's locale.
    let english = read_xlsx(Cursor::new(&bytes), Locale::en()).unwrap();
    assert_eq!(english.formula_text(0, 0, 1).unwrap(), "=SUM(A1:A2,2.5)");
    assert_eq!(english.style_at(0, 1, 0).num_fmt.as_ref(), "m/d/yyyy");
    assert_eq!(value(&english, 0, "A2"), value(&wb, 0, "A2"));
    assert_eq!(value(&english, 0, "B1"), value(&wb, 0, "B1"));
    assert_eq!(english.sheets[0].name, "Feuil1");
}

#[test]
fn dispatch_on_extension() {
    let wb = sample();
    let dir = tempfile::tempdir().unwrap();
    for name in ["a.xlsx", "b.XLSX", "c.xlsm"] {
        let path = dir.path().join(name);
        save(&wb, 0, &path).unwrap();
        assert_same(&wb, &load(&path, Locale::en()).unwrap());
    }
    // A macro-enabled name needs the matching content type to open in Excel.
    let xlsm = std::fs::read(dir.path().join("c.xlsm")).unwrap();
    assert!(
        part(&xlsm, "[Content_Types].xml")
            .contains("application/vnd.ms-excel.sheet.macroEnabled.main+xml")
    );
    // A workbook under an unknown extension is recognised by its content.
    let odd = dir.path().join("book.backup");
    std::fs::copy(dir.path().join("a.xlsx"), &odd).unwrap();
    assert_same(&wb, &load(&odd, Locale::en()).unwrap());

    let unsupported = dir.path().join("x.pdf");
    assert!(matches!(
        save(&wb, 0, &unsupported),
        Err(opensheet_io::IoError::Unsupported(_))
    ));
    std::fs::write(&unsupported, b"%PDF-1.7").unwrap();
    assert!(matches!(
        load(&unsupported, Locale::en()),
        Err(opensheet_io::IoError::Unsupported(_))
    ));
    // Saving replaces an existing file and leaves no temporary files behind.
    save(&Workbook::new(Locale::en()), 0, &dir.path().join("a.xlsx")).unwrap();
    assert!(
        load(&dir.path().join("a.xlsx"), Locale::en())
            .unwrap()
            .sheets[0]
            .cells
            .is_empty()
    );
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn errors() {
    use opensheet_io::IoError;
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("missing.xlsx");
    assert!(matches!(
        load_xlsx(&missing, Locale::en()),
        Err(IoError::Io(_))
    ));

    let not_zip = dir.path().join("text.xlsx");
    std::fs::write(&not_zip, "just some text").unwrap();
    let err = load_xlsx(&not_zip, Locale::en()).err().unwrap();
    assert!(matches!(err, IoError::Zip(_)), "{err:?}");
    assert!(err.to_string().contains("not a valid .xlsx file"));

    // Encrypted workbooks and .xls files are OLE compound documents.
    let ole = dir.path().join("secret.xlsx");
    std::fs::write(
        &ole,
        [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0, 0, 0],
    )
    .unwrap();
    assert!(matches!(
        load_xlsx(&ole, Locale::en()),
        Err(IoError::Unsupported(_))
    ));

    // A zip that is not a workbook.
    let other = dir.path().join("other.xlsx");
    std::fs::write(&other, common::package(&[("hello.txt", "hi")])).unwrap();
    assert!(matches!(
        load_xlsx(&other, Locale::en()),
        Err(IoError::Invalid(_))
    ));

    // A read-only document is not silently replaced.
    let locked = dir.path().join("locked.xlsx");
    save_xlsx(&Workbook::new(Locale::en()), &locked).unwrap();
    let before = std::fs::read(&locked).unwrap();
    let mut permissions = std::fs::metadata(&locked).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&locked, permissions).unwrap();
    assert!(matches!(save_xlsx(&sample(), &locked), Err(IoError::Io(_))));
    assert_eq!(std::fs::read(&locked).unwrap(), before);

    let unwritable = dir.path().join("no/such/dir/book.xlsx");
    assert!(matches!(
        save_xlsx(&Workbook::new(Locale::en()), &unwritable),
        Err(IoError::Io(_))
    ));
    let err: Box<dyn std::error::Error> = Box::new(IoError::NoSuchSheet(2));
    assert_eq!(err.to_string(), "the workbook has no sheet number 3");
}

#[test]
fn a_large_sheet_round_trips() {
    let mut wb = Workbook::new(Locale::en());
    for r in 0..3000u32 {
        wb.set_value(0, r, 0, Value::Number(r as f64 * 1.25));
        wb.set_value(0, r, 1, Value::text(format!("row {}", r % 100)));
        enter(
            &mut wb,
            0,
            &Rect::cell(r, 2).name(),
            &format!("=A{0}*2+LEN(B{0})", r + 1),
        );
    }
    wb.commit("fill");
    let loaded = read_xlsx(Cursor::new(to_bytes(&wb)), Locale::en()).unwrap();
    assert_same(&wb, &loaded);
    assert_eq!(at("C3000"), (2999, 2));
    assert_eq!(formula(&loaded, 0, "C3000").unwrap(), "A3000*2+LEN(B3000)");
}

#[test]
fn files_stay_valid_for_excel() {
    let en = Locale::en();
    let mut wb = Workbook::new(en);
    wb.add_sheet(1);
    assert!(wb.rename_sheet(1, "Other"));
    enter(&mut wb, 1, "A1", "5");
    enter(&mut wb, 0, "A1", "=Other!A1+1");
    enter(&mut wb, 0, "A2", "=SUM(Other!A:A,B2)");
    wb.define_name("Good", parse_formula("Sheet1!$B$2", en).unwrap());
    wb.define_name("On_other", parse_formula("Other!$A$1", en).unwrap());
    // Names Excel would reject: a cell address, a space.
    wb.define_name("AB12", parse_formula("1", en).unwrap());
    wb.define_name("two words", parse_formula("2", en).unwrap());
    wb.set_col_width(0, 0, Some(1800.0));
    wb.commit("setup");
    assert_eq!(value(&wb, 0, "A1"), Value::Number(6.0));
    // Deleting the sheet leaves references to it behind.
    assert!(wb.delete_sheet(1));
    wb.commit("delete");
    assert_eq!(value(&wb, 0, "A1"), Value::Error(ErrorKind::Ref));

    let bytes = to_bytes(&wb);
    let sheet = part(&bytes, "xl/worksheets/sheet1.xml");
    // Excel reports a file as damaged when a formula names a missing sheet.
    assert!(sheet.contains("<f>#REF!+1</f><v>#REF!</v>"), "{sheet}");
    assert!(sheet.contains("<f>SUM(#REF!,B2)</f>"));
    assert!(
        sheet.contains("width=\"255.71428571\""),
        "columns are at most 255 characters wide: {sheet}"
    );
    let workbook = part(&bytes, "xl/workbook.xml");
    assert!(workbook.contains("<definedName name=\"Good\">Sheet1!$B$2</definedName>"));
    assert!(workbook.contains("<definedName name=\"On_other\">#REF!</definedName>"));
    assert!(
        !workbook.contains("AB12") && !workbook.contains("two words"),
        "{workbook}"
    );

    let loaded = read_xlsx(Cursor::new(&bytes), en).unwrap();
    assert_eq!(formula(&loaded, 0, "A1").unwrap(), "#REF!+1");
    assert_eq!(value(&loaded, 0, "A1"), Value::Error(ErrorKind::Ref));
    assert_eq!(
        loaded.names.keys().map(|k| k.as_str()).collect::<Vec<_>>(),
        ["GOOD", "ON_OTHER"]
    );
}

#[test]
fn damaged_files_never_panic() {
    // Cut every part of a real package short at many places: loading may
    // fail or give a partial workbook, but it must return.
    let bytes = to_bytes(&sample());
    let names = part_names(&bytes);
    let parts: Vec<(String, String)> = names.iter().map(|n| (n.clone(), part(&bytes, n))).collect();
    let mut attempts = 0;
    for (victim, text) in &parts {
        let step = (text.len() / 12).max(1);
        for cut in (0..text.len())
            .step_by(step)
            .filter(|i| text.is_char_boundary(*i))
        {
            let damaged: Vec<(&str, &str)> = parts
                .iter()
                .map(|(n, t)| (n.as_str(), if n == victim { &t[..cut] } else { t.as_str() }))
                .collect();
            let _ = read_xlsx(Cursor::new(common::package(&damaged)), Locale::en());
            attempts += 1;
        }
    }
    assert!(attempts > 100);
    // A truncated archive is an error, not a panic.
    for len in [0, 1, 4, 22, bytes.len() / 2, bytes.len() - 1] {
        assert!(
            read_xlsx(Cursor::new(&bytes[..len]), Locale::en()).is_err(),
            "{len} bytes"
        );
    }
}
