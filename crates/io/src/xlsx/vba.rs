//! The VBA project of a macro-enabled workbook, kept as it is.
//!
//! In the package the project is `xl/vbaProject.bin`, a part the workbook
//! part is related to; its digital signatures are parts the project is
//! related to in turn (`xl/_rels/vbaProject.bin.rels`). The workbook part
//! itself is declared with a macro-enabled content type. The workbook and
//! every sheet say under which *code name* the project knows them
//! (`<workbookPr codeName="ThisWorkbook"/>`, `<sheetPr codeName="Sheet1"/>`).
//!
//! Nothing of the project is interpreted here: its bytes go back into the
//! file exactly as they came out of it.

use super::parts::{Extras, push_relationship};
use super::read::{Package, Rel, relationships};
use super::write::NS_PKG_REL;
use super::xml::{Event, Reader, XML_DECL};
use opensheet_engine::Workbook;
use opensheet_engine::vba::{VbaPart, VbaProject};
use std::fmt::Write as _;
use std::io::{Read, Seek};
use std::sync::Arc;

/// The family of the relationship types of VBA projects.
const NS_REL_2006: &str = "http://schemas.microsoft.com/office/2006/relationships";
const CT_PROJECT: &str = "application/vnd.ms-office.vbaProject";

/// The signatures a project can carry: the last word of the type of
/// their relationship, and their content type.
const SIGNATURES: [(&str, &str); 3] = [
    ("vbaProjectSignature", "application/vnd.ms-office.vbaProjectSignature"),
    ("vbaProjectSignatureAgile", "application/vnd.ms-office.vbaProjectSignatureAgile"),
    ("vbaProjectSignatureV3", "application/vnd.ms-office.vbaProjectSignatureV3"),
];

/// What kind of file a package is: it decides the content type of the
/// workbook part and whether a VBA project may be in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PackageKind {
    /// `.xlsx`
    Workbook,
    /// `.xlsm`
    MacroEnabled,
    /// `.xltx`
    Template,
    /// `.xltm`
    MacroEnabledTemplate,
}

impl PackageKind {
    /// The kind a file extension (in lower case) asks for.
    pub fn of_extension(extension: &str) -> PackageKind {
        match extension {
            "xlsm" => PackageKind::MacroEnabled,
            "xltx" => PackageKind::Template,
            "xltm" => PackageKind::MacroEnabledTemplate,
            _ => PackageKind::Workbook,
        }
    }

    pub fn keeps_macros(self) -> bool {
        matches!(self, PackageKind::MacroEnabled | PackageKind::MacroEnabledTemplate)
    }

    /// The content type of the workbook part.
    pub fn content_type(self) -> &'static str {
        match self {
            PackageKind::Workbook => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml",
            PackageKind::MacroEnabled => "application/vnd.ms-excel.sheet.macroEnabled.main+xml",
            PackageKind::Template => "application/vnd.openxmlformats-officedocument.spreadsheetml.template.main+xml",
            PackageKind::MacroEnabledTemplate => "application/vnd.ms-excel.template.macroEnabled.main+xml",
        }
    }
}

/// The code name of a workbook part: the attribute of its `<workbookPr>`.
fn workbook_code_name(workbook_xml: &str) -> Option<String> {
    let start = workbook_xml.find("<workbookPr")?;
    let mut r = Reader::new(&workbook_xml[start..]);
    match r.next() {
        Event::Open(t) if t.name == "workbookPr" => t.attr("codeName").map(|name| name.into_owned()).filter(|name| !name.is_empty()),
        _ => None,
    }
}

