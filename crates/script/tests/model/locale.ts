// A French workbook: the API speaks English, the Local members French.
function main(workbook: ExcelScript.Workbook) {
  const sheet = workbook.getActiveWorksheet();
  sheet.getRange("B1").setFormula("=SUM(A1:A2)");
  eq(sheet.getRange("B1").getFormulaLocal(), "=SOMME(A1:A2)");
  eq(sheet.getRange("B1").getFormula(), "=SUM(A1:A2)");
  eq(sheet.getRange("B1").getValue(), 4);
  sheet.getRange("B2").setFormulaLocal("=MOYENNE(A1;A2)");
  eq(sheet.getRange("B2").getFormula(), "=AVERAGE(A1,A2)");
  eq(sheet.getRange("B1:B2").getFormulasLocal(), [["=SOMME(A1:A2)"], ["=MOYENNE(A1;A2)"]]);
  sheet.getRange("C1").setValue("1.25");
  eq(sheet.getRange("C1").getValue(), 1.25);
  sheet.getRange("C1").setNumberFormatLocal("0,000");
  eq(sheet.getRange("C1").getNumberFormat(), "0.000");
  eq(sheet.getRange("C1").getNumberFormatLocal(), "0,000");
  eq(sheet.getRange("C1").getText(), "1,250");
  sheet.getRange("C2").setValue("12/31/2024");
  eq(sheet.getRange("C2").getValue(), 45657);
  sheet.getRange("C3").setFormula("=IF(A1>1,TRUE,FALSE)");
  eq(sheet.getRange("C3").getValue(), true);
  eq(sheet.getRange("C3").getText(), "VRAI");
  eq(workbook.addNamedItem("Somme", "=SUM(Feuil1!$A$1:$A$2)").getValue(), 4);
  eq(workbook.addNamedItemFormulaLocal("Moyenne", "=MOYENNE(Feuil1!$A$1:$A$2)").getFormula(), "=AVERAGE(Feuil1!$A$1:$A$2)");
}
