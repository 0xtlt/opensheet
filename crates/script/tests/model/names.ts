function main(workbook: ExcelScript.Workbook) {
  const sheet = workbook.getActiveWorksheet();
  const rate = workbook.addNamedItem("Rate", "=0.2", "VAT");
  const data = workbook.addNamedItem("Data", sheet.getRange("A1:B3"));
  eq(workbook.getNames().map(n => n.getName()), ["Data", "Rate"]);
  eq(data.getRange().getAddress(), "Sheet1!A1:B3");
  eq(data.getFormula(), "=Sheet1!$A$1:$B$3");
  eq(data.getType(), ExcelScript.NamedItemType.range);
  eq(data.getValue(), "Sheet1!$A$1:$B$3");
  eq(data.getScope(), ExcelScript.NamedItemScope.workbook);
  eq(data.getWorksheet(), undefined);
  eq(data.getVisible(), true);
  eq(data.getComment(), "");
  eq(rate.getComment(), "VAT");
  rate.setComment("tax");
  eq(rate.getComment(), "tax");
  eq(rate.getType(), "Double");
  eq(rate.getValue(), 0.2);
  throws(() => rate.getRange(), "does not refer to a range");
  rate.setFormula("=0.25");
  eq(rate.getFormula(), "=0.25");
  sheet.getRange("D1").setFormula("=SUM(Data)*Rate");
  eq(sheet.getRange("D1").getValue(), 21 * 0.25);
  eq(workbook.getNamedItem("rate")?.getName(), "Rate");
  eq(workbook.getNamedItem("Nope"), undefined);
  throws(() => workbook.addNamedItem("Rate", "=1"), "already exists");
  throws(() => workbook.addNamedItem("A1", "=1"), "not valid");
  throws(() => workbook.addNamedItem("Bad", "=SUM("), "invalid");

  const local = sheet.addNamedItem("Local", "A1:A3");
  eq(local.getScope(), "Worksheet");
  eq(local.getWorksheet()?.getName(), "Sheet1");
  eq(local.getRange().getAddress(), "Sheet1!A1:A3");
  eq(sheet.getNames().length, 1);
  eq(sheet.getNamedItem("local")?.getName(), "Local");
  eq(sheet.getNamedItem("Rate"), undefined);
  eq(sheet.getRange("Local").getAddress(), "Sheet1!A1:A3");
  eq(sheet.getRange("Data").getValues(), [[1, 4], [2, 5], [3, 6]]);

  const total = workbook.addNamedItemFormulaLocal("Total", "=SUM(Sheet1!$A$1:$A$3)");
  eq(total.getValue(), 6);
  eq(total.getType(), "Integer");
  const twice = sheet.addNamedItemFormulaLocal("Twice", "=Sheet1!$A$1*2");
  eq(twice.getValue(), 2);
  const label = workbook.addNamedItem("Label", "=\"hello\"");
  eq(label.getType(), "String");
  eq(label.getValue(), "hello");

  rate.delete();
  eq(workbook.getNamedItem("Rate"), undefined);
  throws(() => rate.getName(), "doesn't exist");
  eq(sheet.getRange("D1").getValue(), "#NAME?");

  const app = workbook.getApplication();
  eq(app.getCalculationMode(), "Automatic");
  sheet.getRange("E1").setFormula("=A1+1");
  eq(sheet.getRange("E1").getValue(), 2);
  app.setCalculationMode(ExcelScript.CalculationMode.manual);
  eq(app.getCalculationMode(), ExcelScript.CalculationMode.manual);
  sheet.getRange("A1").setValue(100);
  eq(sheet.getRange("E1").getValue(), 2, "manual calculation waits");
  app.calculate();
  eq(sheet.getRange("E1").getValue(), 101);
  app.calculate(ExcelScript.CalculationType.full);
  app.setCalculationMode(ExcelScript.CalculationMode.automatic);
  eq(total.getValue(), 105);
}
