//! Defined names as the Name Manager needs them: scopes, comments,
//! constants and formulas, relative references, validation, renaming, and
//! how they follow the sheets.

mod common;

use common::{calc, enter, err, num, shown, value, workbook};
use opensheet_engine::ErrorKind::{Name, Ref};
use opensheet_engine::names::{NameError, NameScope, is_valid_name};
use opensheet_engine::parser::parse_formula;
use opensheet_engine::{Locale, Rect, Reference, Value, Workbook};

fn expr(text: &str) -> opensheet_engine::Expr {
    parse_formula(text, Locale::en()).unwrap()
}

/// The Name Manager's list: (scope, name, refers to, comment).
fn listing(wb: &Workbook) -> Vec<(NameScope, String, String, String)> {
    wb.all_names()
        .into_iter()
        .map(|(scope, def)| {
            (
                scope,
                def.name.clone(),
                def.refers_to(wb.locale, 0, 0),
                def.comment.clone(),
            )
        })
        .collect()
}

#[test]
fn validation() {
    let en = Locale::en();
    for name in [
        "Rate", "_x", "\\x", "\\", "Tax.2024", "été", "A1B", "ABCD1", "XFE1", "R2D2", "Cost",
        "Row", "a?b", "x_1", "Ω",
    ] {
        assert!(is_valid_name(name, en), "{name}");
    }
    for name in [
        "",
        "A1",
        "$A$1",
        "a1",
        "XFD1048576",
        "TAX2024",
        "2x",
        "my name",
        "a-b",
        "a+b",
        "R",
        "c",
        "rc",
        "R1C1",
        "C12",
        "r5c",
        "R5",
        "true",
        "FALSE",
        "a!b",
        "#x",
        "a#",
        ".a",
        "?a",
        "x'y",
        "a[b]",
        "x,y",
    ] {
        assert!(!is_valid_name(name, en), "{name}");
    }
    assert!(!is_valid_name(&"x".repeat(256), en));
    assert!(is_valid_name(&"x".repeat(255), en));
    // The letters of the R1C1 notation and the booleans depend on the
    // language; the English ones stay reserved everywhere.
    let fr = Locale::fr();
    for name in [
        "L", "l1c1", "L5", "C", "LC", "R", "R1C1", "VRAI", "faux", "TRUE",
    ] {
        assert!(!is_valid_name(name, fr), "{name}");
    }
    assert!(is_valid_name("L", en) && is_valid_name("VRAI", en) && is_valid_name("Ligne", fr));
    assert!(
        !is_valid_name("Z1S1", Locale::de())
            && !is_valid_name("S", Locale::de())
            && !is_valid_name("WAHR", Locale::de())
    );
    assert!(!is_valid_name("F3", Locale::es()) && !is_valid_name("f1c1", Locale::es()));
    assert!(!is_valid_name("VERO", Locale::it()));

    let mut wb = workbook(2);
    wb.add_name(NameScope::Workbook, "Rate", expr("0.2"), "")
        .unwrap();
    wb.add_name(NameScope::Sheet(1), "Local", expr("1"), "")
        .unwrap();
    for bad in ["A1", "R1C1", "2x", "a b", "", "TRUE", "c"] {
        assert_eq!(
            wb.add_name(NameScope::Workbook, bad, expr("1"), ""),
            Err(NameError::Invalid),
            "{bad}"
        );
        assert_eq!(
            wb.add_name(NameScope::Sheet(0), bad, expr("1"), ""),
            Err(NameError::Invalid),
            "{bad}"
        );
    }
    // Unique in its scope, whatever the case; the same spelling is free in
    // another scope.
    assert_eq!(
        wb.add_name(NameScope::Workbook, "RATE", expr("1"), ""),
        Err(NameError::Duplicate)
    );
    assert_eq!(
        wb.add_name(NameScope::Workbook, "rate", expr("1"), ""),
        Err(NameError::Duplicate)
    );
    assert_eq!(
        wb.add_name(NameScope::Sheet(1), "LOCAL", expr("1"), ""),
        Err(NameError::Duplicate)
    );
    assert_eq!(
        wb.add_name(NameScope::Sheet(0), "Rate", expr("0.5"), ""),
        Ok(())
    );
    assert_eq!(
        wb.add_name(NameScope::Sheet(0), "Local", expr("2"), ""),
        Ok(())
    );
    assert_eq!(
        wb.add_name(NameScope::Workbook, "Local", expr("3"), ""),
        Ok(())
    );
    assert_eq!(
        wb.add_name(NameScope::Sheet(7), "Fine", expr("1"), ""),
        Err(NameError::NotFound)
    );
    assert_eq!(
        wb.check_name(NameScope::Workbook, "Rate", Some("rate")),
        Ok(())
    );
    assert_eq!(
        wb.check_name(NameScope::Workbook, "Rate", Some("Other")),
        Err(NameError::Duplicate)
    );
    assert_eq!(wb.all_names().len(), 5);
    // Surrounding spaces are not part of the name.
    wb.add_name(NameScope::Workbook, "  Padded ", expr("1"), "")
        .unwrap();
    assert!(
        wb.name_in(NameScope::Workbook, "padded")
            .is_some_and(|d| d.name == "Padded")
    );
}

