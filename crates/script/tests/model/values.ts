function main(workbook: ExcelScript.Workbook) {
  const sheet = workbook.getActiveWorksheet();
  const all = sheet.getRange("A1:C4");
  eq(all.getAddress(), "Sheet1!A1:C4");
  eq(all.getAddressLocal(), "Sheet1!A1:C4");
  eq([all.getRowCount(), all.getColumnCount(), all.getCellCount(), all.getRowIndex(), all.getColumnIndex()], [4, 3, 12, 0, 0]);
  eq(all.getWorksheet().getName(), "Sheet1");
  eq(all.getValues(), [["Name", "Qty", "Price"], ["Pen", 10, 1.5], ["Ink", 2, 12.25], ["Pad", 5, 3]]);
  eq(all.getValue(), "Name");
  eq(sheet.getRange("B2").getValueType(), "Double");
  eq(sheet.getRange("A1:B1").getValueTypes(), [["String", "String"]]);
  eq(sheet.getRange("Z9").getValueType(), ExcelScript.RangeValueType.empty);
  eq(sheet.getRange("Z9").getValue(), "");

  const total = sheet.getRange("D2:D4");
  total.setFormula("=B2*C2");
  eq(total.getFormulas(), [["=B2*C2"], ["=B3*C3"], ["=B4*C4"]]);
  eq(total.getValues(), [[15], [24.5], [15]]);
  eq(total.getFormula(), "=B2*C2");
  sheet.getRange("D1").setValue("Total");
  sheet.getRange("E1:F2").setValues([["x", 1], [true, null]]);
  eq(sheet.getRange("E1:F2").getValues(), [["x", 1], [true, ""]]);
  eq(sheet.getRange("E2").getValueType(), "Boolean");
  throws(() => sheet.getRange("E1:F2").setValues([[1, 2, 3]]), "doesn't match the size");
  throws(() => sheet.getRange("E1:F2").setValues([[1, 2], [3]]), "doesn't match the size");
  sheet.getRange("E1:F2").setFormulas([["=1+1", "=SUM(B2:B4)"], ["text", "=E1&\"!\""]]);
  eq(sheet.getRange("E1:F2").getValues(), [[2, 17], ["text", "2!"]]);
  eq(sheet.getRange("E1:F2").getFormulas(), [["=1+1", "=SUM(B2:B4)"], ["text", "=E1&\"!\""]]);
  eq(sheet.getRange("F1").getFormulaLocal(), "=SUM(B2:B4)");
  eq(sheet.getRange("E1:F1").getFormulasLocal(), [["=1+1", "=SUM(B2:B4)"]]);
  sheet.getRange("G1").setFormulaLocal("=MAX(B2:B4)");
  sheet.getRange("G2:G3").setFormulasLocal([["=MIN(B2:B4)"], ["=G2+1"]]);
  eq(sheet.getRange("G1:G3").getValues(), [[10], [2], [3]]);
  throws(() => sheet.getRange("H1").setFormula("=SUM("), "not valid");

  // Text is read as typed in an English Excel.
  sheet.getRange("H1").setValue("12/31/2024");
  eq(sheet.getRange("H1").getValue(), 45657);
  eq(sheet.getRange("H1").getText(), "12/31/2024");
  sheet.getRange("H2").setValue("50%");
  eq(sheet.getRange("H2").getValue(), 0.5);
  eq(sheet.getRange("H2").getText(), "50%");
  sheet.getRange("H3").setValue("'007");
  eq(sheet.getRange("H3").getValue(), "007");
  sheet.getRange("H3").setValue("");
  eq(sheet.getRange("H3").getValueType(), "Empty");
  sheet.getRange("H4:H5").setValue(7);
  eq(sheet.getRange("H4:H5").getValues(), [[7], [7]]);

  const price = sheet.getRange("C2:C4");
  price.setNumberFormat("0.00");
  eq(price.getTexts(), [["1.50"], ["12.25"], ["3.00"]]);
  eq(price.getNumberFormat(), "0.00");
  eq(price.getNumberFormats(), [["0.00"], ["0.00"], ["0.00"]]);
  price.setNumberFormats([["0"], ["#,##0.0"], ["0%"]]);
  eq(price.getTexts(), [["2"], ["12.3"], ["300%"]]);
  price.setNumberFormatLocal("0.000");
  eq(price.getNumberFormatLocal(), "0.000");
  eq(price.getNumberFormatsLocal(), [["0.000"], ["0.000"], ["0.000"]]);
  price.setNumberFormatsLocal([["0.0"], ["0.0"], ["General"]]);
  eq(price.getTexts(), [["1.5"], ["12.3"], ["3"]]);
  eq(sheet.getRange("B2").getText(), "10");
  eq(sheet.getRange("B2").getNumberFormat(), "General");

  sheet.getRange("I1").setFormula("=1/0");
  eq(sheet.getRange("I1").getValue(), "#DIV/0!");
  eq(sheet.getRange("I1").getValueType(), "Error");
  sheet.getRange("I1").calculate();
  sheet.calculate(true);
  eq(sheet.getRange("A1:C4").getTables().length, 0);
}