/// Keep the VBA project of a package and the code name of the workbook.
/// `rels` are the relationships of the workbook part.
pub(super) fn read<R: Read + Seek>(pkg: &mut Package<R>, rels: &[Rel], workbook_xml: &str, wb: &mut Workbook) {
    wb.vba.workbook_code_name = workbook_code_name(workbook_xml);
    let Some(part) = rels.iter().find(|rel| rel.kind == "vbaProject").map(|rel| rel.target.clone()) else {
        return;
    };
    let Ok(Some(bin)) = pkg.read_bytes(&part) else {
        return;
    };
    let mut signatures = Vec::new();
    for rel in relationships(pkg, &part) {
        if !SIGNATURES.iter().any(|(kind, _)| *kind == rel.kind) {
            continue;
        }
        if let Ok(Some(data)) = pkg.read_bytes(&rel.target) {
            let name = rel.target.rsplit('/').next().unwrap_or(&rel.target).to_string();
            signatures.push(VbaPart {
                name,
                kind: rel.kind,
                data: Arc::from(data),
            });
        }
    }
    wb.vba.project = Some(VbaProject {
        bin: Arc::from(bin),
        signatures,
    });
}

/// The attribute that gives the workbook part or a worksheet its code
/// name; nothing when it has none.
pub(crate) fn push_code_name(x: &mut String, code_name: Option<&str>) {
    if let Some(name) = code_name.filter(|name| !name.is_empty()) {
        x.push_str(" codeName=\"");
        super::xml::push_attr(x, name);
        x.push('"');
    }
}

/// The project of the workbook and its signatures, as parts of a
/// macro-enabled package.
pub(crate) fn add_parts(wb: &Workbook, extras: &mut Extras) {
    let Some(project) = &wb.vba.project else {
        return;
    };
    extras.add_part("xl/vbaProject.bin".to_string(), Some(CT_PROJECT), project.bin.to_vec());
    extras.relate_workbook(&format!("{NS_REL_2006}/vbaProject"), "vbaProject.bin");
    let mut rels = String::new();
    for signature in &project.signatures {
        let Some((kind, content_type)) = SIGNATURES.iter().find(|(kind, _)| *kind == signature.kind) else {
            continue;
        };
        // A name that would leave `xl/` is not one a signature has.
        if signature.name.is_empty() || signature.name.contains(['/', '\\']) {
            continue;
        }
        extras.add_part(format!("xl/{}", signature.name), Some(content_type), signature.data.to_vec());
        let id = rels.matches("<Relationship ").count() + 1;
        push_relationship(&mut rels, id, &format!("{NS_REL_2006}/{kind}"), &signature.name);
    }
    if !rels.is_empty() {
        let mut x = String::from(XML_DECL);
        w!(x, "<Relationships xmlns=\"{NS_PKG_REL}\">{rels}</Relationships>");
        extras.add_part("xl/_rels/vbaProject.bin.rels".to_string(), None, x);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kind_of_a_package_follows_its_extension() {
        assert_eq!(PackageKind::of_extension("xlsx"), PackageKind::Workbook);
        assert_eq!(PackageKind::of_extension("xlsm"), PackageKind::MacroEnabled);
        assert_eq!(PackageKind::of_extension("xltx"), PackageKind::Template);
        assert_eq!(PackageKind::of_extension("xltm"), PackageKind::MacroEnabledTemplate);
        assert!(PackageKind::MacroEnabled.keeps_macros() && PackageKind::MacroEnabledTemplate.keeps_macros());
        assert!(!PackageKind::Workbook.keeps_macros() && !PackageKind::Template.keeps_macros());
    }

    #[test]
    fn code_names() {
        let xml = "<workbook><fileVersion codeName=\"{X}\"/><workbookPr date1904=\"1\" codeName=\"Ce&amp;Classeur\"/></workbook>";
        assert_eq!(workbook_code_name(xml).as_deref(), Some("Ce&Classeur"));
        assert_eq!(workbook_code_name("<workbook><workbookPr/></workbook>"), None);
        assert_eq!(workbook_code_name("<workbook/>"), None);
        let mut x = String::new();
        push_code_name(&mut x, Some("A&B"));
        push_code_name(&mut x, None);
        push_code_name(&mut x, Some(""));
        assert_eq!(x, " codeName=\"A&amp;B\"");
    }
}
