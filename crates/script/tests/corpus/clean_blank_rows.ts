/**
 * Deletes the rows of the used range that hold nothing, trims the text
 * of the others and turns numbers stored as text into numbers.
 */
function main(workbook: ExcelScript.Workbook) {
  const sheet = workbook.getActiveWorksheet();
  const used = sheet.getUsedRange();
  if (!used) {
    return 0;
  }
  const values = used.getValues();
  let deleted = 0;
  // From the bottom, so the rows above keep their place.
  for (let row = values.length - 1; row >= 0; row--) {
    if (values[row].every(cell => cell === "" || (typeof cell === "string" && cell.trim() === ""))) {
      used.getRow(row).getEntireRow().delete(ExcelScript.DeleteShiftDirection.up);
      deleted++;
    }
  }
  const kept = sheet.getUsedRange();
  if (kept) {
    const cleaned = kept.getValues().map(row =>
      row.map(cell => {
        if (typeof cell !== "string") {
          return cell;
        }
        const text = cell.trim().replace(/\s+/g, " ");
        return text !== "" && !isNaN(Number(text)) ? Number(text) : text;
      })
    );
    kept.setValues(cleaned);
  }
  console.log(`Deleted ${deleted} blank rows.`);
  return deleted;
}
