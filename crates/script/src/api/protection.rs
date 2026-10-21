//! `WorksheetProtection`: protecting a sheet, with what stays allowed
//! and a password, and removing the protection.
//!
//! A script protects and unprotects as a part of its run: when it fails
//! the sheet is protected as it was, with everything else. When it ends
//! well the undo history is forgotten, as it is when a sheet is
//! protected by hand (see `Workbook::set_sheet_protection`).

use serde_json::{Map, Value as J};

use opensheet_engine::protect::{PasswordHash, Permission, SheetProtection};

use super::{ApiClass, Call, Res, Ret, class};
use crate::host::Host;

/// The keys of `WorksheetProtectionOptions` and what each allows.
const OPTIONS: [(&str, Permission); 13] = [
    ("allowAutoFilter", Permission::AutoFilter),
    ("allowDeleteColumns", Permission::DeleteColumns),
    ("allowDeleteRows", Permission::DeleteRows),
    ("allowEditObjects", Permission::EditObjects),
    ("allowEditScenarios", Permission::EditScenarios),
    ("allowFormatCells", Permission::FormatCells),
    ("allowFormatColumns", Permission::FormatColumns),
    ("allowFormatRows", Permission::FormatRows),
    ("allowInsertColumns", Permission::InsertColumns),
    ("allowInsertHyperlinks", Permission::InsertHyperlinks),
    ("allowInsertRows", Permission::InsertRows),
    ("allowPivotTables", Permission::PivotTables),
    ("allowSort", Permission::Sort),
];

fn this_sheet(h: &Host, call: &Call) -> Res<usize> {
    h.sheet(call, call.this_int(0))
}

fn protection<'h>(h: &'h Host, call: &Call) -> Res<(usize, Option<&'h SheetProtection>)> {
    let sheet = this_sheet(h, call)?;
    Ok((sheet, h.wb.sheets[sheet].protection.as_ref()))
}

/// Lay the options a script gives over a protection: what it does not
/// name stays as it is.
fn apply(call: &Call, given: &J, onto: &mut SheetProtection) -> Res<()> {
    if given.is_null() {
        return Ok(());
    }
    let given = given.as_object().ok_or_else(|| call.invalid())?;
    for (key, permission) in OPTIONS {
        if let Some(allowed) = given.get(key).filter(|value| !value.is_null()) {
            onto.set(permission, allowed.as_bool().ok_or_else(|| call.invalid())?);
        }
    }
    if let Some(mode) = given.get("selectionMode").filter(|value| !value.is_null()) {
        let (locked, unlocked) = match mode.as_str() {
            Some("Normal") => (true, true),
            Some("Unlocked") => (false, true),
            Some("None") => (false, false),
            _ => return Err(call.invalid()),
        };
        onto.set(Permission::SelectLockedCells, locked);
        onto.set(Permission::SelectUnlockedCells, unlocked);
    }
    Ok(())
}

fn password_arg<'a>(call: &Call<'a>, index: usize) -> Res<&'a str> {
    Ok(call.opt_str(index)?.unwrap_or_default())
}

/// Whether a password opens a protection (any does when it has none).
fn opens(protection: &SheetProtection, password: &str) -> bool {
    protection
        .password
        .as_ref()
        .is_none_or(|hash| hash.verify(password))
}

fn hashed(password: &str) -> Option<PasswordHash> {
    (!password.is_empty()).then(|| PasswordHash::new(password))
}

pub(super) const WORKSHEET_PROTECTION: ApiClass = class!("WorksheetProtection", "The protection of a worksheet: locked cells cannot be changed, and options say what else stays allowed.", {
    getProtected "(): boolean" "Whether the worksheet is protected." => |h, c| Ok(protection(h, c)?.1.is_some().into());
    getIsPasswordProtected "(): boolean" "Whether removing the protection takes a password." => |h, c| {
        Ok(protection(h, c)?.1.is_some_and(|protection| protection.password.is_some()).into())
    };
    getOptions "(): ExcelScript.WorksheetProtectionOptions" "What the protection allows: allowFormatCells, allowSort, allowAutoFilter..., and selectionMode (Normal, Unlocked or None). For an unprotected worksheet, what Protect Sheet proposes." => |h, c| {
        let held = protection(h, c)?.1.cloned().unwrap_or_default();
        let mut out = Map::new();
        for (key, permission) in OPTIONS {
            out.insert(key.to_string(), held.allows(permission).into());
        }
        let mode = match (held.allows(Permission::SelectLockedCells), held.allows(Permission::SelectUnlockedCells)) {
            (true, _) => "Normal",
            (false, true) => "Unlocked",
            (false, false) => "None",
        };
        out.insert("selectionMode".to_string(), mode.into());
        Ok(Ret::Value(J::Object(out)))
    };
    protect "(options?: ExcelScript.WorksheetProtectionOptions, password?: string): void" "Protects the worksheet: without options, selecting cells is all that stays allowed. Fails when it is already protected." => |h, c| {
        let (sheet, held) = protection(h, c)?;
        if held.is_some() {
            return Err(c.fail("The worksheet is already protected."));
        }
        let mut new = SheetProtection::default();
        apply(c, c.arg(0), &mut new)?;
        new.password = hashed(password_arg(c, 1)?);
        h.wb.set_sheet_protection(sheet, Some(new));
        h.protection_changed = true;
        Ok(Ret::Nothing)
    };
    unprotect "(password?: string): void" "Removes the protection of the worksheet; with the password when it has one." => |h, c| {
        let (sheet, held) = protection(h, c)?;
        let password = password_arg(c, 0)?;
        match held {
            None => Ok(Ret::Nothing),
            Some(held) if !opens(held, password) => Err(c.fail("The password you supplied is not correct. Verify that the CAPS LOCK key is off and be sure to use the correct capitalization.")),
            Some(_) => {
                h.wb.set_sheet_protection(sheet, None);
                h.protection_changed = true;
                Ok(Ret::Nothing)
            }
        }
    };
    checkPassword "(password?: string): boolean" "Whether a password removes the protection (any does when it has none, or when the worksheet is not protected)." => |h, c| {
        let (_, held) = protection(h, c)?;
        let password = password_arg(c, 0)?;
        Ok(held.is_none_or(|held| opens(held, password)).into())
    };
    updateOptions "(options: ExcelScript.WorksheetProtectionOptions): void" "Changes what the protection allows; what the options do not name stays as it is. Fails for a worksheet that is not protected or has a password (unprotect it first)." => |h, c| {
        let (sheet, held) = protection(h, c)?;
        let Some(held) = held.filter(|held| held.password.is_none()) else {
            return Err(c.fail("The options can be changed on a worksheet protected without a password only."));
        };
        let mut new = held.clone();
        if !c.arg(0).is_object() {
            return Err(c.invalid());
        }
        apply(c, c.arg(0), &mut new)?;
        h.wb.set_sheet_protection(sheet, Some(new));
        h.protection_changed = true;
        Ok(Ret::Nothing)
    };
    setPassword "(password?: string): void" "Gives the protection a password, or takes it away with none. Fails for a worksheet that is not protected or already has a password (unprotect it first)." => |h, c| {
        let (sheet, held) = protection(h, c)?;
        let Some(held) = held.filter(|held| held.password.is_none()) else {
            return Err(c.fail("A password can be set on a worksheet protected without one only."));
        };
        let mut new = held.clone();
        new.password = hashed(password_arg(c, 0)?);
        h.wb.set_sheet_protection(sheet, Some(new));
        h.protection_changed = true;
        Ok(Ret::Nothing)
    };
});
