function main(workbook: ExcelScript.Workbook) {
  const sheet = workbook.getActiveWorksheet();
  eq(sheet.getCharts().length, 0);
  const chart = sheet.addChart(ExcelScript.ChartType.columnClustered, sheet.getRange("A1:C4"));
  eq([chart.getName(), sheet.getCharts().length, chart.getWorksheet().getName()], ["Chart 1", 1, "Sheet1"]);
  eq(sheet.getChart("chart 1")?.getId(), chart.getId());
  eq(sheet.getChart("Nope"), undefined);
  chart.setName("Stock");
  eq(sheet.getChart("Stock")?.getName(), "Stock");
  eq(chart.getChartType(), "ColumnClustered");
  chart.setChartType(ExcelScript.ChartType.barStacked);
  eq(chart.getChartType(), ExcelScript.ChartType.barStacked);
  throws(() => chart.setChartType(ExcelScript.ChartType.treemap), "ExcelScript.ChartType.treemap is not supported");

  const title = chart.getTitle();
  eq([title.getVisible(), title.getOverlay()], [true, false]);
  title.setText("Stock on hand");
  eq(title.getText(), "Stock on hand");
  title.setOverlay(true);
  title.setVisible(false);
  eq([title.getVisible(), title.getText(), title.getOverlay()], [false, "", true]);
  title.setVisible(true);
  title.setText("Stock");

  const legend = chart.getLegend();
  // Two series: the chart came with its legend, at the bottom.
  eq([legend.getVisible(), legend.getPosition(), legend.getOverlay()], [true, "Bottom", false]);
  legend.setVisible(false);
  eq([legend.getVisible(), legend.getPosition()], [false, "Invalid"]);
  legend.setVisible(true);
  eq(legend.getPosition(), "Right");
  legend.setPosition(ExcelScript.ChartLegendPosition.left);
  legend.setOverlay(true);
  eq([legend.getVisible(), legend.getPosition(), legend.getOverlay()], [true, "Left", true]);
  throws(() => legend.setPosition(ExcelScript.ChartLegendPosition.custom), "not supported");
  legend.setVisible(false);
  legend.setPosition(ExcelScript.ChartLegendPosition.corner);

  // Places and sizes are in points: a column is 48 wide, a row 15 high.
  chart.setPosition("E2", "H12");
  near(chart.getLeft(), 4 * 48);
  near(chart.getTop(), 15);
  near(chart.getWidth(), 4 * 48);
  near(chart.getHeight(), 11 * 15);
  chart.setPosition(sheet.getRange("B3"));
  near(chart.getLeft(), 48);
  near(chart.getWidth(), 4 * 48);
  chart.setLeft(100);
  chart.setTop(20);
  chart.setWidth(300);
  chart.setHeight(200);
  near(chart.getLeft(), 100);
  near(chart.getTop(), 20);
  near(chart.getWidth(), 300);
  near(chart.getHeight(), 200);
  throws(() => chart.setWidth(-1), "invalid");

  // One series: its name is the title Excel makes up.
  const pie = sheet.addChart(ExcelScript.ChartType.pie, sheet.getRange("A1:B4"), ExcelScript.ChartSeriesBy.columns);
  eq([pie.getName(), pie.getTitle().getText()], ["Chart 2", "Qty"]);
  throws(() => pie.setName("stock"), "already exists");
  throws(() => sheet.addChart(ExcelScript.ChartType.line, sheet.getRange("X1:Y9")), "nothing to chart");
  const elsewhere = workbook.addWorksheet("Charts").addChart(ExcelScript.ChartType.lineMarkers, sheet.getRange("A1:C4"), ExcelScript.ChartSeriesBy.rows);
  eq(elsewhere.getWorksheet().getName(), "Charts");
  pie.delete();
  eq(sheet.getCharts().map(c => c.getName()), ["Stock"]);
  throws(() => pie.getName(), "doesn't exist");
}
