/**
 * Copies the rows whose status is "Open" to a sheet named "Open items",
 * made anew each time, with the header and the column widths.
 */
function main(workbook: ExcelScript.Workbook) {
  const source = workbook.getWorksheet("Tasks");
  if (!source) {
    throw new Error("There is no sheet named Tasks.");
  }
  workbook.getWorksheet("Open items")?.delete();
  const target = workbook.addWorksheet("Open items");

  const data = source.getUsedRange()!;
  const values = data.getValues();
  const status = values[0].indexOf("Status");
  target.getRange("A1").copyFrom(data.getRow(0), ExcelScript.RangeCopyType.all);

  let next = 1;
  for (let row = 1; row < values.length; row++) {
    if (values[row][status] === "Open") {
      target.getCell(next, 0).copyFrom(data.getRow(row), ExcelScript.RangeCopyType.all);
      next++;
    }
  }
  for (let column = 0; column < data.getColumnCount(); column++) {
    target.getCell(0, column).getFormat().setColumnWidth(data.getCell(0, column).getFormat().getColumnWidth());
  }
  target.setTabColor("#70AD47");
  target.activate();
  target.getRange("A1").select();
  return next - 1;
}
