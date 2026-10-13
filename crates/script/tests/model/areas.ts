function main(workbook: ExcelScript.Workbook) {
  const sheet = workbook.getActiveWorksheet();
  // Name Qty Price over A1:C4; D2:D4 holds formulas, one of them an error.
  sheet.getRange("D2:D4").setFormulas([["=B2*C2"], ["=B3/0"], ["=A4&\"!\""]]);
  sheet.getRange("B3").clear(ExcelScript.ClearApplyTo.contents);
  sheet.getRange("E2").setValue(true);
  const data = sheet.getRange("A1:E4");

  // Several ranges held together.
  const areas = sheet.getRanges("A1:B2, D4");
  eq([areas.getAddress(), areas.getAddressLocal(), areas.getAreaCount(), areas.getCellCount(), areas.getWorksheet().getName()], ["Sheet1!A1:B2,Sheet1!D4", "Sheet1!A1:B2,Sheet1!D4", 2, 5, "Sheet1"]);
  eq(areas.getAreas().map(r => r.getAddress()), ["Sheet1!A1:B2", "Sheet1!D4"]);
  eq([areas.getIsEntireColumn(), areas.getIsEntireRow(), areas.getEntireColumn().getIsEntireColumn(), areas.getEntireRow().getIsEntireRow()], [false, false, true, true]);
  eq([areas.getEntireColumn().getAddress(), areas.getEntireRow().getAddress()], ["Sheet1!A:B,Sheet1!D:D", "Sheet1!1:2,Sheet1!4:4"]);
  eq(areas.getOffsetRangeAreas(1, 1).getAddress(), "Sheet1!B2:C3,Sheet1!E5");
  throws(() => areas.getOffsetRangeAreas(-1, 0), "invalid");
  eq([areas.getIntersection("B1:D9")?.getAddress(), areas.getIntersection(sheet.getRanges("A2, D4:D5"))?.getAddress(), areas.getIntersection(sheet.getRange("F1"))], ["Sheet1!B1:B2,Sheet1!D4", "Sheet1!A2,Sheet1!D4", undefined]);
  eq([sheet.getRanges("A1:Z1, K9:L10").getUsedRangeAreas()?.getAddress(), sheet.getRanges("K9:L10").getUsedRangeAreas(true)], ["Sheet1!A1:C1", undefined]);
  eq(sheet.getRanges().getIsEntireColumn(), true);
  throws(() => sheet.getRanges("A1, nope!"), "invalid");

  // Special cells, as Go To Special finds them.
  eq(data.getSpecialCells(ExcelScript.SpecialCellType.blanks)?.getAddress(), "Sheet1!D1:E1,Sheet1!B3,Sheet1!E3:E4");
  eq(data.getSpecialCells(ExcelScript.SpecialCellType.constants)?.getAddress(), "Sheet1!A1:C2,Sheet1!E2,Sheet1!A3,Sheet1!C3,Sheet1!A4:C4");
  eq(data.getSpecialCells(ExcelScript.SpecialCellType.constants, ExcelScript.SpecialCellValueType.numbers)?.getAddress(), "Sheet1!B2:C2,Sheet1!C3,Sheet1!B4:C4");
  eq(data.getSpecialCells(ExcelScript.SpecialCellType.constants, ExcelScript.SpecialCellValueType.logicalText)?.getAddress(), "Sheet1!A1:C1,Sheet1!A2:A4,Sheet1!E2");
  eq(data.getSpecialCells(ExcelScript.SpecialCellType.formulas)?.getAddress(), "Sheet1!D2:D4");
  eq(data.getSpecialCells(ExcelScript.SpecialCellType.formulas, ExcelScript.SpecialCellValueType.errors)?.getAddress(), "Sheet1!D3");
  eq(data.getSpecialCells(ExcelScript.SpecialCellType.formulas, ExcelScript.SpecialCellValueType.numbersText)?.getAddress(), "Sheet1!D2,Sheet1!D4");
  eq(sheet.getRange("F1:G2").getSpecialCells(ExcelScript.SpecialCellType.constants), undefined);
  eq(areas.getSpecialCells(ExcelScript.SpecialCellType.formulas)?.getAddress(), "Sheet1!D4");
  eq(data.getSpecialCells(ExcelScript.SpecialCellType.conditionalFormats), undefined);
  sheet.getRange("B2:B9").addConditionalFormat(ExcelScript.ConditionalFormatType.dataBar);
  sheet.getRange("C2:C3").getDataValidation().setRule({ decimal: { formula1: 0, operator: ExcelScript.DataValidationOperator.greaterThan } });
  eq(data.getSpecialCells(ExcelScript.SpecialCellType.conditionalFormats)?.getAddress(), "Sheet1!B2:B4");
  eq([sheet.getRange("B4:C4").getSpecialCells(ExcelScript.SpecialCellType.sameConditionalFormat)?.getAddress(), sheet.getRange("A1:B4").getSpecialCells(ExcelScript.SpecialCellType.sameConditionalFormat)], ["Sheet1!B4", undefined]);
  eq([data.getSpecialCells(ExcelScript.SpecialCellType.dataValidations)?.getAddress(), sheet.getRange("C3:C9").getSpecialCells(ExcelScript.SpecialCellType.sameDataValidation)?.getAddress()], ["Sheet1!C2:C3", "Sheet1!C3"]);

  // Formatting several ranges at once: the usual end of getSpecialCells.
  const blanks = data.getSpecialCells(ExcelScript.SpecialCellType.blanks)!;
  blanks.getFormat().getFill().setColor("yellow");
  blanks.getFormat().getFont().setBold(true);
  blanks.getFormat().getRangeBorder(ExcelScript.BorderIndex.edgeBottom).setColor("red");
  eq(["D1", "B3", "E4", "A1"].map(a => sheet.getRange(a).getFormat().getFill().getColor()), ["#FFFF00", "#FFFF00", "#FFFF00", "#FFFFFF"]);
  eq([sheet.getRange("E4").getFormat().getFont().getBold(), sheet.getRange("E4").getFormat().getRangeBorder(ExcelScript.BorderIndex.edgeBottom).getColor()], [true, "#FF0000"]);
  eq(blanks.getFormat().getFill().getColor(), "#FFFF00");
  sheet.getRanges("D1:E1, E3:E4").clear(ExcelScript.ClearApplyTo.formats);
  eq(sheet.getRange("E4").getFormat().getFill().getColor(), "#FFFFFF");
  sheet.getRanges("E2, A9").clear();
  eq(sheet.getRange("E2").getValue(), "");
  areas.calculate();

  // Find All.
  eq(data.findAll("P", { completeMatch: false, matchCase: true })?.getAddress(), "Sheet1!C1,Sheet1!A2,Sheet1!A4");
  eq(sheet.findAll("pen", { completeMatch: true, matchCase: false })?.getAddress(), "Sheet1!A2");
  eq([sheet.findAll("zzz", {}), sheet.getRange("B1:B4").findAll("P", {})], [undefined, undefined]);

  // Merged cells.
  eq(data.getMergedAreas(), undefined);
  sheet.getRange("G1:H2").merge();
  sheet.getRange("G5:G6").merge();
  eq([sheet.getRange("A1:H3").getMergedAreas()?.getAddress(), sheet.getRange("G1:G9").getMergedAreas()?.getAreaCount()], ["Sheet1!G1:H2", 2]);

  // What shows: row 3 is hidden, and column B.
  sheet.getRange("3:3").setRowHidden(true);
  sheet.getRange("B:B").setColumnHidden(true);
  eq(data.getSpecialCells(ExcelScript.SpecialCellType.visible)?.getAddress(), "Sheet1!A1:A2,Sheet1!C1:E2,Sheet1!A4,Sheet1!C4:E4");
  const view = sheet.getRange("A1:D4").getVisibleView();
  eq([view.getRowCount(), view.getColumnCount(), view.getIndex(), view.getRange().getAddress()], [3, 3, -1, "Sheet1!A1:D4"]);
  eq(view.getValues(), [["Name", "Price", ""], ["Pen", 1.5, 15], ["Pad", 3, "Pad!"]]);
  eq(view.getText()[1], ["Pen", "1.5", "15"]);
  eq(view.getFormulas()[2], ["Pad", 3, "=A4&\"!\""]);
  eq([view.getNumberFormat()[0][0], view.getValueTypes()[1]], ["General", ["String", "Double", "Double"]]);
  eq(view.getCellAddresses()[2], ["Sheet1!A4", "Sheet1!C4", "Sheet1!D4"]);
  const rows = view.getRows();
  eq([rows.length, rows[2].getIndex(), rows[2].getRowCount(), rows[2].getRange().getAddress(), rows[2].getValues()], [3, 2, 1, "Sheet1!A4:D4", [["Pad", 3, "Pad!"]]]);
  eq(rows[2].getRows().length, 1);
}
