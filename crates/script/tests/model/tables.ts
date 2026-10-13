function main(workbook: ExcelScript.Workbook) {
  const sheet = workbook.getActiveWorksheet();
  const value = (address: string) => sheet.getRange(address).getValue();
  const table = sheet.addTable("A1:C4", true);
  const shown = () => table.getRangeBetweenHeaderAndTotal().getColumn(0).getValues()
    .filter((_, i) => !table.getRangeBetweenHeaderAndTotal().getCell(i, 0).getRowHidden()).map(row => row[0]);
  eq(table.getName(), "Table1");
  eq([workbook.getTables().length, sheet.getTables()[0].getName()], [1, "Table1"]);
  eq(workbook.getTable("table1")?.getId(), table.getId());
  eq(workbook.getTable(table.getId())?.getName(), "Table1");
  eq([sheet.getTable("Nope"), sheet.getTable("Table1")?.getName()], [undefined, "Table1"]);
  eq(sheet.getRange("A1:B2").getTables().length, 1);
  eq(sheet.getRange("A1:B2").getTables(true).length, 0);
  table.setName("Stock");
  throws(() => table.setName("A1"), "not valid");
  eq(table.getWorksheet().getName(), "Sheet1");
  eq(table.getRange().getAddress(), "Sheet1!A1:C4");
  eq(table.getHeaderRowRange().getAddress(), "Sheet1!A1:C1");
  eq(table.getRangeBetweenHeaderAndTotal().getAddress(), "Sheet1!A2:C4");
  throws(() => table.getTotalRowRange(), "no total row");
  eq(table.getRowCount(), 3);
  eq(table.getColumns().map(c => c.getName()), ["Name", "Qty", "Price"]);
  const qty = table.getColumnByName("Qty")!;
  eq([qty.getId(), qty.getIndex()], [2, 1]);
  eq([table.getColumn("price")?.getName(), table.getColumn(1)?.getName(), table.getColumnById(3)?.getName(), table.getColumn("zzz")], ["Price", "Name", "Price", undefined]);
  eq([qty.getRange().getAddress(), qty.getHeaderRowRange().getAddress(), qty.getRangeBetweenHeaderAndTotal().getAddress()], ["Sheet1!B1:B4", "Sheet1!B1", "Sheet1!B2:B4"]);

  // Rows.
  table.addRow(-1, ["Cap", 7, 0.5]);
  eq(table.getRowCount(), 4);
  eq(sheet.getRange("A5:C5").getValues(), [["Cap", 7, 0.5]]);
  table.addRow(0, ["Top", 1, 1]);
  eq([value("A2"), value("A3")], ["Top", "Pen"]);
  table.addRows(undefined, [["X", 1, 1], ["Y", 2, 2]]);
  eq(table.getRowCount(), 7);
  throws(() => table.addRow(-1, [1, 2]), "doesn't match");
  table.deleteRowsAt(0);
  eq(value("A2"), "Pen");
  table.deleteRowsAt(4, 2);
  eq(table.getRowCount(), 4);

  // Columns.
  const total = table.addColumn(-1, ["Total", "=B2*C2", "=B3*C3", "=B4*C4", "=B5*C5"]);
  eq([total.getName(), table.getRange().getAddress(), value("D2")], ["Total", "Sheet1!A1:D5", 15]);
  const note = table.addColumn(1, undefined, "Note");
  eq(table.getColumns().map(c => c.getName()), ["Name", "Note", "Qty", "Price", "Total"]);
  note.setName("Remark");
  eq(table.getColumn("Remark")?.getIndex(), 1);
  note.delete();
  eq(table.getColumns().length, 4);
  sheet.getRange("G1").setFormula("=SUM(Stock[Qty])");
  eq(value("G1"), 24);
  // What is typed right under a table joins it.
  sheet.getRange("A6:C6").setValues([["Nib", 3, 2]]);
  eq([table.getRowCount(), value("G1"), value("D6")], [5, 27, 6]);

  // The check boxes of Table Design.
  const options = () => [table.getShowHeaders(), table.getShowTotals(), table.getShowBandedRows(), table.getShowBandedColumns(),
    table.getHighlightFirstColumn(), table.getHighlightLastColumn(), table.getShowFilterButton()];
  eq(options(), [true, false, true, false, false, false, true]);
  table.setShowTotals(true);
  eq(table.getTotalRowRange().getAddress(), "Sheet1!A7:D7");
  eq(table.getColumn("Total")?.getTotalRowRange().getAddress(), "Sheet1!D7");
  table.setShowBandedRows(false);
  table.setShowBandedColumns(true);
  table.setHighlightFirstColumn(true);
  table.setHighlightLastColumn(true);
  table.setShowFilterButton(false);
  eq(options(), [true, true, false, true, true, true, false]);
  table.setShowTotals(false);
  table.setShowHeaders(false);
  throws(() => table.getHeaderRowRange(), "no header row");
  table.setShowHeaders(true);
  table.setShowFilterButton(true);
  eq(table.getPredefinedTableStyle(), "TableStyleMedium2");
  table.setPredefinedTableStyle("TableStyleLight9");
  eq(table.getPredefinedTableStyle(), "TableStyleLight9");
  throws(() => table.setPredefinedTableStyle("Nope"), "no table style");
  table.resize("A1:D4");
  eq(table.getRowCount(), 3);
  table.resize(sheet.getRange("A1:D6"));
  eq(table.getRowCount(), 5);

  // Sorting and filtering.
  table.getSort().apply([{ key: 1, ascending: false }]);
  eq(shown(), ["Pen", "Cap", "Pad", "Nib", "Ink"]);
  const filter = table.getColumnByName("Qty")!.getFilter();
  filter.applyCustomFilter(">=5");
  eq(shown(), ["Pen", "Cap", "Pad"]);
  eq(table.getAutoFilter().getIsDataFiltered(), true);
  filter.clear();
  eq(table.getAutoFilter().getIsDataFiltered(), false);
  filter.applyValuesFilter(["10", "7"]);
  eq(shown(), ["Pen", "Cap"]);
  table.clearFilters();
  eq(shown().length, 5);
  filter.applyTopItemsFilter(2);
  eq(shown(), ["Pen", "Cap"]);
  filter.applyBottomItemsFilter(1);
  eq(shown(), ["Ink"]);
  filter.applyTopPercentFilter(40);
  eq(shown(), ["Pen", "Cap"]);
  filter.applyBottomPercentFilter(40);
  eq(shown(), ["Nib", "Ink"]);
  filter.applyDynamicFilter(ExcelScript.DynamicFilterCriteria.belowAverage);
  eq(shown(), ["Pad", "Nib", "Ink"]);
  filter.applyCustomFilter("<3", ">6", ExcelScript.FilterOperator.or);
  eq(shown(), ["Pen", "Cap", "Ink"]);
  filter.apply({ filterOn: ExcelScript.FilterOn.values, values: ["5"] });
  eq(shown(), ["Pad"]);
  filter.clear();
  sheet.getRange("A2").getFormat().getFill().setColor("yellow");
  sheet.getRange("A3").getFormat().getFont().setColor("red");
  const byName = table.getColumnByName("Name")!.getFilter();
  byName.applyCellColorFilter("yellow");
  eq(shown(), ["Pen"]);
  byName.applyFontColorFilter("red");
  eq(shown(), ["Cap"]);
  table.clearFilters();
  filter.applyCustomFilter(">=5");
  sheet.getRange("B5").setValue(50);
  eq(shown(), ["Pen", "Cap", "Pad"]);
  table.reapplyFilters();
  eq(shown(), ["Pen", "Cap", "Pad", "Nib"]);
  sheet.getRange("B5").setValue(3);
  table.clearFilters();
  const auto = table.getAutoFilter();
  auto.apply(table.getRange(), 0, { filterOn: ExcelScript.FilterOn.values, values: ["Pen"] });
  eq([shown(), auto.getRange()?.getAddress(), auto.getEnabled()], [["Pen"], "Sheet1!A1:D6", true]);
  auto.clearColumnCriteria(0);
  auto.apply(table.getRange(), 1, { filterOn: ExcelScript.FilterOn.custom, criterion1: ">100" });
  auto.clearCriteria();
  auto.reapply();
  auto.remove();
  eq([table.getShowFilterButton(), shown().length], [false, 5]);
  // Without its buttons a table still sorts.
  table.getSort().apply([{ key: 0 }], true);
  eq(shown(), ["Cap", "Ink", "Nib", "Pad", "Pen"]);

  const second = workbook.addTable("Sheet1!J1:K3", false);
  eq(second.getHeaderRowRange().getValues(), [["Column1", "Column2"]]);
  eq(second.getRange().getAddress(), "Sheet1!J1:K4");
  throws(() => sheet.addTable("A1:B2", true), "overlap");
  eq(second.convertToRange().getAddress(), "Sheet1!J1:K4");
  eq(workbook.getTables().length, 1);
  throws(() => second.getName(), "doesn't exist");
  // A table made of its headers alone takes its first row in the blank one.
  sheet.getRange("M1:N1").setValues([["Key", "Value"]]);
  const fresh = sheet.addTable("M1:N1", true);
  fresh.addRow(-1, ["a", 1]);
  fresh.addRows(-1, [["b", 2], ["c", 3]]);
  eq(fresh.getRange().getAddress(), "Sheet1!M1:N4");
  table.delete();
  eq([workbook.getTables().length, value("A1")], [1, ""]);
}
