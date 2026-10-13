/**
 * Makes a table of the data, sorts it by amount (largest first), keeps
 * the rows of one region and gives it a style.
 */
function main(workbook: ExcelScript.Workbook) {
  const sheet = workbook.getActiveWorksheet();
  const table = sheet.addTable(sheet.getUsedRange()!, true);
  table.setName("Sales");
  table.setPredefinedTableStyle("TableStyleMedium9");
  table.setShowBandedRows(true);

  const amount = table.getColumnByName("Amount");
  if (!amount) {
    throw new Error("The data has no Amount column.");
  }
  table.getSort().apply([{ key: amount.getIndex(), ascending: false }]);
  table.getColumnByName("Region")!.getFilter().applyValuesFilter(["North", "East"]);

  table.setShowTotals(true);
  const total = amount.getTotalRowRange();
  total.setFormula("=SUBTOTAL(109,[Amount])");
  amount.getRange().setNumberFormat("#,##0");
  const visible = table.getRangeBetweenHeaderAndTotal().getValues().filter((_, row) => !table.getRangeBetweenHeaderAndTotal().getRow(row).getRowHidden());
  console.log(`${visible.length} of ${table.getRowCount()} rows shown, total ${total.getText()}`);
}
