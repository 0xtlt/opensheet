/**
 * Returns the rows of the first table as objects keyed by the names of
 * its columns, as flows read them.
 */
interface TableRow {
  [column: string]: string | number | boolean;
}

function main(workbook: ExcelScript.Workbook): TableRow[] {
  const table = workbook.getTables()[0];
  if (!table) {
    throw new Error("The workbook has no table.");
  }
  const names = table.getColumns().map(column => column.getName());
  const texts = table.getRangeBetweenHeaderAndTotal().getTexts();
  const values = table.getRangeBetweenHeaderAndTotal().getValues();
  return values.map((row, r) => {
    const item: TableRow = {};
    row.forEach((value, c) => {
      // Dates travel as text, the way the cells show them.
      item[names[c]] = names[c] === "Date" ? texts[r][c] : value;
    });
    return item;
  });
}