#[test]
fn scopes() {
    let mut wb = workbook(3);
    enter(&mut wb, 0, "A1", "10");
    enter(&mut wb, 1, "A1", "20");
    enter(&mut wb, 2, "A1", "30");
    wb.add_name(
        NameScope::Workbook,
        "Amount",
        expr("Sheet1!$A$1"),
        "for everyone",
    )
    .unwrap();
    wb.add_name(
        NameScope::Sheet(1),
        "Amount",
        expr("Sheet2!$A$1"),
        "for Sheet2",
    )
    .unwrap();
    wb.add_name(NameScope::Sheet(2), "Third", expr("Sheet3!$A$1*2"), "")
        .unwrap();
    wb.add_name(NameScope::Workbook, "Rate", expr("0.5"), "")
        .unwrap();
    wb.add_name(NameScope::Workbook, "Greeting", expr("\"hello\""), "")
        .unwrap();
    wb.add_name(
        NameScope::Workbook,
        "Twice",
        expr("Amount*2"),
        "a name using a name",
    )
    .unwrap();
    wb.commit("names");
    assert_eq!(
        listing(&wb),
        [
            (
                NameScope::Workbook,
                "Amount".into(),
                "=Sheet1!$A$1".into(),
                "for everyone".into()
            ),
            (
                NameScope::Sheet(1),
                "Amount".into(),
                "=Sheet2!$A$1".into(),
                "for Sheet2".into()
            ),
            (
                NameScope::Workbook,
                "Greeting".into(),
                "=\"hello\"".into(),
                "".into()
            ),
            (NameScope::Workbook, "Rate".into(), "=0.5".into(), "".into()),
            (
                NameScope::Sheet(2),
                "Third".into(),
                "=Sheet3!$A$1*2".into(),
                "".into()
            ),
            (
                NameScope::Workbook,
                "Twice".into(),
                "=Amount*2".into(),
                "a name using a name".into()
            ),
        ]
    );
    // From a sheet, its own name wins; `Sheet!Name` asks another sheet.
    for (sheet, formula, expected) in [
        (0, "=Amount", num(10.0)),
        (1, "=Amount", num(20.0)),
        (2, "=amount", num(10.0)),
        (0, "=Sheet2!Amount", num(20.0)),
        (1, "=Sheet1!Amount+Sheet3!Amount", num(20.0)),
        (0, "=Sheet3!Third+Sheet2!Rate", num(60.5)),
        (2, "=Third", num(60.0)),
        (0, "=Third", err(Name)),
        (0, "=Sheet2!Third", err(Name)),
        (0, "=Nowhere!Amount", err(Ref)),
        (0, "=Amount*Rate&Greeting", Value::text("5hello")),
        // A name is evaluated where it is used.
        (0, "=Twice", num(20.0)),
        (1, "=Twice", num(40.0)),
        (
            0,
            "=ISREF(Amount)&ISREF(Rate)&ROW(Sheet2!Amount)",
            Value::text("TRUEFALSE1"),
        ),
        (0, "=SUM(INDIRECT(\"Amount\"))", num(10.0)),
    ] {
        assert_eq!(
            calc(&mut wb, sheet, "C3", formula),
            expected,
            "{formula} on sheet {sheet}"
        );
    }
    assert_eq!(
        wb.lookup_name(1, "AMOUNT").map(|(scope, _)| scope),
        Some(NameScope::Sheet(1))
    );
    assert_eq!(
        wb.lookup_name(2, "amount").map(|(scope, _)| scope),
        Some(NameScope::Workbook)
    );
    assert!(wb.lookup_name(0, "Third").is_none());
    assert_eq!(
        wb.name_reference(1, 0, 0, "Amount"),
        Some(Reference::new(1, Rect::parse("A1").unwrap()))
    );
    assert_eq!(wb.name_reference(0, 0, 0, "Rate"), None, "not a range");
    // The old entry point still defines workbook names, unchecked.
    wb.define_name("Legacy", expr("Sheet1!$A$1+1"));
    wb.define_name("two words", expr("2"));
    assert_eq!(
        wb.defined_name("legacy").map(|e| e.to_text(Locale::en())),
        Some("Sheet1!$A$1+1".into())
    );
    assert_eq!(calc(&mut wb, 2, "D1", "=Legacy"), num(11.0));
    assert!(wb.name_in(NameScope::Workbook, "two words").is_some());

    // Deleting a name leaves its users with #NAME?.
    enter(&mut wb, 1, "C3", "=Amount");
    wb.commit("use");
    assert!(wb.delete_name(NameScope::Sheet(1), "amount"));
    wb.commit("delete name");
    assert_eq!(
        value(&wb, 1, "C3"),
        num(10.0),
        "the workbook's name shows through"
    );
    assert!(wb.delete_name(NameScope::Workbook, "Amount"));
    assert!(!wb.delete_name(NameScope::Workbook, "Amount"));
    wb.commit("delete name");
    assert_eq!(value(&wb, 1, "C3"), err(Name));
    assert!(wb.undo() && wb.undo());
    assert_eq!(value(&wb, 1, "C3"), num(20.0));
}

