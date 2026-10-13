/**
 * Colours the cells of the Score column: red under 50, yellow under 80,
 * green from 80, and writes a grade beside each.
 */
enum Grade {
  Fail = "Fail",
  Pass = "Pass",
  Merit = "Merit",
}

const COLORS: Record<Grade, string> = {
  [Grade.Fail]: "#F8CBAD",
  [Grade.Pass]: "#FFE699",
  [Grade.Merit]: "#C6E0B4",
};

function grade(score: number): Grade {
  if (score < 50) return Grade.Fail;
  return score < 80 ? Grade.Pass : Grade.Merit;
}

function main(workbook: ExcelScript.Workbook) {
  const sheet = workbook.getActiveWorksheet();
  const data = sheet.getRange("A1").getSurroundingRegion();
  const headers = data.getRow(0).getValues()[0];
  const column = headers.indexOf("Score");
  const scores = data.getColumn(column);
  const out = data.getColumnsAfter(1);
  out.getCell(0, 0).setValue("Grade");
  out.getCell(0, 0).copyFrom(scores.getCell(0, 0), ExcelScript.RangeCopyType.formats);

  const counts: { [key: string]: number } = {};
  scores.getValues().forEach((row, index) => {
    const score = row[0];
    if (index === 0 || typeof score !== "number") {
      return;
    }
    const g = grade(score);
    counts[g] = (counts[g] ?? 0) + 1;
    scores.getCell(index, 0).getFormat().getFill().setColor(COLORS[g]);
    out.getCell(index, 0).setValue(g);
    if (g === Grade.Fail) {
      scores.getCell(index, 0).getFormat().getFont().setBold(true);
    }
  });
  console.log(counts);
  return counts;
}
