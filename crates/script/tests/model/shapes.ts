function main(workbook: ExcelScript.Workbook) {
  const sheet = workbook.getActiveWorksheet();
  eq([sheet.getShapes().length, sheet.getShape("Nope")], [0, undefined]);

  // A geometric shape: place, size, name, look.
  const box = sheet.addGeometricShape(ExcelScript.GeometricShapeType.rectangle);
  eq([box.getType(), box.getGeometricShapeType(), box.getName(), box.getVisible()], ["GeometricShape", "Rectangle", "Rectangle 1", true]);
  eq([box.getLeft(), box.getTop(), box.getWidth(), box.getHeight()], [0, 0, 72, 72]);
  box.setLeft(120);
  box.setTop(30);
  box.setWidth(150);
  box.setHeight(60);
  box.incrementLeft(-20);
  box.incrementTop(15);
  eq([box.getLeft(), box.getTop(), box.getWidth(), box.getHeight()].map(n => Math.round(n * 100) / 100), [100, 45, 150, 60]);
  throws(() => box.setWidth(-5), "invalid");
  box.setName("Banner");
  eq([box.getName(), sheet.getShape("banner")?.getId(), sheet.getShape(box.getId())?.getName()], ["Banner", box.getId(), "Banner"]);
  throws(() => box.setName(" "), "invalid");
  box.setGeometricShapeType(ExcelScript.GeometricShapeType.roundRectangle);
  eq(box.getGeometricShapeType(), "RoundRectangle");
  eq([box.getRotation(), box.getLockAspectRatio(), box.getPlacement(), box.getAltTextDescription()], [0, false, "TwoCell", ""]);
  box.setRotation(350);
  box.incrementRotation(25);
  box.setLockAspectRatio(true);
  box.setPlacement(ExcelScript.Placement.absolute);
  box.setAltTextDescription("The title of the report");
  eq([box.getRotation(), box.getLockAspectRatio(), box.getPlacement(), box.getAltTextDescription()], [15, true, "Absolute", "The title of the report"]);
  box.setPlacement(ExcelScript.Placement.oneCell);
  eq(box.getPlacement(), "OneCell");
  box.setRotation(0);

  const fill = box.getFill();
  eq([fill.getType(), fill.getForegroundColor().length], ["Solid", 7]);
  fill.setSolidColor("navy");
  eq(fill.getForegroundColor(), "#000080");
  fill.setForegroundColor("#336699");
  eq(fill.getForegroundColor(), "#336699");
  fill.clear();
  eq([fill.getType(), fill.getForegroundColor()], ["NoFill", ""]);
  fill.setSolidColor("#336699");

  const line = box.getLineFormat();
  eq([line.getVisible(), line.getWeight(), line.getDashStyle()], [true, 1, "Solid"]);
  line.setColor("red");
  line.setWeight(2.5);
  line.setDashStyle(ExcelScript.ShapeLineDashStyle.longDashDot);
  eq([line.getColor(), line.getWeight(), line.getDashStyle()], ["#FF0000", 2.5, "LongDashDot"]);
  line.setDashStyle(ExcelScript.ShapeLineDashStyle.systemDash);
  eq(line.getDashStyle(), "SquareDot");
  line.setVisible(false);
  eq([line.getVisible(), line.getColor(), line.getWeight()], [false, "", 0]);
  line.setVisible(true);
  eq([line.getColor(), line.getWeight()], ["#000000", 1]);
  throws(() => line.setWeight(0), "invalid");

  // Text.
  const frame = box.getTextFrame();
  eq([frame.getHasText(), frame.getHorizontalAlignment(), frame.getVerticalAlignment()], [false, "Center", "Middle"]);
  const text = frame.getTextRange();
  text.setText("Quarterly report");
  eq([frame.getHasText(), text.getText()], [true, "Quarterly report"]);
  frame.setHorizontalAlignment(ExcelScript.ShapeTextHorizontalAlignment.right);
  frame.setVerticalAlignment(ExcelScript.ShapeTextVerticalAlignment.bottom);
  eq([frame.getHorizontalAlignment(), frame.getVerticalAlignment()], ["Right", "Bottom"]);
  throws(() => frame.setHorizontalAlignment(ExcelScript.ShapeTextHorizontalAlignment.distributed), "not supported");
  throws(() => frame.setVerticalAlignment(ExcelScript.ShapeTextVerticalAlignment.justified), "not supported");
  const font = text.getFont();
  eq([font.getBold(), font.getItalic(), font.getUnderline(), font.getSize(), font.getColor()], [false, false, "None", 11, "#FFFFFF"]);
  font.setName("Georgia");
  font.setSize(20);
  font.setBold(true);
  font.setItalic(true);
  font.setUnderline(ExcelScript.ShapeFontUnderlineStyle.double);
  font.setColor("yellow");
  eq([font.getName(), font.getSize(), font.getBold(), font.getItalic(), font.getUnderline(), font.getColor()], ["Georgia", 20, true, true, "Double", "#FFFF00"]);
  throws(() => font.setUnderline(ExcelScript.ShapeFontUnderlineStyle.wavy), "not supported");
  font.setColor("");
  eq(font.getColor(), "");
  frame.deleteText();
  eq([frame.getHasText(), text.getText()], [false, ""]);
  text.setText("Q3");

  // A text box and lines.
  const note = sheet.addTextBox("Draft");
  eq([note.getType(), note.getTextFrame().getTextRange().getText(), note.getTextFrame().getHorizontalAlignment(), note.getWidth(), note.getHeight()], ["GeometricShape", "Draft", "Left", 72, 36]);
  eq(sheet.addTextBox().getTextFrame().getHasText(), false);
  const rule = sheet.addLine(200, 100, 50, 40);
  eq([rule.getType(), rule.getLeft(), rule.getTop(), rule.getWidth(), rule.getHeight()].map(v => (typeof v === "number" ? Math.round(v) : v)), ["Line", 50, 40, 150, 60]);
  eq([rule.getFill().getType(), rule.getLineFormat().getVisible()], ["NoFill", true]);
  throws(() => rule.getGeometricShapeType(), "not a geometric shape");
  throws(() => rule.setGeometricShapeType(ExcelScript.GeometricShapeType.ellipse), "not a geometric shape");
  throws(() => sheet.addLine(0, 0, 10, 10, ExcelScript.ConnectorType.elbow), "not supported");
  eq(sheet.addLine(0, 0, 10, 10, ExcelScript.ConnectorType.straight).getType(), "Line");

  // A picture.
  const png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
  const picture = sheet.addImage(png);
  eq([picture.getType(), picture.getFill().getType(), picture.getTextFrame().getHasText(), picture.getLineFormat().getVisible()], ["Image", "NoFill", false, false]);
  const image = picture.getImage();
  eq([image.getFormat(), image.getId(), image.getShape().getId()], ["PNG", picture.getId(), picture.getId()]);
  eq(picture.getImageAsBase64(ExcelScript.PictureFormat.png), png);
  eq(sheet.addImage("data:image/png;base64," + png).getImage().getFormat(), "PNG");
  throws(() => picture.getImageAsBase64(ExcelScript.PictureFormat.jpeg), "not supported");
  throws(() => box.getImageAsBase64(ExcelScript.PictureFormat.png), "not supported");
  throws(() => box.getImage(), "not a picture");
  throws(() => sheet.addImage("aGVsbG8="), "not a PNG");
  throws(() => sheet.addImage("***"), "base64");
  throws(() => picture.getTextFrame().getTextRange(), "not a geometric shape");
  throws(() => picture.getFill().setSolidColor("red"), "not a geometric shape");

  // Stacking order, visibility, deletion.
  eq(sheet.getShapes().map(s => s.getType()), ["GeometricShape", "GeometricShape", "GeometricShape", "Line", "Line", "Image", "Image"]);
  eq([box.getZOrderPosition(), picture.getZOrderPosition()], [0, 5]);
  box.setZOrder(ExcelScript.ShapeZOrder.bringToFront);
  eq([box.getZOrderPosition(), sheet.getShapes()[6].getName()], [6, "Banner"]);
  box.setZOrder(ExcelScript.ShapeZOrder.sendBackward);
  eq(box.getZOrderPosition(), 5);
  box.setZOrder(ExcelScript.ShapeZOrder.bringForward);
  box.setZOrder(ExcelScript.ShapeZOrder.sendToBack);
  eq(box.getZOrderPosition(), 0);
  note.setVisible(false);
  eq(note.getVisible(), false);
  rule.delete();
  throws(() => rule.getName(), "doesn't exist");
  eq(sheet.getShapes().length, 6);
  throws(() => sheet.addGeometricShape(ExcelScript.GeometricShapeType.heart), "not supported");
}
