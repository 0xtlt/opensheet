use opensheet_engine::locale::Locale;
use opensheet_engine::workbook::Workbook;
use opensheet_script::{Context, run};

#[test]
fn smoke() {
    let mut wb = Workbook::new(Locale::en());
    wb.snapshot();
    let out = run(
        "async function main(workbook: ExcelScript.Workbook) {\n  const s = workbook.getActiveWorksheet();\n  console.log('hi', [1, 'a', {b: 2}], s.getName());\n  await 1;\n  s.getFoo();\n  return 3;\n}",
        &mut wb,
        &Context::default(),
    );
    panic!("{out:#?}");
}