#[test]
fn relative_references() {
    let mut wb = workbook(2);
    for (i, v) in [10, 20, 30].iter().enumerate() {
        enter(&mut wb, 0, &format!("A{}", i + 1), &v.to_string());
        enter(&mut wb, 0, &format!("B{}", i + 1), &(v * 2).to_string());
        enter(&mut wb, 1, &format!("A{}", i + 1), &(v + 1).to_string());
    }
    // Defined with B1 selected: "the cell on my left" and "the three cells
    // from mine down, one column to the left".
    let left = wb.parse_name_formula("=A1", 0, 0, 1).unwrap();
    assert_eq!(
        left.to_text(Locale::en()),
        "Sheet1!XFD1",
        "stored as read from A1"
    );
    wb.add_name(NameScope::Workbook, "Left", left, "").unwrap();
    let column = wb.parse_name_formula("=SUM($A1:$A3)", 0, 0, 1).unwrap();
    wb.add_name(NameScope::Workbook, "Below", column, "")
        .unwrap();
    let fixed = wb.parse_name_formula("$A$1:$A$3", 0, 5, 5).unwrap();
    wb.add_name(NameScope::Workbook, "Fixed", fixed, "")
        .unwrap();
    let above = wb.parse_name_formula("=Sheet2!$A1", 0, 1, 0).unwrap();
    wb.add_name(NameScope::Workbook, "AboveOnTwo", above, "")
        .unwrap();
    wb.commit("names");

    assert_eq!(calc(&mut wb, 0, "C1", "=Left"), num(20.0));
    assert_eq!(calc(&mut wb, 0, "C3", "=Left*2"), num(120.0));
    assert_eq!(calc(&mut wb, 0, "B5", "=Left+1"), num(1.0));
    assert_eq!(calc(&mut wb, 0, "E1", "=Below"), num(60.0));
    assert_eq!(calc(&mut wb, 0, "E2", "=Below"), num(50.0));
    assert_eq!(calc(&mut wb, 0, "E3", "=SUM(Fixed)+ROWS(Fixed)"), num(63.0));
    assert_eq!(calc(&mut wb, 0, "F3", "=AboveOnTwo"), num(21.0));
    // The Name Manager shows the reference as it reads from the selected cell.
    let def = wb.name_in(NameScope::Workbook, "left").unwrap();
    assert_eq!(def.refers_to(wb.locale, 0, 1), "=Sheet1!A1");
    assert_eq!(def.refers_to(wb.locale, 4, 3), "=Sheet1!C5");
    assert_eq!(
        wb.name_in(NameScope::Workbook, "Below")
            .unwrap()
            .refers_to(wb.locale, 9, 9),
        "=SUM(Sheet1!$A10:$A12)"
    );
    assert_eq!(
        wb.name_in(NameScope::Workbook, "Fixed")
            .unwrap()
            .refers_to(wb.locale, 9, 9),
        "=Sheet1!$A$1:$A$3"
    );
    assert_eq!(
        wb.name_reference(0, 2, 1, "Left"),
        Some(Reference::new(0, Rect::parse("A3").unwrap()))
    );
    // Off the edge of the sheet the reference comes back on the other side.
    assert_eq!(calc(&mut wb, 0, "A8", "=Left"), num(0.0));
    assert_eq!(def_text(&wb, "Left", 7, 0), "=Sheet1!XFD8");
    // French, with the semicolon and decimal comma.
    let mut fr = Workbook::new(Locale::fr());
    enter(&mut fr, 0, "A1", "4");
    let e = fr.parse_name_formula("=SOMME($A$1;1,5)", 0, 0, 0).unwrap();
    fr.add_name(NameScope::Workbook, "Somme", e, "").unwrap();
    assert_eq!(calc(&mut fr, 0, "C1", "=Somme*2"), num(11.0));
    assert_eq!(def_text(&fr, "somme", 0, 0), "=SOMME(Feuil1!$A$1;1,5)");
    assert!(fr.parse_name_formula("=SOMME(A1,", 0, 0, 0).is_err());
}

