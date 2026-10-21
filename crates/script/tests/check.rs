//! `check`: what it finds in a script without running it, what it leaves
//! alone, and `complete`, which tells an editor what can follow a dot.

use opensheet_script::{CompletionKind, ErrorKind, check, complete, members_of};

/// The findings of a script as (line, column, message).
fn findings(source: &str) -> Vec<(u32, u32, String)> {
    check(source)
        .into_iter()
        .map(|finding| {
            assert_eq!(finding.kind, ErrorKind::Check, "{}", finding.message);
            (
                finding.line.expect("a line"),
                finding.column.expect("a column"),
                finding.message,
            )
        })
        .collect()
}

/// The findings of the statements of a `main`, which start at line 2.
fn in_main(body: &str) -> Vec<(u32, u32, String)> {
    findings(&format!(
        "function main(workbook: ExcelScript.Workbook) {{\n{body}\n}}\n"
    ))
}

fn silent(body: &str) {
    assert_eq!(in_main(body), Vec::new(), "{body}");
}

#[test]
fn every_script_of_the_tests_is_left_alone() {
    let tests = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let mut seen = 0;
    let mut wrong: Vec<String> = Vec::new();
    for folder in ["corpus", "model", "."] {
        for entry in std::fs::read_dir(tests.join(folder)).expect("a folder of scripts") {
            let path = entry.expect("an entry").path();
            if path.extension().is_none_or(|extension| extension != "ts") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("a script");
            for finding in check(&source) {
                wrong.push(format!(
                    "{}:{}:{}: {}",
                    path.file_name().unwrap().to_string_lossy(),
                    finding.line.unwrap_or(0),
                    finding.column.unwrap_or(0),
                    finding.message
                ));
            }
            seen += 1;
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    assert!(seen >= 30, "{seen} scripts");
}

#[test]
fn a_member_a_class_does_not_have_is_found_with_its_place() {
    // A misspelling at the end of a chain: line 2, at the member.
    assert_eq!(
        in_main("  workbook.getActiveWorksheet().getRange(\"A1\").setValeus([[1]]);"),
        [(2, 48, "Range.setValeus is not supported in OpenSheet: no such member, or one of Office Scripts that OpenSheet does not have".to_string())]
    );
    // Through bindings, for-of, callbacks of arrays of the model, casts,
    // annotations, optional chaining and `!`.
    let found = in_main(concat!(
        "  const sheet = workbook.getActiveWorksheet();\n",
        "  const range = sheet.getRange(\"A1:B2\");\n",
        "  range.values = [[1]];\n",
        "  console.log(range.address);\n",
        "  for (const each of workbook.getWorksheets()) { each.getNam(); }\n",
        "  workbook.getTables().forEach(table => table.getRowz());\n",
        "  workbook.getTables()[0].getColumn(\"A\").getFilter().applyValuesFilter([\"x\"]).nope();\n",
        "  workbook.getTable(\"T\")?.getRanje();\n",
        "  workbook.getTable(\"T\")!.getRanje();\n",
        "  let chart: ExcelScript.Chart | undefined;\n",
        "  chart.setTitel(\"x\");\n",
        "  (workbook.getNamedItem(\"n\") as ExcelScript.NamedItem).getFormla();\n",
        "  const found = workbook.getWorksheets().find(s => s.getName() === \"x\");\n",
        "  found.activat();\n",
    ));
    let places: Vec<(u32, u32, &str)> = found
        .iter()
        .map(|(line, column, message)| (*line, *column, message.split(" is not").next().unwrap()))
        .collect();
    assert_eq!(
        places,
        [
            (4, 9, "Range.values"),
            (5, 21, "Range.address"),
            (6, 55, "Worksheet.getNam"),
            (7, 47, "Table.getRowz"),
            (9, 27, "Table.getRanje"),
            (10, 27, "Table.getRanje"),
            (12, 9, "Chart.setTitel"),
            (13, 57, "NamedItem.getFormla"),
            (15, 9, "Worksheet.activat"),
        ]
    );
    // What a member returns that is not an object of the model ends the
    // chain: line 8 calls a method of `void`, which is not looked at.
    // Functions of the script: a declared return type is followed, and
    // so is a parameter with its annotation.
    let helpers = concat!(
        "function main(workbook: ExcelScript.Workbook) {\n",
        "  first(workbook).getRange().clearr();\n",
        "}\n",
        "function first(book: ExcelScript.Workbook): ExcelScript.Worksheet {\n",
        "  return book.getFirstWorksheett();\n",
        "}\n",
        "const paint = (range: ExcelScript.Range) => range.getFormat().getFil();\n",
    );
    let places: Vec<(u32, u32)> = findings(helpers)
        .iter()
        .map(|(line, column, _)| (*line, *column))
        .collect();
    assert_eq!(places, [(2, 30), (5, 15), (7, 63)]);
    // `main` receives the workbook even when it does not say so.
    assert_eq!(
        findings("function main(wb) {\n  wb.getSheets();\n}\n").len(),
        1
    );
}

#[test]
fn the_number_of_arguments_is_counted() {
    let found = in_main(concat!(
        "  const sheet = workbook.getActiveWorksheet();\n",
        "  sheet.getCell(1);\n",
        "  sheet.getRange(\"A1\", \"B2\");\n",
        "  sheet.getRange().setValue();\n",
        "  sheet.getRangeByIndexes(0, 0, 1, 1, 1);\n",
        "  workbook.addWorksheet(\"a\", \"b\");\n",
    ));
    assert_eq!(
        found,
        [
            (
                3,
                9,
                "Worksheet.getCell takes 2 arguments, not 1".to_string()
            ),
            (
                4,
                9,
                "Worksheet.getRange takes 0 to 1 arguments, not 2".to_string()
            ),
            (5, 20, "Range.setValue takes 1 argument, not 0".to_string()),
            (
                6,
                9,
                "Worksheet.getRangeByIndexes takes 4 arguments, not 5".to_string()
            ),
            (
                7,
                12,
                "Workbook.addWorksheet takes 0 to 1 arguments, not 2".to_string()
            ),
        ]
    );
    // Arguments that are spread are not counted.
    silent(
        "  const at: [number, number] = [0, 0];\n  workbook.getActiveWorksheet().getCell(...at);",
    );
}

#[test]
fn enum_values_that_do_not_exist_are_found() {
    let found = in_main(concat!(
        "  const a = ExcelScript.HorizontalAlignment.centre;\n",
        "  const b = ExcelScript.ChartType.waterfall;\n",
        "  const c = ExcelScript.SlicerSortType.ascending;\n",
        "  const d = ExcelScript.ClearApplyTo.contents;\n",
    ));
    let places: Vec<(u32, u32, &str)> = found
        .iter()
        .map(|(line, column, message)| (*line, *column, message.split(" is not").next().unwrap()))
        .collect();
    assert_eq!(
        places,
        [
            (2, 45, "ExcelScript.HorizontalAlignment.centre"),
            (3, 35, "ExcelScript.ChartType.waterfall"),
            (4, 25, "ExcelScript.SlicerSortType")
        ]
    );
    // In the place of a type the namespace holds more than the table.
    silent(
        "  const link: ExcelScript.RangeHyperlink = { address: \"https://example.org\" };\n  let x: ExcelScript.Slicer | undefined;",
    );
}

#[test]
fn where_the_type_is_not_certain_nothing_is_said() {
    // `any`, with or without the word.
    silent("  const sheet: any = workbook.getActiveWorksheet();\n  sheet.nope();");
    silent("  (workbook as any).nope();\n  (workbook as unknown as { nope(): void }).nope();");
    silent("  const anything = JSON.parse(\"{}\");\n  anything.getRange().nope();");
    // A parameter without an annotation, here and in callbacks of arrays
    // the script made itself.
    assert_eq!(
        findings(
            "function main(workbook: ExcelScript.Workbook) {\n  helper(workbook);\n}\nfunction helper(book) {\n  book.nope();\n}\n"
        ),
        Vec::new()
    );
    silent("  const names = [\"a\"].map(name => name.nope);\n  [workbook].forEach(w => w.nope());");
    // What `filter`, `map`, `slice` and the like give.
    silent(
        "  const sheets = workbook.getWorksheets().filter(s => s.getName() !== \"x\");\n  sheets[0].nope();\n  sheets.forEach(s => s.nope());",
    );
    silent(
        "  workbook.getWorksheets().map(s => s)[0].nope();\n  workbook.getWorksheets().slice(1).forEach(s => s.nope());",
    );
    // A binding that is assigned again, a union, a ternary of two kinds.
    silent(
        "  let thing = workbook.getActiveWorksheet();\n  thing = JSON.parse(\"1\");\n  thing.nope();",
    );
    silent(
        "  let either: ExcelScript.Range | ExcelScript.Table = workbook.getTables()[0];\n  either.nope();",
    );
    silent(
        "  const one = Math.random() > 0.5 ? workbook.getActiveWorksheet() : workbook.getTables()[0];\n  one.nope();",
    );
    // Destructuring, fields of objects and of classes.
    silent(
        "  const { a } = { a: workbook };\n  a.nope();\n  const box = { sheet: workbook.getActiveWorksheet() };\n  box.sheet.nope();",
    );
    // A name that shadows a typed one.
    silent(
        "  const sheet = workbook.getActiveWorksheet();\n  { const sheet = { nope() {} }; sheet.nope(); }\n  sheet.getName();",
    );
    // Asking whether a member exists is not calling it.
    silent("  if (typeof workbook.getSlicers === \"function\") { console.log(\"slicers\"); }");
    // What every object answers, and plain values a member returns.
    silent(
        "  workbook.toString();\n  workbook.getName().toUpperCase().nope;\n  workbook.getActiveWorksheet().getRange().getValues().length;",
    );
    // A script of its own `ExcelScript`.
    assert_eq!(
        findings(
            "const ExcelScript = { Nope: { x: 1 } };\nfunction main(workbook) {\n  return ExcelScript.Nope.x;\n}\n"
        ),
        Vec::new()
    );
}

#[test]
fn a_text_that_is_not_typescript_gives_its_syntax_errors_alone() {
    let errors = check("function main(workbook: ExcelScript.Workbook) {\n  workbook.nope(;\n}\n");
    assert!(!errors.is_empty() && errors.iter().all(|error| error.kind == ErrorKind::Syntax));
    let errors = check("function other(workbook: ExcelScript.Workbook) {\n  workbook.nope();\n}\n");
    assert!(errors.len() == 1 && errors[0].kind == ErrorKind::Syntax);
}

/// The names offered where the `|` of a text stands, with the class they
/// are of and the beginning already typed.
fn offered(text: &str) -> (Option<String>, Vec<&'static str>, String) {
    let caret = text.find('|').expect("a caret");
    let source = text.replacen('|', "", 1);
    let found = complete(&source, caret);
    let typed = source[found.start..caret].to_string();
    (
        found.of,
        found.items.iter().map(|item| item.name).collect(),
        typed,
    )
}

#[test]
fn an_editor_is_told_what_can_follow_a_dot() {
    // After a chain, while the line is unfinished.
    let (of, names, typed) = offered(
        "function main(workbook: ExcelScript.Workbook) {\n  const sheet = workbook.getActiveWorksheet();\n  sheet.getRange(\"A1\").getFo|\n}\n",
    );
    assert_eq!((of.as_deref(), typed.as_str()), (Some("Range"), "getFo"));
    assert!(
        names.contains(&"getFormat")
            && names.contains(&"setValues")
            && !names.contains(&"getRange")
    );
    // Right after the dot, in the middle of a statement, after `?.`.
    assert_eq!(
        offered("function main(workbook: ExcelScript.Workbook) {\n  workbook.|\n}\n")
            .0
            .as_deref(),
        Some("Workbook")
    );
    assert_eq!(offered("function main(workbook: ExcelScript.Workbook) {\n  const n = workbook.getTable(\"T\")?.|.length;\n}\n").0.as_deref(), Some("Table"));
    assert_eq!(offered("function main(workbook: ExcelScript.Workbook) {\n  const total = 1 + workbook.getActiveWorksheet().get|;\n}\n").0.as_deref(), Some("Worksheet"));
    // Bindings, elements of arrays, parameters of callbacks.
    let script = "function main(workbook: ExcelScript.Workbook) {\n  for (const table of workbook.getTables()) {\n    table.getColumns().forEach(column => column.|);\n  }\n}\n";
    assert_eq!(offered(script).0.as_deref(), Some("TableColumn"));
    // The namespace and its enums.
    let (of, names, _) =
        offered("function main(workbook: ExcelScript.Workbook) {\n  const a = ExcelScript.|\n}\n");
    assert!(
        of.as_deref() == Some("ExcelScript")
            && names.contains(&"ChartType")
            && names.contains(&"SheetVisibility")
    );
    let found = complete(
        "function main(workbook: ExcelScript.Workbook) {\n  const a = ExcelScript.SheetVisibility.v\n}\n",
        89,
    );
    assert_eq!(found.of.as_deref(), Some("ExcelScript.SheetVisibility"));
    assert_eq!(
        found
            .items
            .iter()
            .map(|item| (item.name, item.kind, item.signature.as_str()))
            .collect::<Vec<_>>(),
        [
            (
                "visible",
                CompletionKind::EnumMember,
                "visible = \"Visible\""
            ),
            ("hidden", CompletionKind::EnumMember, "hidden = \"Hidden\""),
            (
                "veryHidden",
                CompletionKind::EnumMember,
                "veryHidden = \"VeryHidden\""
            ),
        ]
    );
    // What is not known, or not after a dot, offers nothing.
    for text in [
        "function main(workbook: ExcelScript.Workbook) {\n  const x = JSON.parse(\"1\");\n  x.|\n}\n",
        "function main(workbook: ExcelScript.Workbook) {\n  work|\n}\n",
        "function main(workbook: ExcelScript.Workbook) {\n  workbook.getWorksheets().|\n}\n",
        "function main(workbook: ExcelScript.Workbook) {\n  if (((( {\n  workbook.|\n}\n",
        "|",
    ] {
        let (of, names, _) = offered(text);
        assert!(of.is_none() && names.is_empty(), "{text}");
    }
    // A member as it is offered, and the members of a class by its name.
    let members = members_of("Worksheet");
    let cell = members
        .iter()
        .find(|member| member.name == "getCell")
        .expect("getCell");
    assert_eq!(
        (cell.kind, cell.signature.as_str(), cell.returns.as_str()),
        (
            CompletionKind::Method,
            "getCell(row: number, column: number): Range",
            "Range"
        )
    );
    assert!(!cell.description.is_empty());
    assert_eq!(
        members_of("Worksheet").len(),
        opensheet_script::api()
            .iter()
            .find(|class| class.name == "Worksheet")
            .unwrap()
            .members
            .len()
    );
    assert!(members_of("Nope").is_empty());
}
