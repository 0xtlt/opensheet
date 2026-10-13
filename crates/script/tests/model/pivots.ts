function main(workbook: ExcelScript.Workbook) {
  const sheet = workbook.getActiveWorksheet();
  eq([workbook.getPivotTables().length, sheet.getPivotTables().length, workbook.getPivotTable("Sales")], [0, 0, undefined]);
  const pivot = sheet.addPivotTable("Sales", sheet.getRange("A1:D7"), "F5");
  eq([pivot.getName(), pivot.getWorksheet().getName(), workbook.getPivotTables().length], ["Sales", "Sheet1", 1]);
  eq([workbook.getPivotTable("sales")?.getId(), sheet.getPivotTable("Sales")?.getId()], [pivot.getId(), pivot.getId()]);
  eq(pivot.getHierarchies().map(h => h.getName()), ["Region", "Product", "Qty", "Amount"]);
  eq(pivot.getHierarchy("Nope"), undefined);
  throws(() => sheet.addPivotTable("Other", "A1:D7", "G6"), "overlap");
  throws(() => sheet.addPivotTable("Bad", "K1:K2", "M1"), "");

  // Rows, columns, values, filters.
  const region = pivot.getHierarchy("Region")!;
  eq([region.getId(), region.getFields().length, region.getPivotField("Region")?.getName(), region.getPivotField("x")], ["0", 1, "Region", undefined]);
  const rows = pivot.addRowHierarchy(region);
  const product = pivot.addRowHierarchy(pivot.getHierarchy("Product")!);
  eq(pivot.getRowHierarchies().map(h => h.getName()), ["Region", "Product"]);
  eq([rows.getPosition(), product.getPosition(), product.getId()], [0, 1, "1"]);
  product.setPosition(0);
  eq(pivot.getRowHierarchies().map(h => h.getName()), ["Product", "Region"]);
  pivot.removeRowHierarchy(product);
  const columns = pivot.addColumnHierarchy(pivot.getHierarchy("Product")!);
  eq([pivot.getColumnHierarchies().map(h => h.getName()), pivot.getColumnHierarchy("product")?.getPosition(), pivot.getRowHierarchy("Product")], [["Product"], 0, undefined]);
  eq([columns.getFields()[0].getName(), columns.getPivotField("Product")?.getId(), pivot.getRowHierarchy("Region")?.getName()], ["Product", "1", "Region"]);
  const amount = pivot.addDataHierarchy(pivot.getHierarchy("Amount")!);
  eq([amount.getName(), amount.getSummarizeBy(), amount.getPosition(), amount.getField().getName(), amount.getId()], ["Sum of Amount", "Sum", 0, "Amount", "0"]);
  eq(pivot.getDataHierarchies().map(h => h.getName()), ["Sum of Amount"]);

  // The report is written at once.
  const layout = pivot.getLayout();
  eq(layout.getRange().getAddress(), "Sheet1!F5:J10");
  eq(layout.getBodyAndTotalRange()?.getAddress(), "Sheet1!G7:J10");
  eq(layout.getRowLabelRange()?.getAddress(), "Sheet1!F7:F10");
  eq(layout.getColumnLabelRange()?.getAddress(), "Sheet1!G6:J6");
  eq(layout.getFilterAxisRange(), undefined);
  const grand = () => sheet.getRange(layout.getRange().getLastCell().getAddress()).getValue();
  eq(grand(), 78.75);
  eq(sheet.getRange("F7:F10").getValues().map(r => r[0]), ["East", "North", "South", "Grand Total"]);

  // Summarise by, show as, number format, names.
  amount.setSummarizeBy(ExcelScript.AggregationFunction.max);
  eq([amount.getName(), amount.getSummarizeBy(), grand()], ["Max of Amount", "Max", 24.5]);
  amount.setSummarizeBy(ExcelScript.AggregationFunction.automatic);
  eq(amount.getSummarizeBy(), "Sum");
  eq(amount.getShowAs(), { calculation: "None" });
  amount.setShowAs({ calculation: ExcelScript.ShowAsCalculation.percentOfGrandTotal });
  eq([amount.getShowAs().calculation, grand()], ["PercentOfGrandTotal", 1]);
  const regionField = rows.getFields()[0];
  amount.setShowAs({ calculation: ExcelScript.ShowAsCalculation.percentOf, baseField: regionField, baseItem: regionField.getPivotItem("North") });
  const rule = amount.getShowAs();
  eq([rule.calculation, rule.baseField?.getName(), rule.baseItem?.getName()], ["PercentOf", "Region", "North"]);
  throws(() => amount.setShowAs({ calculation: ExcelScript.ShowAsCalculation.runningTotal }), "base field");
  amount.setShowAs({ calculation: ExcelScript.ShowAsCalculation.none });
  eq(amount.getNumberFormat(), "");
  amount.setNumberFormat("#,##0.00");
  eq([amount.getNumberFormat(), sheet.getRange("J10").getText()], ["#,##0.00", "78.75"]);
  amount.setName("Revenue");
  eq([amount.getName(), pivot.getDataHierarchy("Revenue")?.getId(), sheet.getRange("F5").getValue()], ["Revenue", "0", "Revenue"]);
  throws(() => amount.setName("Region"), "already has this name");
  const qty = pivot.addDataHierarchy(pivot.getHierarchy("Qty")!);
  eq([qty.getPosition(), pivot.getDataHierarchies().length], [1, 2]);
  qty.setPosition(0);
  eq(pivot.getDataHierarchies().map(h => h.getName()), ["Sum of Qty", "Revenue"]);
  pivot.getDataHierarchies()[0].setToDefault();
  eq(pivot.getDataHierarchies().map(h => h.getName()), ["Revenue"]);
  const count = pivot.addDataHierarchy(pivot.getHierarchy("Region")!);
  eq(count.getSummarizeBy(), "Count");
  pivot.removeDataHierarchy(count);

  // Fields and items.
  eq([regionField.getId(), regionField.getItems().map(i => i.getName())], ["0", ["East", "North", "South"]]);
  regionField.sortByLabels(ExcelScript.SortBy.descending);
  eq(sheet.getRange("F7:F9").getValues().map(r => r[0]), ["South", "North", "East"]);
  regionField.sortByValues(ExcelScript.SortBy.descending, amount);
  eq(sheet.getRange("F7:F9").getValues().map(r => r[0]), ["North", "South", "East"]);
  regionField.sortByLabels(ExcelScript.SortBy.ascending);
  const north = regionField.getPivotItem("north")!;
  eq([north.getName(), north.getVisible(), north.getIsExpanded(), regionField.getPivotItem("West"), north.getId()], ["North", true, true, undefined, "0"]);
  north.setVisible(false);
  eq([north.getVisible(), regionField.isFiltered(), regionField.isFiltered(ExcelScript.PivotFilterType.manual), regionField.isFiltered(ExcelScript.PivotFilterType.label), grand()], [false, true, true, false, 39.25]);
  north.setVisible(true);
  north.setIsExpanded(false);
  eq(north.getIsExpanded(), false);
  north.setIsExpanded(true);
  north.setName("Nord");
  eq([north.getName(), sheet.getRange("F8").getValue()], ["Nord", "Nord"]);
  throws(() => north.setName("East"), "already has this name");
  north.setName("");
  regionField.setName("Area");
  eq([regionField.getName(), rows.getName(), pivot.getRowHierarchy("Area")?.getId()], ["Area", "Area", "0"]);
  rows.setName("Region");
  eq([regionField.getShowAllItems(), regionField.getSubtotals().automatic, regionField.getSubtotals().sum], [false, true, false]);
  regionField.setShowAllItems(true);
  regionField.setSubtotals({ automatic: false, sum: true, max: true });
  eq([regionField.getShowAllItems(), regionField.getSubtotals().automatic, regionField.getSubtotals().sum, regionField.getSubtotals().count], [true, false, true, false]);
  regionField.setSubtotals({ automatic: true });

  // Filters.
  regionField.applyFilter({ manualFilter: { selectedItems: ["East", north] } });
  eq([regionField.getItems().map(i => i.getVisible()), grand()], [[true, true, false], 56.25]);
  regionField.clearAllFilters();
  regionField.applyFilter({ labelFilter: { condition: ExcelScript.LabelFilterCondition.beginsWith, substring: "S" } });
  eq([regionField.isFiltered(ExcelScript.PivotFilterType.label), grand()], [true, 22.5]);
  regionField.applyFilter({ labelFilter: { condition: ExcelScript.LabelFilterCondition.contains, substring: "o", exclusive: true } });
  eq(grand(), 16.75);
  regionField.clearFilter(ExcelScript.PivotFilterType.label);
  regionField.applyFilter({ valueFilter: { condition: ExcelScript.ValueFilterCondition.greaterThan, comparator: 20, value: "Revenue" } });
  eq([regionField.isFiltered(ExcelScript.PivotFilterType.value), regionField.isFiltered(ExcelScript.PivotFilterType.date), grand()], [true, false, 62]);
  regionField.applyFilter({ valueFilter: { condition: ExcelScript.ValueFilterCondition.topN, threshold: 1, selectionType: ExcelScript.TopBottomSelectionType.items, value: "Revenue" } });
  eq(grand(), 39.5);
  regionField.clearFilter(ExcelScript.PivotFilterType.value);
  regionField.clearFilter(ExcelScript.PivotFilterType.manual);
  regionField.clearFilter(ExcelScript.PivotFilterType.date);
  eq([regionField.isFiltered(), grand()], [false, 78.75]);
  throws(() => regionField.applyFilter({ dateFilter: { condition: "Today" } } as ExcelScript.PivotFilters), "not supported");
  throws(() => regionField.applyFilter({ valueFilter: { condition: ExcelScript.ValueFilterCondition.equals, comparator: 1, value: "Nope" } }), "no value field");

  // A filter field.
  const filter = pivot.addFilterHierarchy(pivot.getHierarchy("Product")!);
  eq([pivot.getColumnHierarchies().length, pivot.getFilterHierarchies().map(h => h.getName()), pivot.getFilterHierarchy("Product")?.getPosition()], [0, ["Product"], 0]);
  eq([filter.getId(), filter.getName(), filter.getFields().length, filter.getPivotField("Product")?.getName(), filter.getEnableMultipleFilterItems()], ["1", "Product", 1, "Product", true]);
  eq([layout.getFilterAxisRange()?.getAddress(), layout.getRange().getAddress()], ["Sheet1!F3:G3", "Sheet1!F5:G9"]);
  filter.setEnableMultipleFilterItems(false);
  eq([filter.getEnableMultipleFilterItems(), sheet.getRange("G3").getValue(), grand()], [false, "Ink", 36.75]);
  filter.setEnableMultipleFilterItems(true);
  eq([filter.getEnableMultipleFilterItems(), grand()], [true, 36.75]);
  filter.getFields()[0].clearAllFilters();
  filter.setName("Item");
  filter.setPosition(0);
  eq(pivot.getFilterHierarchy("Item")?.getName(), "Item");
  filter.setToDefault();
  eq(pivot.getFilterHierarchies().length, 0);
  pivot.removeFilterHierarchy(pivot.addFilterHierarchy(pivot.getHierarchy("Qty")!));
  const again = pivot.addColumnHierarchy(pivot.getHierarchy("Item")!);
  again.setName("Product");
  eq(again.getName(), "Product");
  again.setToDefault();
  pivot.removeColumnHierarchy(pivot.addColumnHierarchy(pivot.getHierarchy("Product")!));
  pivot.getHierarchy("Product")!.setName("Thing");
  eq(pivot.getHierarchies()[1].getName(), "Thing");
  pivot.addRowHierarchy(pivot.getHierarchy("Thing")!);

  // Layout.
  eq([layout.getLayoutType(), layout.getSubtotalLocation(), layout.getShowRowGrandTotals(), layout.getShowColumnGrandTotals(), layout.getShowFieldHeaders()], ["Compact", "AtTop", true, true, true]);
  eq(layout.getRange().getAddress(), "Sheet1!F5:G15");
  layout.setLayoutType(ExcelScript.PivotLayoutType.tabular);
  eq([layout.getLayoutType(), layout.getRange().getAddress()], ["Tabular", "Sheet1!F5:H15"]);
  layout.setSubtotalLocation(ExcelScript.SubtotalLocationType.off);
  eq([layout.getSubtotalLocation(), layout.getRange().getAddress()], ["Off", "Sheet1!F5:H12"]);
  layout.repeatAllItemLabels(true);
  eq(sheet.getRange("F7").getValue(), "East");
  layout.repeatAllItemLabels(false);
  eq(sheet.getRange("F7").getValue(), "");
  layout.displayBlankLineAfterEachItem(true);
  eq(layout.getRange().getAddress(), "Sheet1!F5:H15");
  layout.displayBlankLineAfterEachItem(false);
  layout.setShowColumnGrandTotals(false);
  eq([layout.getShowColumnGrandTotals(), layout.getRange().getAddress()], [false, "Sheet1!F5:H11"]);
  layout.setShowColumnGrandTotals(true);
  layout.setShowRowGrandTotals(false);
  layout.setShowFieldHeaders(false);
  eq([layout.getShowRowGrandTotals(), layout.getShowFieldHeaders()], [false, false]);
  layout.setShowFieldHeaders(true);
  layout.setSubtotalLocation(ExcelScript.SubtotalLocationType.atBottom);
  eq(layout.getSubtotalLocation(), "AtBottom");
  layout.setLayoutType(ExcelScript.PivotLayoutType.outline);
  eq([layout.getFillEmptyCells(), layout.getEmptyCellText()], [true, ""]);
  layout.setFillEmptyCells(false);
  eq(layout.getFillEmptyCells(), false);
  layout.setFillEmptyCells(true);
  layout.setEmptyCellText("-");
  eq([layout.getFillEmptyCells(), layout.getEmptyCellText()], [true, "-"]);

  // Refresh reads the source again.
  sheet.getRange("D2").setValue(115);
  eq(sheet.getRange("K1").setFormula("=SUM(D2:D7)"), undefined);
  pivot.refresh();
  eq([grand(), sheet.getRange("K1").getValue()], [178.75, 178.75]);
  sheet.getRange("D2").setValue(15);
  workbook.refreshAllPivotTables();
  eq(grand(), 78.75);
  sheet.refreshAllPivotTables();
  pivot.setName("Report");
  eq(workbook.getPivotTable("Report")?.getName(), "Report");

  // A second one, of a table, then deleted.
  const other = workbook.addWorksheet("Other");
  const table = other.addTable(other.getRange("A1:B3"), true);
  other.getRange("A1:B3").setValues([["Kind", "N"], ["a", 1], ["b", 2]]);
  const second = workbook.addPivotTable("OfTable", table, "Other!D3");
  second.addRowHierarchy(second.getHierarchy("Kind")!);
  second.addDataHierarchy(second.getHierarchy("N")!);
  eq([other.getRange("E6").getValue(), other.getPivotTables().length, workbook.getPivotTables().length], [3, 1, 2]);
  second.delete();
  eq([other.getRange("E6").getValue(), other.getPivotTables().length], ["", 0]);
  throws(() => second.getName(), "doesn't exist");
}