fn def_text(wb: &Workbook, name: &str, row: u32, col: u32) -> String {
    wb.name_in(NameScope::Workbook, name)
        .unwrap()
        .refers_to(wb.locale, row, col)
}

#[test]
fn several_areas_and_other_formulas() {
    let mut wb = workbook(2);
    for (cell, v) in [("A1", 1), ("A2", 2), ("C1", 10), ("C2", 20)] {
        enter(&mut wb, 0, cell, &v.to_string());
    }
    enter(&mut wb, 1, "A1", "100");
    let both = wb
        .parse_name_formula("=$A$1:$A$2,$C$1:$C$2", 0, 0, 0)
        .unwrap();
    wb.add_name(NameScope::Workbook, "Both", both, "").unwrap();
    wb.add_name(NameScope::Workbook, "Deep", expr("Sheet1:Sheet2!$A$1"), "")
        .unwrap();
    wb.add_name(
        NameScope::Workbook,
        "Crossing",
        expr("Sheet1!$A:$A Sheet1!$2:$2"),
        "",
    )
    .unwrap();
    wb.add_name(
        NameScope::Workbook,
        "Total",
        expr("SUM(Sheet1!$A$1:$A$2)*2"),
        "",
    )
    .unwrap();
    wb.add_name(NameScope::Workbook, "Loop", expr("Loop+1"), "")
        .unwrap();
    wb.commit("names");
    assert_eq!(
        def_text(&wb, "Both", 0, 0),
        "=Sheet1!$A$1:$A$2,Sheet1!$C$1:$C$2"
    );
    assert_eq!(
        calc(
            &mut wb,
            0,
            "E1",
            "=SUM(Both)+AREAS(Both)*1000+COUNT(Both)*100"
        ),
        num(2433.0)
    );
    assert_eq!(
        calc(&mut wb, 0, "E1", "=INDEX(Both,2,1,2)+LARGE(Both,1)"),
        num(40.0)
    );
    assert_eq!(
        calc(&mut wb, 0, "E1", "=SUM(Deep)+SUM(Sheet2!Deep,1)"),
        num(203.0)
    );
    assert_eq!(calc(&mut wb, 0, "E1", "=Crossing+Total"), num(8.0));
    assert_eq!(
        calc(&mut wb, 0, "E1", "=Both"),
        err(opensheet_engine::ErrorKind::Value)
    );
    // A name defined in terms of itself ends in an error, not in a crash.
    assert!(matches!(calc(&mut wb, 0, "E1", "=Loop"), Value::Error(_)));
    assert_eq!(wb.evaluate(0, "=SUM(Both)"), Ok(num(33.0)));
}

