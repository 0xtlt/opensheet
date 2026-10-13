// Calls every member of the model with arguments no script should give:
// each call may fail, none may bring the engine down.
type Anything = { [member: string]: (...args: unknown[]) => unknown };

function build(workbook: ExcelScript.Workbook, round: number): { [name: string]: unknown } {
  const sheet = workbook.addWorksheet();
  sheet.getRange("A1:C4").setValues([["Name", "Qty", "Price"], ["Pen", 10, 1.5], ["Ink", 2, 12.25], ["Pad", 5, 3]]);
  sheet.getRange("E1:G4").setValues([["Name", "Qty", "Price"], ["Pen", 10, 1.5], ["Ink", 2, 12.25], ["Pad", 5, 3]]);
  sheet.getRange("I1:I3").setValues([[1], [1], [2]]);
  const range = sheet.getRange("A1:C4");
  const table = sheet.addTable("E1:G4", true);
  const chart = sheet.addChart(ExcelScript.ChartType.columnClustered, range);
  const column = table.getColumns()[1];
  const rule = (type: ExcelScript.ConditionalFormatType) => range.addConditionalFormat(type);
  const cellValue = rule(ExcelScript.ConditionalFormatType.cellValue);
  const dataBar = rule(ExcelScript.ConditionalFormatType.dataBar).getDataBar()!;
  const custom = rule(ExcelScript.ConditionalFormatType.custom).getCustom()!;
  const ruleFormat = cellValue.getCellValue()!.getFormat();
  return {
    Workbook: workbook,
    Application: workbook.getApplication(),
    NamedItem: workbook.addNamedItem(`Name${round}`, range),
    Worksheet: sheet,
    WorksheetFreezePanes: sheet.getFreezePanes(),
    AutoFilter: sheet.getAutoFilter(),
    Range: range,
    RangeSort: range.getSort(),
    RemoveDuplicatesResult: sheet.getRange("I1:I3").removeDuplicates([0], false),
    RangeFormat: range.getFormat(),
    RangeFill: range.getFormat().getFill(),
    RangeFont: range.getFormat().getFont(),
    RangeBorder: range.getFormat().getRangeBorder(ExcelScript.BorderIndex.insideHorizontal),
    FormatProtection: range.getFormat().getProtection(),
    Table: table,
    TableColumn: column,
    Filter: column.getFilter(),
    TableSort: table.getSort(),
    Chart: chart,
    ChartTitle: chart.getTitle(),
    ChartLegend: chart.getLegend(),
    RangeAreas: cellValue.getRanges(),
    ConditionalFormat: cellValue,
    CellValueConditionalFormat: cellValue.getCellValue(),
    ColorScaleConditionalFormat: rule(ExcelScript.ConditionalFormatType.colorScale).getColorScale(),
    DataBarConditionalFormat: dataBar,
    ConditionalDataBarPositiveFormat: dataBar.getPositiveFormat(),
    ConditionalDataBarNegativeFormat: dataBar.getNegativeFormat(),
    IconSetConditionalFormat: rule(ExcelScript.ConditionalFormatType.iconSet).getIconSet(),
    PresetCriteriaConditionalFormat: rule(ExcelScript.ConditionalFormatType.presetCriteria).getPreset(),
    TextConditionalFormat: rule(ExcelScript.ConditionalFormatType.containsText).getTextComparison(),
    TopBottomConditionalFormat: rule(ExcelScript.ConditionalFormatType.topBottom).getTopBottom(),
    CustomConditionalFormat: custom,
    ConditionalFormatRule: custom.getRule(),
    ConditionalRangeFormat: ruleFormat,
    ConditionalRangeFill: ruleFormat.getFill(),
    ConditionalRangeFont: ruleFormat.getFont(),
    ConditionalRangeBorder: ruleFormat.getConditionalRangeBorderTop(),
  };
}

function main(workbook: ExcelScript.Workbook) {
  const classes: { name: string; members: string[] }[] = JSON.parse(CLASSES);
  let calls = 0;
  let round = 0;
  for (const cls of classes) {
    for (const member of cls.members) {
      const made = build(workbook, round++);
      const target = made[cls.name] as Anything;
      if (!target) {
        throw new Error(`no ${cls.name} to call`);
      }
      const sheet = made.Worksheet as ExcelScript.Worksheet;
      const odd: unknown[][] = [
        [], [null], [0], [-1], [1e99], [0.5], ["x"], [""], ["A1:B2"], ["ZZZ99999999"], [true], [{}], [[]], [[[1]]],
        [1, 1, 1, 1], [-1, -1, -1, -1], [1048575, 16383, 5, 5], ["A1", "B2", "C3"], [made.Range], [sheet], [made.Table],
        [made.Range, 5, { filterOn: "Values", values: [{}] }], [[{ key: 99 }]], [[{ key: -1, sortOn: "CellColor" }]], [["a"], "b"],
        [0, [1, 2]], [-1, [[1], [2, 3]]], ["Top", "Top"], ["=SUM(", "=1/0"], [{ address: 5, documentReference: {} }],
      ];
      for (const args of odd) {
        try {
          target[member](...args);
        } catch (error) {
          // Failing is fine.
        }
        calls++;
      }
      try {
        sheet.delete();
      } catch (error) {
        // It went with the call.
      }
    }
  }
  return calls;
}
