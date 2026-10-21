//! VBA projects: kept, not run.
//!
//! A macro-enabled workbook (`.xlsm`) carries a VBA project, a binary
//! part OpenSheet neither runs nor changes. It is kept as it was read,
//! byte for byte, with the parts that sign it, and goes back into the
//! file when the workbook is saved as a macro-enabled one.
//!
//! The project names the workbook and each sheet by a *code name*
//! (`ThisWorkbook`, `Sheet1`), which is not the name on the tab: it stays
//! when the sheet is renamed or moved, goes with a sheet that is deleted,
//! and a copy of a sheet has none (Excel gives it one of its own, with a
//! new module, when it next opens the file).
//!
//! Reading the modules of a project is the business of `opensheet_io`.

use crate::workbook::Workbook;
use std::sync::Arc;

/// A part that belongs to the project: one of its digital signatures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VbaPart {
    /// The name of the file, next to the project (`vbaProjectSignature.bin`).
    pub name: String,
    /// The kind of signature: the last word of the type of the
    /// relationship that leads to it (`vbaProjectSignature`,
    /// `vbaProjectSignatureAgile`, `vbaProjectSignatureV3`).
    pub kind: String,
    pub data: Arc<[u8]>,
}

/// The VBA project of a workbook, as it is in the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VbaProject {
    /// `xl/vbaProject.bin`.
    pub bin: Arc<[u8]>,
    /// Its signatures. They sign the project, not the file: they stay
    /// valid as long as the project is written back unchanged.
    pub signatures: Vec<VbaPart>,
}

/// What a workbook keeps of its macros.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Macros {
    pub project: Option<VbaProject>,
    /// The code name of the workbook (`ThisWorkbook`).
    pub workbook_code_name: Option<String>,
}

impl Workbook {
    /// True when the workbook carries a VBA project.
    pub fn has_vba_project(&self) -> bool {
        self.vba.project.is_some()
    }
}

/// See `features::on_sheet_duplicated`: two sheets cannot share a code
/// name, and the module of the original is not copied.
pub(crate) fn on_sheet_duplicated(wb: &mut Workbook, copy: usize, _original: &str) {
    if let Some(sheet) = wb.sheets.get_mut(copy) {
        sheet.code_name = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Locale;

    #[test]
    fn code_names_follow_the_sheets() {
        let mut wb = Workbook::new(Locale::en());
        wb.add_sheet(1);
        wb.add_sheet(2);
        for (index, name) in ["Sheet1", "Sheet2", "Sheet3"].into_iter().enumerate() {
            wb.sheets[index].code_name = Some(name.to_string());
        }
        wb.commit("setup");
        let names = |wb: &Workbook| -> Vec<(String, Option<String>)> {
            wb.sheets
                .iter()
                .map(|s| (s.name.clone(), s.code_name.clone()))
                .collect()
        };
        let code = |name: &str| Some(name.to_string());

        // Renaming and moving a sheet leave its code name alone.
        assert!(wb.rename_sheet(0, "Sales"));
        wb.move_sheet(0, 2);
        wb.commit("rename and move");
        assert_eq!(
            names(&wb),
            [
                ("Sheet2".to_string(), code("Sheet2")),
                ("Sheet3".to_string(), code("Sheet3")),
                ("Sales".to_string(), code("Sheet1"))
            ]
        );

        // A copy has no code name; the original keeps its own.
        let copy = wb.duplicate_sheet(2).unwrap();
        wb.commit("copy");
        assert_eq!(wb.sheets[copy].code_name, None);
        assert_eq!(wb.sheets[2].code_name, code("Sheet1"));

        // A deleted sheet takes its code name with it, and undo brings
        // both back.
        assert!(wb.delete_sheet(0));
        wb.commit("delete");
        assert!(wb.sheets.iter().all(|s| s.code_name != code("Sheet2")));
        assert!(wb.undo());
        assert_eq!(wb.sheets[0].code_name, code("Sheet2"));
    }

    #[test]
    fn a_workbook_knows_whether_it_has_a_project() {
        let mut wb = Workbook::new(Locale::en());
        assert!(!wb.has_vba_project());
        wb.vba.project = Some(VbaProject {
            bin: Arc::from(&b"project"[..]),
            signatures: Vec::new(),
        });
        assert!(wb.has_vba_project());
    }
}