#[test]
fn renaming_rewrites_the_formulas() {
    let mut wb = workbook(2);
    enter(&mut wb, 0, "A1", "10");
    enter(&mut wb, 1, "A1", "20");
    wb.add_name(NameScope::Workbook, "Data", expr("Sheet1!$A$1"), "global")
        .unwrap();
    wb.add_name(NameScope::Sheet(1), "Data", expr("Sheet2!$A$1"), "local")
        .unwrap();
    wb.add_name(NameScope::Workbook, "Twice", expr("Data*2"), "")
        .unwrap();
    wb.add_name(NameScope::Sheet(1), "Thrice", expr("Data*3"), "")
        .unwrap();
    enter(&mut wb, 0, "B1", "=Data+data*2+Sheet2!Data+Sheet1!Data");
    enter(&mut wb, 0, "B2", "=LET(Data,5,Data+1)+\"Data\"");
    enter(&mut wb, 1, "B1", "=Data+Sheet1!DATA+Sheet2!Data");
    wb.commit("setup");
    assert_eq!(value(&wb, 0, "B1"), num(60.0));
    assert_eq!(value(&wb, 1, "B1"), num(50.0));

    // The workbook's name: every use that means it, and only those.
    assert_eq!(
        wb.rename_name(NameScope::Workbook, "data", "Amount"),
        Ok(())
    );
    wb.commit("rename");
    assert_eq!(
        shown(&wb, 0, "B1"),
        "=Amount+Amount*2+Sheet2!Data+Sheet1!Amount"
    );
    assert_eq!(
        shown(&wb, 0, "B2"),
        "=LET(Data,5,Data+1)+\"Data\"",
        "a LET variable and a text are not the name"
    );
    assert_eq!(shown(&wb, 1, "B1"), "=Data+Sheet1!Amount+Sheet2!Data");
    assert_eq!(def_text(&wb, "Twice", 0, 0), "=Amount*2");
    assert_eq!(
        wb.name_in(NameScope::Sheet(1), "Thrice")
            .unwrap()
            .refers_to(wb.locale, 0, 0),
        "=Data*3"
    );
    assert_eq!(
        wb.name_in(NameScope::Workbook, "Amount").unwrap().comment,
        "global"
    );
    assert!(wb.name_in(NameScope::Workbook, "Data").is_none());
    assert_eq!(
        (value(&wb, 0, "B1"), value(&wb, 1, "B1")),
        (num(60.0), num(50.0))
    );

    // The sheet's name.
    assert_eq!(wb.rename_name(NameScope::Sheet(1), "Data", "Mine"), Ok(()));
    wb.commit("rename");
    assert_eq!(
        shown(&wb, 0, "B1"),
        "=Amount+Amount*2+Sheet2!Mine+Sheet1!Amount"
    );
    assert_eq!(shown(&wb, 1, "B1"), "=Mine+Sheet1!Amount+Sheet2!Mine");
    assert_eq!(
        wb.name_in(NameScope::Sheet(1), "Thrice")
            .unwrap()
            .refers_to(wb.locale, 0, 0),
        "=Mine*3"
    );
    assert_eq!(
        (value(&wb, 0, "B1"), value(&wb, 1, "B1")),
        (num(60.0), num(50.0))
    );

    // Refusals leave everything as it was.
    assert_eq!(
        wb.rename_name(NameScope::Workbook, "Amount", "Twice"),
        Err(NameError::Duplicate)
    );
    assert_eq!(
        wb.rename_name(NameScope::Workbook, "Amount", "A1"),
        Err(NameError::Invalid)
    );
    assert_eq!(
        wb.rename_name(NameScope::Workbook, "Nope", "Fine"),
        Err(NameError::NotFound)
    );
    assert_eq!(
        wb.rename_name(NameScope::Sheet(0), "Amount", "Fine"),
        Err(NameError::NotFound)
    );
    // Changing only the case is a rename too.
    assert_eq!(
        wb.rename_name(NameScope::Workbook, "amount", "AMOUNT"),
        Ok(())
    );
    wb.commit("case");
    assert_eq!(shown(&wb, 1, "B1"), "=Mine+Sheet1!AMOUNT+Sheet2!Mine");

    // Undo, step by step.
    assert!(wb.undo() && wb.undo());
    assert_eq!(
        shown(&wb, 0, "B1"),
        "=Amount+Amount*2+Sheet2!Data+Sheet1!Amount"
    );
    assert!(wb.undo());
    assert_eq!(shown(&wb, 0, "B1"), "=Data+data*2+Sheet2!Data+Sheet1!Data");
    assert!(
        wb.name_in(NameScope::Workbook, "Data").is_some()
            && wb.name_in(NameScope::Workbook, "Amount").is_none()
    );

    // Edit: name, definition and comment at once.
    let new = wb.parse_name_formula("=$A$1*100", 0, 0, 0).unwrap();
    assert_eq!(
        wb.edit_name(NameScope::Workbook, "Data", "Hundred", new, "times 100"),
        Ok(())
    );
    wb.commit("edit");
    assert_eq!(
        shown(&wb, 0, "B1"),
        "=Hundred+Hundred*2+Sheet2!Data+Sheet1!Hundred"
    );
    assert_eq!(value(&wb, 0, "B1"), num(4020.0));
    let def = wb.name_in(NameScope::Workbook, "hundred").unwrap();
    assert_eq!(
        (
            def.refers_to(wb.locale, 0, 0).as_str(),
            def.comment.as_str()
        ),
        ("=Sheet1!$A$1*100", "times 100")
    );
    assert_eq!(
        wb.edit_name(NameScope::Workbook, "Hundred", "Twice", expr("1"), ""),
        Err(NameError::Duplicate)
    );
    assert!(wb.undo());
    assert_eq!(value(&wb, 0, "B1"), num(60.0));
    assert_eq!(
        wb.name_in(NameScope::Workbook, "Data").unwrap().comment,
        "global"
    );
}

