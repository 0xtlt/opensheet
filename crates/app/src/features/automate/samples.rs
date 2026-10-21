//! The sample scripts of the Automate tab, written for OpenSheet to show
//! what the Office Scripts API can do here. They run from the gallery as
//! they are; editing one makes a copy among the user's own scripts.

use crate::i18n::t;
use opensheet_script::Script;

pub(crate) struct Sample {
    /// What tells the sample from the others, whatever the language.
    pub id: &'static str,
    pub body: &'static str,
}

impl Sample {
    /// The name in the gallery.
    pub fn name(&self) -> &'static str {
        match self.id {
            "header" => t("Format a Header Row", "Mettre en forme une ligne d'en-tête"),
            "total" => t("Add a Total Row", "Ajouter une ligne de total"),
            "table" => t("Create and Sort a Table", "Créer et trier un tableau"),
            "unhide" => t(
                "Unhide All Rows and Columns",
                "Afficher toutes les lignes et colonnes",
            ),
            "summary" => t("Build a Summary Sheet", "Créer une feuille de synthèse"),
            _ => t(
                "Color Cells Above Average",
                "Colorer les cellules au-dessus de la moyenne",
            ),
        }
    }

    pub fn description(&self) -> &'static str {
        match self.id {
            "header" => t(
                "Makes the first row of the sheet a header row and fits the columns to their content.",
                "Transforme la première ligne de la feuille en ligne d'en-tête et ajuste les colonnes à leur contenu.",
            ),
            "total" => t(
                "Adds a row of sums under the list around the selected cell.",
                "Ajoute une ligne de sommes sous la liste qui entoure la cellule sélectionnée.",
            ),
            "table" => t(
                "Makes a table of the range around the selected cell, sorts it, shows its total row and keeps the rows whose last column is above average.",
                "Crée un tableau à partir de la plage qui entoure la cellule sélectionnée, le trie, affiche sa ligne de total et conserve les lignes dont la dernière colonne est au-dessus de la moyenne.",
            ),
            "unhide" => t(
                "Shows the hidden rows and columns of every sheet.",
                "Affiche les lignes et les colonnes masquées de toutes les feuilles.",
            ),
            "summary" => t(
                "Lists the sheets of the workbook and the size of their data on a sheet of its own.",
                "Liste les feuilles du classeur et la taille de leurs données sur une feuille à part.",
            ),
            _ => t(
                "Fills the selected numbers that are above their average.",
                "Colore les nombres sélectionnés qui dépassent leur moyenne.",
            ),
        }
    }

    pub fn script(&self) -> Script {
        Script {
            name: self.name().to_string(),
            description: self.description().to_string(),
            body: self.body.to_string(),
        }
    }
}

pub(crate) const SAMPLES: [Sample; 6] = [
    Sample {
        id: "header",
        body: HEADER,
    },
    Sample {
        id: "total",
        body: TOTAL,
    },
    Sample {
        id: "table",
        body: TABLE,
    },
    Sample {
        id: "unhide",
        body: UNHIDE,
    },
    Sample {
        id: "summary",
        body: SUMMARY,
    },
    Sample {
        id: "average",
        body: AVERAGE,
    },
];

const HEADER: &str = r##"function main(workbook: ExcelScript.Workbook) {
  // The first row of the used range is the header row.
  const sheet = workbook.getActiveWorksheet();
  const used = sheet.getUsedRange();
  if (!used) {
    console.log("The sheet is empty.");
    return;
  }
  const header = used.getRow(0);
  const format = header.getFormat();
  format.getFill().setColor("#1F4E78");
  format.getFont().setColor("#FFFFFF");
  format.getFont().setBold(true);
  format.setHorizontalAlignment(ExcelScript.HorizontalAlignment.center);

  // Every column as wide as what it holds.
  used.getFormat().autofitColumns();
  console.log("Formatted " + header.getAddress() + ".");
}
"##;

