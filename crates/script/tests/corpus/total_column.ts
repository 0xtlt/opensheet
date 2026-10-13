/**
 * Adds a "Total" under every column of numbers of the data, and an
 * "Amount" column that multiplies quantity by price.
 */
function main(workbook: ExcelScript.Workbook) {
  const sheet = workbook.getActiveWorksheet();
  const data = sheet.getUsedRange(true)!;
  const rows = data.getRowCount();
  const headers = data.getRow(0).getValues()[0] as string[];
  const qty = headers.indexOf("Qty");
  const price = headers.indexOf("Price");

  // Amount = Qty * Price, next to the data.
  const amount = data.getColumnsAfter(1);
  amount.getCell(0, 0).setValue("Amount");
  const first = amount.getCell(1, 0);
  first.setFormula(`=${data.getCell(1, qty).getAddress().split("!")[1]}*${data.getCell(1, price).getAddress().split("!")[1]}`);
  first.autoFill(amount.getCell(1, 0).getResizedRange(rows - 2, 0));

  // Totals under the columns of numbers.
  const totals = data.getLastRow().getOffsetRange(1, 0).getResizedRange(0, 1);
  totals.getCell(0, 0).setValue("Total");
  for (const column of [qty, headers.length]) {
    const cells = data.getColumn(0).getOffsetRange(1, column).getResizedRange(-1, 0);
    totals.getCell(0, column).setFormula(`=SUM(${cells.getAddress().split("!")[1]})`);
  }
  totals.getFormat().getFont().setBold(true);
  totals.getFormat().getRangeBorder(ExcelScript.BorderIndex.edgeTop).setStyle(ExcelScript.BorderLineStyle.double);
  amount.getOffsetRange(1, 0).setNumberFormat("#,##0.00");
  totals.getCell(0, headers.length).setNumberFormat("#,##0.00");
  return totals.getCell(0, headers.length).getValue() as number;
}
