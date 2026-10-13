/**
 * Renames the sheets after the title in their cell A1, puts them in
 * alphabetical order and colours the tab of those that hold data.
 */
function main(workbook: ExcelScript.Workbook) {
  const sheets = workbook.getWorksheets();
  for (const sheet of sheets) {
    const title = String(sheet.getRange("A1").getValue()).trim();
    if (title !== "" && title !== sheet.getName() && !workbook.getWorksheet(title)) {
      sheet.setName(title.slice(0, 31));
    }
  }
  const sorted = [...sheets].sort((a, b) => a.getName().localeCompare(b.getName()));
  sorted.forEach((sheet, index) => {
    sheet.setPosition(index);
    sheet.setTabColor(sheet.getUsedRange(true) ? "#5B9BD5" : "");
  });
  workbook.getFirstWorksheet().activate();
  return workbook.getWorksheets().map(sheet => sheet.getName());
}