const TOTAL: &str = r##"function main(workbook: ExcelScript.Workbook) {
  // The list is the block of cells around the selected one; its first
  // row holds the titles of the columns.
  const list = workbook.getSelectedRange().getSurroundingRegion();
  const rows = list.getRowCount();
  const columns = list.getColumnCount();
  const values = list.getValues();
  if (rows < 2) {
    console.log("Select a cell of a list with a header row first.");
    return;
  }

  // The row right under the list.
  const total = list.getLastRow().getOffsetRange(1, 0);
  total.getColumn(0).setValue("Total");
  let sums = 0;
  for (let c = 1; c < columns; c++) {
    // Only columns of numbers are added up.
    if (typeof values[rows - 1][c] !== "number") {
      continue;
    }
    // The column without its title.
    const data = list.getColumn(c).getOffsetRange(1, 0).getResizedRange(-1, 0);
    total.getColumn(c).setFormula("=SUM(" + data.getAddress() + ")");
    sums++;
  }
  total.getFormat().getFont().setBold(true);
  console.log("Added " + sums + " sums in " + total.getAddress() + ".");
}
"##;

const TABLE: &str = r##"function main(workbook: ExcelScript.Workbook) {
  const sheet = workbook.getActiveWorksheet();
  if (sheet.getTables().length > 0) {
    console.log("This sheet already has a table.");
    return;
  }

  // The block of cells around the selected one, titles in its first row.
  const region = workbook.getSelectedRange().getSurroundingRegion();
  const table = sheet.addTable(region, true);
  table.setPredefinedTableStyle("TableStyleMedium2");

  // Sort on the first column, smallest first, and show the total row.
  table.getSort().apply([{ key: 0, ascending: true }]);
  table.setShowTotals(true);
  table.getTotalRowRange().getFormat().getFont().setBold(true);

  // Keep the rows whose last column is above its average: the total row
  // then adds up what shows.
  const columns = table.getColumns();
  columns[columns.length - 1].getFilter().applyDynamicFilter(ExcelScript.DynamicFilterCriteria.aboveAverage);
  console.log(table.getName() + " covers " + table.getRange().getAddress() + ".");
}
"##;

const UNHIDE: &str = r##"function main(workbook: ExcelScript.Workbook) {
  // On every sheet, the whole sheet is one range.
  const sheets = workbook.getWorksheets();
  for (const sheet of sheets) {
    const everything = sheet.getRange();
    everything.setRowHidden(false);
    everything.setColumnHidden(false);
  }
  console.log("Rows and columns shown on " + sheets.length + " sheet(s).");
}
"##;

const SUMMARY: &str = r##"function main(workbook: ExcelScript.Workbook) {
  // The summary goes on a sheet of its own, made on the first run and
  // emptied on the next ones.
  const name = "Summary";
  let summary = workbook.getWorksheet(name);
  if (!summary) {
    summary = workbook.addWorksheet(name);
  }
  const old = summary.getUsedRange();
  if (old) {
    old.clear();
  }

  // One row for each of the other sheets.
  const rows: (string | number)[][] = [["Sheet", "Used range", "Rows", "Columns"]];
  for (const sheet of workbook.getWorksheets()) {
    if (sheet.getName() === name) {
      continue;
    }
    const used = sheet.getUsedRange();
    if (used) {
      rows.push([sheet.getName(), used.getAddress(), used.getRowCount(), used.getColumnCount()]);
    } else {
      rows.push([sheet.getName(), "", 0, 0]);
    }
  }

  const target = summary.getRange("A1").getResizedRange(rows.length - 1, 3);
  target.setValues(rows);
  target.getRow(0).getFormat().getFont().setBold(true);
  target.getFormat().autofitColumns();
  summary.activate();
  console.log("Listed " + (rows.length - 1) + " sheet(s).");
}
"##;

const AVERAGE: &str = r##"function main(workbook: ExcelScript.Workbook) {
  const range = workbook.getSelectedRange();
  const values = range.getValues();

  // The average of the numbers of the selection.
  let sum = 0;
  let count = 0;
  for (const row of values) {
    for (const value of row) {
      if (typeof value === "number") {
        sum += value;
        count++;
      }
    }
  }
  if (count === 0) {
    console.log("There is no number in the selection.");
    return;
  }
  const average = sum / count;

  // A green fill for each number above it.
  let above = 0;
  for (let r = 0; r < values.length; r++) {
    for (let c = 0; c < values[r].length; c++) {
      const value = values[r][c];
      if (typeof value === "number" && value > average) {
        range.getRow(r).getColumn(c).getFormat().getFill().setColor("#C6EFCE");
        above++;
      }
    }
  }
  console.log("Average: " + average + ". Cells above it: " + above + ".");
}
"##;
