function main(workbook: ExcelScript.Workbook) {
  const sheet = workbook.getActiveWorksheet();
  const value = (address: string) => sheet.getRange(address).getValue();
  const names = () => sheet.getRange("A2:A4").getValues().map(row => row[0]);
  const list = sheet.getRange("A1:C4");

  // Sorting.
  list.getSort().apply([{ key: 1, ascending: false }], false, true);
  eq(names(), ["Pen", "Pad", "Ink"]);
  list.getSort().apply([{ key: 0 }], false, true);
  eq(names(), ["Ink", "Pad", "Pen"]);
  sheet.getRange("A2:C4").getSort().apply([{ key: 2, sortOn: ExcelScript.SortOn.value, ascending: true }]);
  eq(names(), ["Pen", "Pad", "Ink"]);
  sheet.getRange("A3").getFormat().getFill().setColor("yellow");
  list.getSort().apply([{ key: 0, sortOn: ExcelScript.SortOn.cellColor, color: "yellow" }], false, true);
  eq(names(), ["Pad", "Pen", "Ink"]);
  throws(() => list.getSort().apply([{ key: 7 }]), "invalid");
  throws(() => list.getSort().apply([{ key: 0, sortOn: ExcelScript.SortOn.icon }]), "ExcelScript.SortOn.icon is not supported");
  sheet.getRange("E1:G2").setValues([[3, 1, 2], ["c", "a", "b"]]);
  sheet.getRange("E1:G2").getSort().apply([{ key: 0 }], false, false, ExcelScript.SortOrientation.columns);
  eq(sheet.getRange("E2:G2").getValues(), [["a", "b", "c"]]);

  // Duplicates.
  sheet.getRange("I1:I5").setValues([["k"], ["a"], ["b"], ["a"], ["B"]]);
  const removed = sheet.getRange("I1:I5").removeDuplicates([0], true);
  eq([removed.getRemoved(), removed.getUniqueRemaining()], [2, 2]);
  eq(sheet.getRange("I1:I5").getValues(), [["k"], ["a"], ["b"], [""], [""]]);

  // Searching.
  eq(list.find("ink", { completeMatch: true, matchCase: false })?.getAddress(), "Sheet1!A4");
  eq(list.find("ink", { matchCase: true }), undefined);
  eq(list.find("P", {})?.getAddress(), "Sheet1!C1");
  eq(list.find("P", { searchDirection: ExcelScript.SearchDirection.backwards })?.getAddress(), "Sheet1!A3");
  eq(sheet.getRange("A1:A4").replaceAll("P", "Z", { matchCase: true }), 2);
  eq(names(), ["Zad", "Zen", "Ink"]);
  eq(sheet.replaceAll("Zen", "Pen", { completeMatch: true }), 1);
  eq(value("A3"), "Pen");

  // The fill handle.
  sheet.getRange("K1:K2").setValues([[1], [2]]);
  sheet.getRange("K1:K2").autoFill("K1:K6");
  eq(value("K6"), 6);
  sheet.getRange("L1").setFormula("=K1*2");
  sheet.getRange("L1").autoFill(sheet.getRange("L1:L6"), ExcelScript.AutoFillType.fillDefault);
  eq([value("L6"), sheet.getRange("L6").getFormula()], [12, "=K6*2"]);
  throws(() => sheet.getRange("L1").autoFill("M1:M3"), "must contain the range");
  throws(() => sheet.getRange("L1").autoFill("L1:L9", ExcelScript.AutoFillType.fillSeries), "ExcelScript.AutoFillType.fillSeries is not supported");
  throws(() => sheet.getRange("L1").autoFill(), "not supported");

  // Hyperlinks.
  const cell = sheet.getRange("N1");
  eq(cell.getHyperlink(), undefined);
  cell.setHyperlink({ address: "https://example.org", screenTip: "Go", textToDisplay: "Site" });
  eq(cell.getHyperlink(), { address: "https://example.org", screenTip: "Go", textToDisplay: "Site" });
  eq(value("N1"), "Site");
  sheet.getRange("N2").setHyperlink({ documentReference: "Sheet1!A1" });
  eq(sheet.getRange("N2").getHyperlink(), { documentReference: "Sheet1!A1", textToDisplay: "Sheet1!A1" });
  cell.clear(ExcelScript.ClearApplyTo.hyperlinks);
  eq([cell.getHyperlink(), value("N1")], [undefined, "Site"]);
  sheet.getRange("N2").clear(ExcelScript.ClearApplyTo.removeHyperlinks);
  eq(sheet.getRange("N2").getHyperlink(), undefined);

  // Hidden rows and columns.
  sheet.getRange("3:4").setRowHidden(true);
  eq([sheet.getRange("A3:A4").getRowHidden(), sheet.getRange("A2:A4").getRowHidden(), sheet.getRange("A3").getHidden()], [true, false, true]);
  sheet.getRange("4:4").setRowHidden(false);
  eq(sheet.getRange("A4").getHidden(), false);
  sheet.getRange("B:C").setColumnHidden(true);
  eq([sheet.getRange("B1:C1").getColumnHidden(), sheet.getRange("B2").getHidden(), sheet.getRange("A2:B2").getHidden()], [true, true, false]);
  sheet.getRange("C:C").setColumnHidden(false);
  eq(sheet.getRange("C1").getColumnHidden(), false);
}
