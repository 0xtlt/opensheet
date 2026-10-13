function main(workbook: ExcelScript.Workbook) {
  const sheet = workbook.getActiveWorksheet();
  const qty = sheet.getRange("B2:B4");
  const price = sheet.getRange("C2:C4");
  eq(qty.getConditionalFormats().length, 0);

  // Cell value: quantities of 5 or more in bold red on yellow.
  const cf = qty.addConditionalFormat(ExcelScript.ConditionalFormatType.cellValue);
  eq(cf.getType(), "CellValue");
  eq([cf.getPriority(), cf.getStopIfTrue(), cf.getRange().getAddress()], [0, false, "Sheet1!B2:B4"]);
  const cell = cf.getCellValue()!;
  eq([cf.getColorScale(), cf.getDataBar(), cf.getIconSet(), cf.getPreset(), cf.getTextComparison(), cf.getTopBottom(), cf.getCustom()].filter(x => x !== undefined).length, 0);
  cell.setRule({ formula1: "=5", operator: ExcelScript.ConditionalCellValueOperator.greaterThanOrEqual });
  eq(cell.getRule(), { formula1: "5", operator: "GreaterThanOrEqual" });
  cell.setRule({ formula1: "2", formula2: "=$C$4*2", operator: ExcelScript.ConditionalCellValueOperator.between });
  eq(cell.getRule(), { formula1: "2", formula2: "=$C$4*2", operator: "Between" });
  throws(() => cell.setRule({ formula1: "2", operator: ExcelScript.ConditionalCellValueOperator.between }), "invalid");
  const format = cell.getFormat();
  const fill = format.getFill();
  eq(fill.getColor(), "");
  fill.setColor("yellow");
  eq(fill.getColor(), "#FFFF00");
  const font = format.getFont();
  eq([font.getBold(), font.getItalic(), font.getStrikethrough(), font.getUnderline(), font.getColor()], [false, false, false, "None", ""]);
  font.setBold(true);
  font.setItalic(true);
  font.setStrikethrough(true);
  font.setUnderline(ExcelScript.ConditionalRangeFontUnderlineStyle.double);
  font.setColor("#FF0000");
  eq([font.getBold(), font.getItalic(), font.getStrikethrough(), font.getUnderline(), font.getColor()], [true, true, true, "Double", "#FF0000"]);
  eq(format.getNumberFormat(), "");
  format.setNumberFormat("0.00");
  eq(format.getNumberFormat(), "0.00");
  eq(format.getBorders().map(b => b.getSideIndex()), ["EdgeTop", "EdgeBottom", "EdgeLeft", "EdgeRight"]);
  const top = format.getConditionalRangeBorderTop();
  eq([top.getStyle(), top.getColor()], ["None", ""]);
  top.setStyle(ExcelScript.ConditionalRangeBorderLineStyle.dash);
  top.setColor("blue");
  eq([top.getStyle(), top.getColor()], ["Dash", "#0000FF"]);
  format.getConditionalRangeBorderBottom().setColor("green");
  eq(format.getConditionalRangeBorder(ExcelScript.ConditionalRangeBorderIndex.edgeBottom).getStyle(), "Continuous");
  format.getConditionalRangeBorderLeft().setStyle(ExcelScript.ConditionalRangeBorderLineStyle.dot);
  format.getConditionalRangeBorderRight().setStyle(ExcelScript.ConditionalRangeBorderLineStyle.continuous);
  throws(() => top.setStyle(ExcelScript.ConditionalRangeBorderLineStyle.dashDot), "not supported");
  font.clear();
  fill.clear();
  eq([font.getBold(), font.getColor(), fill.getColor()], [false, "", ""]);
  fill.setColor("#FFFF00");
  cf.setStopIfTrue(true);
  eq(cf.getStopIfTrue(), true);

  // Colour scale.
  const scale = price.addConditionalFormat(ExcelScript.ConditionalFormatType.colorScale);
  eq([scale.getPriority(), cf.getPriority()], [0, 1]);
  const colors = scale.getColorScale()!;
  eq(colors.getThreeColorScale(), true);
  colors.setCriteria({
    minimum: { type: ExcelScript.ConditionalFormatColorCriterionType.lowestValue, color: "white" },
    maximum: { type: ExcelScript.ConditionalFormatColorCriterionType.number, formula: "12", color: "#00FF00" },
  });
  eq(colors.getThreeColorScale(), false);
  eq(colors.getCriteria(), { maximum: { color: "#00FF00", formula: "12", type: "Number" }, minimum: { color: "#FFFFFF", type: "LowestValue" } });
  colors.setCriteria({
    minimum: { type: ExcelScript.ConditionalFormatColorCriterionType.percent, formula: "10", color: "red" },
    midpoint: { type: ExcelScript.ConditionalFormatColorCriterionType.percentile, formula: "=50", color: "yellow" },
    maximum: { type: ExcelScript.ConditionalFormatColorCriterionType.highestValue, color: "green" },
  });
  eq(colors.getCriteria().midpoint, { color: "#FFFF00", formula: "50", type: "Percentile" });

  // Data bars.
  const bars = qty.addConditionalFormat(ExcelScript.ConditionalFormatType.dataBar).getDataBar()!;
  eq([bars.getAxisColor(), bars.getAxisFormat(), bars.getBarDirection(), bars.getShowDataBarOnly()], ["#000000", "Automatic", "Context", false]);
  bars.setAxisColor("gray");
  bars.setAxisFormat(ExcelScript.ConditionalDataBarAxisFormat.cellMidPoint);
  bars.setBarDirection(ExcelScript.ConditionalDataBarDirection.rightToLeft);
  bars.setShowDataBarOnly(true);
  eq([bars.getAxisColor(), bars.getAxisFormat(), bars.getBarDirection(), bars.getShowDataBarOnly()], ["#808080", "CellMidPoint", "RightToLeft", true]);
  eq([bars.getLowerBoundRule(), bars.getUpperBoundRule()], [{ type: "Automatic" }, { type: "Automatic" }]);
  bars.setLowerBoundRule({ type: ExcelScript.ConditionalFormatRuleType.number, formula: "0" });
  bars.setUpperBoundRule({ type: ExcelScript.ConditionalFormatRuleType.formula, formula: "=MAX($B$2:$B$4)*2" });
  eq([bars.getLowerBoundRule(), bars.getUpperBoundRule()], [{ formula: "0", type: "Number" }, { formula: "=MAX($B$2:$B$4)*2", type: "Formula" }]);
  const positive = bars.getPositiveFormat();
  eq([positive.getFillColor(), positive.getGradientFill(), positive.getBorderColor()], ["#638EC6", true, "#638EC6"]);
  positive.setFillColor("#112233");
  positive.setGradientFill(false);
  positive.setBorderColor("");
  eq([positive.getFillColor(), positive.getGradientFill(), positive.getBorderColor()], ["#112233", false, ""]);
  const negative = bars.getNegativeFormat();
  eq([negative.getFillColor(), negative.getMatchPositiveFillColor(), negative.getBorderColor(), negative.getMatchPositiveBorderColor()], ["#FF0000", false, "#FF0000", false]);
  negative.setMatchPositiveFillColor(true);
  negative.setMatchPositiveBorderColor(true);
  eq([negative.getFillColor(), negative.getMatchPositiveFillColor(), negative.getBorderColor(), negative.getMatchPositiveBorderColor()], ["#112233", true, "", true]);
  negative.setFillColor("purple");
  negative.setBorderColor("black");
  eq([negative.getFillColor(), negative.getMatchPositiveFillColor(), negative.getBorderColor()], ["#800080", false, "#000000"]);

  // Icon sets.
  const icons = price.addConditionalFormat(ExcelScript.ConditionalFormatType.iconSet).getIconSet()!;
  eq([icons.getStyle(), icons.getReverseIconOrder(), icons.getShowIconOnly()], ["ThreeTrafficLights1", false, false]);
  eq(icons.getCriteria().map(c => c.formula), ["", "33", "67"]);
  icons.setStyle(ExcelScript.IconSet.threeArrows);
  icons.setReverseIconOrder(true);
  icons.setShowIconOnly(true);
  icons.setCriteria([
    { type: ExcelScript.ConditionalFormatIconRuleType.invalid, operator: ExcelScript.ConditionalIconCriterionOperator.invalid, formula: "" },
    { type: ExcelScript.ConditionalFormatIconRuleType.number, operator: ExcelScript.ConditionalIconCriterionOperator.greaterThan, formula: "=2" },
    { type: ExcelScript.ConditionalFormatIconRuleType.percentile, operator: ExcelScript.ConditionalIconCriterionOperator.greaterThanOrEqual, formula: "80", customIcon: { set: ExcelScript.IconSet.threeFlags, index: 2 } },
  ]);
  eq([icons.getStyle(), icons.getReverseIconOrder(), icons.getShowIconOnly()], ["ThreeArrows", true, true]);
  eq(icons.getCriteria(), [
    { formula: "", operator: "Invalid", type: "Invalid" },
    { formula: "2", operator: "GreaterThan", type: "Number" },
    { customIcon: { index: 2, set: "ThreeFlags" }, formula: "80", operator: "GreaterThanOrEqual", type: "Percentile" },
  ]);
  throws(() => icons.setCriteria([]), "3 criteria");
  icons.setStyle(ExcelScript.IconSet.fiveRating);
  eq(icons.getCriteria().length, 5);

  // Top and bottom, presets, text, formula.
  const best = price.addConditionalFormat(ExcelScript.ConditionalFormatType.topBottom).getTopBottom()!;
  eq(best.getRule(), { rank: 10, type: "TopItems" });
  best.setRule({ rank: 1, type: ExcelScript.ConditionalTopBottomCriterionType.bottomPercent });
  eq(best.getRule(), { rank: 1, type: "BottomPercent" });
  throws(() => best.setRule({ rank: 0, type: ExcelScript.ConditionalTopBottomCriterionType.topItems }), "invalid");
  best.getFormat().getFont().setBold(true);

  const preset = price.addConditionalFormat(ExcelScript.ConditionalFormatType.presetCriteria).getPreset()!;
  eq(preset.getRule(), { criterion: "DuplicateValues" });
  for (const criterion of ["Blanks", "NonErrors", "LastSevenDays", "EqualOrBelowAverage", "TwoStdDevAboveAverage", "UniqueValues"]) {
    preset.setRule({ criterion: criterion as ExcelScript.ConditionalFormatPresetCriterion });
    eq(preset.getRule().criterion, criterion);
  }
  preset.setRule({ criterion: ExcelScript.ConditionalFormatPresetCriterion.aboveAverage });
  preset.getFormat().getFill().setColor("#C6EFCE");

  const names = sheet.getRange("A2:A4");
  const text = names.addConditionalFormat(ExcelScript.ConditionalFormatType.containsText).getTextComparison()!;
  text.setRule({ operator: ExcelScript.ConditionalTextOperator.beginsWith, text: "P" });
  eq(text.getRule(), { operator: "BeginsWith", text: "P" });
  text.getFormat().getFont().setColor("#9C0006");

  const custom = names.addConditionalFormat(ExcelScript.ConditionalFormatType.custom);
  const formula = custom.getCustom()!.getRule();
  formula.setFormula("=$B2>C2");
  eq([formula.getFormula(), formula.getFormulaLocal()], ["=$B2>C2", "=$B2>C2"]);
  formula.setFormulaLocal("=AND($B2>1,LEN($A2)=3)");
  eq(formula.getFormula(), "=AND($B2>1,LEN($A2)=3)");
  throws(() => formula.setFormula("B2>1"), "invalid");
  custom.getCustom()!.getFormat().getFill().setColor("orange");

  // Priorities, ranges, identifiers, deletion.
  eq(sheet.getRange("A1:C4").getConditionalFormats().map(f => f.getType()), ["Custom", "ContainsText", "PresetCriteria", "TopBottom", "IconSet", "DataBar", "ColorScale", "CellValue"]);
  eq(cf.getPriority(), 7);
  cf.setPriority(0);
  eq([cf.getPriority(), custom.getPriority(), scale.getPriority()], [0, 1, 7]);
  custom.setPriority(5);
  eq([cf.getPriority(), custom.getPriority(), scale.getPriority()], [0, 5, 7]);
  eq(qty.getConditionalFormats().map(f => f.getType()), ["CellValue", "DataBar"]);
  eq(qty.getConditionalFormat(cf.getId())?.getType(), "CellValue");
  eq(qty.getConditionalFormat("999"), undefined);
  custom.setRanges("A2:A3, C2");
  eq([custom.getRanges().getAddress(), custom.getRanges().getAreaCount()], ["Sheet1!A2:A3,Sheet1!C2", 2]);
  throws(() => custom.getRange(), "several ranges");
  custom.setRanges(sheet.getRange("A2:A4"));
  eq(custom.getRange().getAddress(), "Sheet1!A2:A4");
  eq(formula.getFormula(), "=AND($B2>1,LEN($A2)=3)");
  scale.delete();
  throws(() => scale.getType(), "doesn't exist");
  eq([cf.getPriority(), custom.getPriority()], [0, 5]);
  // Clearing a range takes its cells out of the rules; rules left without cells go.
  sheet.getRange("C2:C4").clearAllConditionalFormats();
  eq(sheet.getRange("A1:C4").getConditionalFormats().map(f => f.getType()), ["CellValue", "ContainsText", "Custom", "DataBar"]);
  eq([cf.getPriority(), custom.getPriority(), bars.getAxisFormat()], [0, 2, "CellMidPoint"]);
  throws(() => best.getRule(), "doesn't exist");
  sheet.getRange("B2").clearAllConditionalFormats();
  eq(cf.getRange().getAddress(), "Sheet1!B3:B4");
  const areas = custom.getRanges();
  eq([areas.getAddressLocal(), areas.getCellCount(), areas.getAreas().length, areas.getWorksheet().getName()], ["Sheet1!A2:A4", 3, 1, "Sheet1"]);
}
