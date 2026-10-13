function main(workbook: ExcelScript.Workbook) {
  const sheet = workbook.getActiveWorksheet();
  const qty = sheet.getRange("B2:B4");
  const dv = qty.getDataValidation();
  eq([dv.getType(), dv.getRule(), dv.getValid(), dv.getInvalidCells()], ["None", {}, true, undefined]);
  eq(dv.getIgnoreBlanks(), true);
  eq(dv.getErrorAlert(), { message: "", showAlert: true, style: "Stop", title: "" });
  eq(dv.getPrompt(), { message: "", showPrompt: true, title: "" });

  // Whole numbers between 3 and 20: Ink (2) breaks the rule.
  dv.setRule({ wholeNumber: { formula1: 3, formula2: "20", operator: ExcelScript.DataValidationOperator.between } });
  eq(dv.getType(), "WholeNumber");
  eq(dv.getRule(), { wholeNumber: { formula1: "3", formula2: "20", operator: "Between" } });
  eq([dv.getValid(), dv.getInvalidCells()?.getAddress()], [false, "Sheet1!B3"]);
  eq(sheet.getRange("B2").getDataValidation().getValid(), true);
  // The checks of the dialog.
  throws(() => dv.setRule({ wholeNumber: { formula1: 1.5, operator: ExcelScript.DataValidationOperator.greaterThan } }), "");
  throws(() => dv.setRule({ wholeNumber: { formula1: 9, formula2: 1, operator: ExcelScript.DataValidationOperator.between } }), "");
  throws(() => dv.setRule({ decimal: { formula1: 9 } as ExcelScript.BasicDataValidation }), "invalid");
  throws(() => dv.setRule({}), "invalid");
  eq(dv.getType(), "WholeNumber");

  dv.setErrorAlert({ message: "From 3 to 20", showAlert: true, style: ExcelScript.DataValidationAlertStyle.warning, title: "Quantity" });
  eq(dv.getErrorAlert(), { message: "From 3 to 20", showAlert: true, style: "Warning", title: "Quantity" });
  dv.setPrompt({ message: "How many?", showPrompt: true, title: "Qty" });
  eq(dv.getPrompt(), { message: "How many?", showPrompt: true, title: "Qty" });
  dv.setIgnoreBlanks(false);
  eq([dv.getIgnoreBlanks(), dv.getType()], [false, "WholeNumber"]);

  // A part of the range, then another rule in it.
  eq(sheet.getRange("B1:B4").getDataValidation().getType(), "Inconsistent");
  const price = sheet.getRange("C2:C4").getDataValidation();
  price.setRule({ decimal: { formula1: "=$B$2/2", operator: ExcelScript.DataValidationOperator.lessThanOrEqualTo } });
  eq(price.getRule(), { decimal: { formula1: "=$B$2/2", operator: "LessThanOrEqualTo" } });
  eq(price.getInvalidCells()?.getAddress(), "Sheet1!C3");
  eq(sheet.getRange("B2:C4").getDataValidation().getType(), "MixedCriteria");
  throws(() => sheet.getRange("B2:C4").getDataValidation().getRule(), "several");
  eq(sheet.getRange("A1:C4").getDataValidation().getInvalidCells()?.getAddress(), "Sheet1!B3:C3");

  // Lists: typed, and from cells.
  const names = sheet.getRange("E2:E3").getDataValidation();
  names.setRule({ list: { inCellDropDown: true, source: "Pen, Ink ,Pad" } });
  eq(names.getRule(), { list: { inCellDropDown: true, source: "Pen,Ink,Pad" } });
  names.setRule({ list: { inCellDropDown: false, source: sheet.getRange("A2:A4") } });
  eq(names.getRule(), { list: { inCellDropDown: false, source: "=Sheet1!$A$2:$A$4" } });
  sheet.getRange("E2").setValue("Pen");
  sheet.getRange("E3").setValue("Cup");
  eq(names.getInvalidCells()?.getAddress(), "Sheet1!E3");
  throws(() => names.setRule({ list: { inCellDropDown: true, source: "=A1:C4" } }), "");

  // Dates, times, lengths, formulas.
  const when = sheet.getRange("F2").getDataValidation();
  when.setRule({ date: { formula1: "1/31/2024", operator: ExcelScript.DataValidationOperator.greaterThan } });
  eq([when.getType(), when.getRule()], ["Date", { date: { formula1: "1/31/2024", operator: "GreaterThan" } }]);
  when.setRule({ time: { formula1: "8:00", formula2: "18:30", operator: ExcelScript.DataValidationOperator.notBetween } });
  eq(when.getType(), "Time");
  when.setRule({ textLength: { formula1: 3, operator: ExcelScript.DataValidationOperator.equalTo } });
  eq(when.getRule(), { textLength: { formula1: "3", operator: "EqualTo" } });
  when.setRule({ custom: { formula: "=LEN(F2)<A1" } });
  eq([when.getType(), when.getRule()], ["Custom", { custom: { formula: "=LEN(F2)<A1" } }]);
  when.clear();
  eq(when.getType(), "None");
  when.clear();
  // A prompt alone is a rule of its own; an alert alone is nothing.
  when.setPrompt({ message: "Free text", showPrompt: true, title: "" });
  eq([when.getType(), when.getPrompt().message], ["None", "Free text"]);
  price.clear();
}
