/**
 * Formats the header row of the data on the active sheet, freezes it,
 * and fits the columns.
 */
function main(workbook: ExcelScript.Workbook) {
  const sheet = workbook.getActiveWorksheet();
  const used = sheet.getUsedRange();
  if (!used) {
    console.log("The sheet is empty.");
    return;
  }
  const header = used.getRow(0);
  const format = header.getFormat();
  format.getFill().setColor("#4472C4");
  format.getFont().setColor("white");
  format.getFont().setBold(true);
  format.setHorizontalAlignment(ExcelScript.HorizontalAlignment.center);
  format.getRangeBorder(ExcelScript.BorderIndex.edgeBottom).setStyle(ExcelScript.BorderLineStyle.continuous);
  format.getRangeBorder(ExcelScript.BorderIndex.edgeBottom).setWeight(ExcelScript.BorderWeight.medium);

  sheet.getFreezePanes().freezeRows(1);
  used.getFormat().autofitColumns();
  console.log(`Formatted ${header.getColumnCount()} headers over ${used.getRowCount() - 1} rows.`);
}
