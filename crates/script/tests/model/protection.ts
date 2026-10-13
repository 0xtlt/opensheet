function main(workbook: ExcelScript.Workbook) {
  const sheet = workbook.getActiveWorksheet();
  const protection = sheet.getProtection();
  eq([protection.getProtected(), protection.getIsPasswordProtected(), protection.checkPassword("anything")], [false, false, true]);
  eq([protection.getOptions().selectionMode, protection.getOptions().allowSort, protection.getOptions().allowFormatCells], ["Normal", false, false]);
  protection.unprotect();

  protection.protect({ allowSort: true, allowFormatCells: true, selectionMode: ExcelScript.ProtectionSelectionMode.unlocked }, "secret");
  eq([protection.getProtected(), protection.getIsPasswordProtected()], [true, true]);
  const options = protection.getOptions();
  eq([options.selectionMode, options.allowSort, options.allowFormatCells, options.allowInsertRows, options.allowEditObjects], ["Unlocked", true, true, false, false]);
  eq([protection.checkPassword("secret"), protection.checkPassword("Secret"), protection.checkPassword()], [true, false, false]);
  throws(() => protection.protect(), "already protected");
  throws(() => protection.unprotect("wrong"), "not correct");
  throws(() => protection.unprotect(), "not correct");
  throws(() => protection.updateOptions({ allowSort: false }), "without a password");
  throws(() => protection.setPassword("other"), "without one");
  // What the options allow goes through, the rest is refused as by hand.
  throws(() => sheet.getRange("A1").setValue("x"), "protected");
  throws(() => sheet.getRange("2:2").insert(ExcelScript.InsertShiftDirection.down), "protected");
  sheet.getRange("A1").getFormat().getFont().setItalic(true);

  // Unprotect, change, protect again: what scripts on protected sheets do.
  protection.unprotect("secret");
  eq(protection.getProtected(), false);
  sheet.getRange("A1").setValue("Item");
  protection.protect();
  eq([protection.getProtected(), protection.getIsPasswordProtected(), protection.getOptions().selectionMode], [true, false, "Normal"]);
  protection.updateOptions({ allowInsertRows: true, selectionMode: ExcelScript.ProtectionSelectionMode.none });
  eq([protection.getOptions().allowInsertRows, protection.getOptions().allowSort, protection.getOptions().selectionMode], [true, false, "None"]);
  throws(() => protection.updateOptions({ selectionMode: "Sideways" as ExcelScript.ProtectionSelectionMode }), "invalid");
  protection.setPassword("pw");
  eq([protection.getIsPasswordProtected(), protection.checkPassword("pw")], [true, true]);
}
