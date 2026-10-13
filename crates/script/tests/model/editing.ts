function main(workbook: ExcelScript.Workbook) {
  const sheet = workbook.getActiveWorksheet();
  const value = (address: string) => sheet.getRange(address).getValue();

  // Rows, columns and cells that make room or close up.
  sheet.getRange("2:2").insert(ExcelScript.InsertShiftDirection.down);
  eq([value("A2"), value("A3")], ["", "Pen"]);
  sheet.getRange("2:2").delete(ExcelScript.DeleteShiftDirection.up);
  eq(value("A2"), "Pen");
  sheet.getRange("B:B").insert(ExcelScript.InsertShiftDirection.right);
  eq([value("B1"), value("C1")], ["", "Qty"]);
  sheet.getRange("B:B").delete(ExcelScript.DeleteShiftDirection.left);
  eq(value("B1"), "Qty");
  const made = sheet.getRange("A2:A3").insert(ExcelScript.InsertShiftDirection.down);
  eq(made.getAddress(), "Sheet1!A2:A3");
  eq([value("A2"), value("A4"), value("B2")], ["", "Pen", 10]);
  sheet.getRange("A2:A3").delete(ExcelScript.DeleteShiftDirection.up);
  eq(value("A2"), "Pen");
  sheet.getRange("A2").insert(ExcelScript.InsertShiftDirection.right);
  eq([value("A2"), value("B2"), value("D2")], ["", "Pen", 1.5]);
  sheet.getRange("A2").delete(ExcelScript.DeleteShiftDirection.left);
  eq(sheet.getRange("A2:C2").getValues(), [["Pen", 10, 1.5]]);

  // Copies.
  sheet.getRange("E1").copyFrom("A1:C3");
  eq(sheet.getRange("E1:G3").getValues(), [["Name", "Qty", "Price"], ["Pen", 10, 1.5], ["Ink", 2, 12.25]]);
  sheet.getRange("E5").copyFrom(sheet.getRange("A1:C2"), ExcelScript.RangeCopyType.values, false, true);
  eq(sheet.getRange("E5:F7").getValues(), [["Name", "Pen"], ["Qty", 10], ["Price", 1.5]]);
  sheet.getRange("D2").setFormula("=B2*C2");
  sheet.getRange("H2").copyFrom("D2", ExcelScript.RangeCopyType.formulas);
  eq(sheet.getRange("H2").getFormula(), "=F2*G2");
  sheet.getRange("H3").copyFrom("D2", ExcelScript.RangeCopyType.values);
  eq([sheet.getRange("H3").getFormula(), value("H3")], [15, 15]);
  sheet.getRange("A1").getFormat().getFont().setBold(true);
  sheet.getRange("J1").copyFrom("A1", ExcelScript.RangeCopyType.formats);
  eq([sheet.getRange("J1").getFormat().getFont().getBold(), value("J1")], [true, ""]);
  sheet.getRange("K1").copyFrom("A1", ExcelScript.RangeCopyType.link);
  eq([sheet.getRange("K1").getFormula(), value("K1")], ["=$A$1", "Name"]);
  sheet.getRange("K3:K4").setValues([["keep"], ["keep"]]);
  sheet.getRange("K3").copyFrom("Z1:Z2", ExcelScript.RangeCopyType.all, true);
  eq(value("K4"), "keep");
  const second = workbook.addWorksheet("Copy");
  second.getRange("A1").copyFrom(sheet.getRange("A1:C4"));
  eq(second.getRange("B2").getValue(), 10);
  eq(second.getRange("A1").getFormat().getFont().getBold(), true);

  // Cut and paste: formulas follow.
  sheet.getRange("M1").setFormula("=B2+1");
  sheet.getRange("B2").moveTo("B8");
  eq([value("B2"), value("B8"), sheet.getRange("M1").getFormula()], ["", 10, "=B8+1"]);
  sheet.getRange("B8").moveTo(sheet.getRange("B2"));
  eq(sheet.getRange("M1").getFormula(), "=B2+1");

  // Clearing.
  sheet.getRange("E1:K7").clear(ExcelScript.ClearApplyTo.contents);
  eq(sheet.getRange("E1:K7").getUsedRange(true), undefined);
  eq(sheet.getRange("J1").getFormat().getFont().getBold(), true);
  sheet.getRange("J1").clear(ExcelScript.ClearApplyTo.formats);
  eq(sheet.getRange("J1").getFormat().getFont().getBold(), false);
  sheet.getRange("M1").clear();
  eq(value("M1"), "");
  sheet.getRange("D2").clear(ExcelScript.ClearApplyTo.all);

  // Merged cells.
  sheet.getRange("N1:O2").merge();
  sheet.getRange("N4:O5").merge(true);
  sheet.getRange("N7:O8").merge();
  sheet.getRange("N7").unmerge();
}
