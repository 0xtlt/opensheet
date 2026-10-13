/**
 * Builds a "Summary" sheet with one row per sheet of the workbook: its
 * name, how many rows of data it has and the total of its Amount column.
 */
interface SheetSummary {
  name: string;
  rows: number;
  total: number;
}

function summarize(sheet: ExcelScript.Worksheet): SheetSummary | undefined {
  const used = sheet.getUsedRange(true);
  if (!used || used.getRowCount() < 2) {
    return undefined;
  }
  const values = used.getValues();
  const amount = values[0].indexOf("Amount");
  if (amount < 0) {
    return undefined;
  }
  const total = values.slice(1).reduce((sum, row) => sum + (typeof row[amount] === "number" ? (row[amount] as number) : 0), 0);
  return { name: sheet.getName(), rows: values.length - 1, total };
}

function main(workbook: ExcelScript.Workbook) {
  const old = workbook.getWorksheet("Summary");
  if (old) {
    old.delete();
  }
  const summaries = workbook.getWorksheets().map(summarize).filter((s): s is SheetSummary => s !== undefined);
  const summary = workbook.addWorksheet("Summary");
  summary.setPosition(0);

  const rows: (string | number)[][] = [["Sheet", "Rows", "Total"]];
  for (const s of summaries) {
    rows.push([s.name, s.rows, s.total]);
  }
  const range = summary.getRangeByIndexes(0, 0, rows.length, 3);
  range.setValues(rows);
  const last = rows.length + 1;
  summary.getRange(`A${last}:C${last}`).setValues([["All", `=SUM(B2:B${last - 1})`, `=SUM(C2:C${last - 1})`]]);
  summary.getRange(`A${last}:C${last}`).getFormat().getFont().setBold(true);
  summary.getRange("A1:C1").getFormat().getFont().setBold(true);
  summary.getRange("A1:C1").getFormat().getFill().setColor("#D9E1F2");
  summary.getRange("C:C").setNumberFormat("#,##0.00");
  for (const s of summaries) {
    const cell = summary.getRange("A1:A" + rows.length).find(s.name, { completeMatch: true });
    cell?.setHyperlink({ documentReference: `'${s.name}'!A1`, screenTip: `Go to ${s.name}` });
  }
  summary.activate();
  console.log(`Summarized ${summaries.length} sheets.`);
  return summaries;
}
