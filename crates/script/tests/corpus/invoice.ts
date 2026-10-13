/**
 * Fills an invoice on a new sheet from the lines of the "Orders" table:
 * dates as serial numbers, formulas for the amounts, a named range for
 * the tax rate, number formats and borders.
 */
class Invoice {
  private row = 3;
  constructor(private readonly sheet: ExcelScript.Worksheet, readonly number: string) {
    sheet.getRange("A1").setValue(`Invoice ${number}`);
    sheet.getRange("A1").getFormat().getFont().setSize(16);
    sheet.getRange("A1:D1").merge();
    sheet.getRange("A3:D3").setValues([["Date", "Item", "Qty", "Amount"]]);
  }

  add(date: Date, item: string, qty: number, price: number): void {
    const r = this.row + 1;
    this.sheet.getRange(`A${r}:D${r}`).setValues([[toSerial(date), item, qty, `=C${r}*${price}`]]);
    this.row++;
  }

  close(rate: ExcelScript.NamedItem): number {
    const first = 4;
    const last = this.row;
    const sheet = this.sheet;
    sheet.getRange(`C${last + 2}:D${last + 4}`).setValues([
      ["Net", `=SUM(D${first}:D${last})`],
      ["Tax", `=D${last + 2}*${rate.getName()}`],
      ["Total", `=D${last + 2}+D${last + 3}`],
    ]);
    sheet.getRange(`A${first}:A${last}`).setNumberFormat("yyyy-mm-dd");
    sheet.getRange(`D${first}:D${last + 4}`).setNumberFormat("#,##0.00");
    const lines = sheet.getRange(`A3:D${last}`).getFormat();
    for (const side of [ExcelScript.BorderIndex.edgeTop, ExcelScript.BorderIndex.edgeBottom, ExcelScript.BorderIndex.insideHorizontal]) {
      lines.getRangeBorder(side).setStyle(ExcelScript.BorderLineStyle.continuous);
    }
    sheet.getRange(`C${last + 4}:D${last + 4}`).getFormat().getFont().setBold(true);
    return sheet.getRange(`D${last + 4}`).getValue() as number;
  }
}

/** The serial number Excel keeps a date as. */
function toSerial(date: Date): number {
  return Math.floor(date.getTime() / 86400000) + 25569;
}

function fromSerial(serial: number): Date {
  return new Date((serial - 25569) * 86400000);
}

async function main(workbook: ExcelScript.Workbook) {
  const orders = workbook.getTable("Orders");
  if (!orders) {
    throw new Error("There is no table named Orders.");
  }
  const rate = workbook.getNamedItem("TaxRate") ?? workbook.addNamedItem("TaxRate", "=0.2", "The tax on invoices");
  const sheet = workbook.addWorksheet("Invoice");
  const invoice = new Invoice(sheet, "2024-017");
  for (const [date, item, qty, price] of orders.getRangeBetweenHeaderAndTotal().getValues() as [number, string, number, number][]) {
    invoice.add(fromSerial(date), item, qty, price);
  }
  const total = invoice.close(rate);
  sheet.getRange("A:D").getFormat().autofitColumns();
  sheet.activate();
  return { number: invoice.number, total };
}