#[test]
fn names_follow_the_sheets() {
    let mut wb = workbook(3);
    for s in 0..3 {
        for r in 1..=4 {
            enter(
                &mut wb,
                s,
                &format!("A{r}"),
                &format!("{}", (s + 1) * 10 + r),
            );
        }
    }
    wb.add_name(NameScope::Workbook, "Global", expr("Sheet2!$A$2:$A$3"), "")
        .unwrap();
    wb.add_name(NameScope::Sheet(1), "Mine", expr("Sheet2!$A$2"), "kept")
        .unwrap();
    wb.add_name(NameScope::Sheet(1), "Far", expr("Sheet3!$A$4"), "")
        .unwrap();
    wb.add_name(NameScope::Sheet(2), "Mine", expr("Sheet3!$A$1"), "")
        .unwrap();
    enter(&mut wb, 0, "C1", "=SUM(Global)+Sheet2!Mine");
    enter(&mut wb, 1, "C1", "=Mine+Far");
    wb.commit("setup");
    assert_eq!(
        (value(&wb, 0, "C1"), value(&wb, 1, "C1")),
        (num(67.0), num(56.0))
    );

    // Rows inserted and deleted.
    wb.insert_rows(1, 0, 2);
    wb.commit("insert");
    assert_eq!(def_text(&wb, "Global", 0, 0), "=Sheet2!$A$4:$A$5");
    assert_eq!(
        wb.name_in(NameScope::Sheet(1), "Mine")
            .unwrap()
            .refers_to(wb.locale, 0, 0),
        "=Sheet2!$A$4"
    );
    assert_eq!(
        wb.name_in(NameScope::Sheet(1), "Far")
            .unwrap()
            .refers_to(wb.locale, 0, 0),
        "=Sheet3!$A$4"
    );
    assert_eq!(
        (value(&wb, 0, "C1"), value(&wb, 1, "C3")),
        (num(67.0), num(56.0))
    );
    wb.delete_rows(1, 3, 1);
    wb.commit("delete");
    assert_eq!(def_text(&wb, "Global", 0, 0), "=Sheet2!$A$4:$A$4");
    assert_eq!(
        wb.name_in(NameScope::Sheet(1), "Mine")
            .unwrap()
            .refers_to(wb.locale, 0, 0),
        "=Sheet2!#REF!"
    );
    assert!(wb.undo() && wb.undo());

    // The sheet renamed: the scope stays, the references follow.
    assert!(wb.rename_sheet(1, "Second one"));
    wb.commit("rename");
    assert_eq!(shown(&wb, 0, "C1"), "=SUM(Global)+'Second one'!Mine");
    assert_eq!(def_text(&wb, "Global", 0, 0), "='Second one'!$A$2:$A$3");
    let mine = wb.name_in(NameScope::Sheet(1), "Mine").unwrap();
    assert_eq!(
        (
            mine.refers_to(wb.locale, 0, 0).as_str(),
            mine.comment.as_str()
        ),
        ("='Second one'!$A$2", "kept")
    );
    assert_eq!(
        (value(&wb, 0, "C1"), value(&wb, 1, "C1")),
        (num(67.0), num(56.0))
    );

    // The sheet moved: its names move with it.
    wb.move_sheet(1, 2);
    wb.commit("move");
    assert!(wb.name_in(NameScope::Sheet(2), "Far").is_some());
    assert_eq!(
        wb.name_in(NameScope::Sheet(1), "Mine")
            .unwrap()
            .refers_to(wb.locale, 0, 0),
        "=Sheet3!$A$1"
    );
    assert_eq!(
        (value(&wb, 0, "C1"), value(&wb, 2, "C1")),
        (num(67.0), num(56.0))
    );

    // The sheet copied: the copy has its own names, about its own cells.
    let copy = wb.duplicate_sheet(2).unwrap();
    wb.commit("copy");
    assert_eq!(wb.sheets[copy].name, "Second one (2)");
    let copied = wb.name_in(NameScope::Sheet(copy), "Mine").unwrap();
    assert_eq!(copied.refers_to(wb.locale, 0, 0), "='Second one (2)'!$A$2");
    assert_eq!(
        wb.name_in(NameScope::Sheet(copy), "Far")
            .unwrap()
            .refers_to(wb.locale, 0, 0),
        "=Sheet3!$A$4"
    );
    assert_eq!(
        wb.name_in(NameScope::Sheet(2), "Mine")
            .unwrap()
            .refers_to(wb.locale, 0, 0),
        "='Second one'!$A$2"
    );
    assert_eq!(value(&wb, copy, "C1"), num(56.0));

    // The sheet deleted: its names go, the others lose their cells.
    assert!(wb.delete_sheet(2));
    wb.commit("delete sheet");
    assert_eq!(
        listing(&wb)
            .into_iter()
            .map(|(scope, name, text, _)| (scope, name, text))
            .collect::<Vec<_>>(),
        [
            (
                NameScope::Sheet(2),
                "Far".to_string(),
                "=Sheet3!$A$4".to_string()
            ),
            (
                NameScope::Workbook,
                "Global".to_string(),
                "=#REF!$A$2:$A$3".to_string()
            ),
            (
                NameScope::Sheet(1),
                "Mine".to_string(),
                "=Sheet3!$A$1".to_string()
            ),
            (
                NameScope::Sheet(2),
                "Mine".to_string(),
                "='Second one (2)'!$A$2".to_string()
            ),
        ]
    );
    assert_eq!(shown(&wb, 0, "C1"), "=SUM(Global)+#REF!");
    assert_eq!(value(&wb, 0, "C1"), err(Ref));
    assert!(wb.undo());
    assert_eq!(wb.all_names().len(), 6);
    assert_eq!(
        (value(&wb, 0, "C1"), value(&wb, 2, "C1")),
        (num(67.0), num(56.0))
    );
}
