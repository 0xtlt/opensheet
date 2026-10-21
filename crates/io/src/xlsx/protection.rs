//! Protection: `<sheetProtection>` in a worksheet part and
//! `<workbookProtection>` in the workbook part. (The `locked` and `hidden`
//! flags of cells are part of their format and live in `styles.xml`.)
//!
//! Passwords are stored as hashes, in either of two forms: a 16-bit hash
//! in hexadecimal (`password="83AF"`), or an algorithm name, a salted and
//! iterated hash, the salt and the number of iterations. Both are kept
//! exactly as read, so that the password still opens the sheet wherever
//! the file goes next.

use super::xml::{Event, Reader, Tag, push_attr};
use opensheet_engine::Rect;
use opensheet_engine::protect::{
    PasswordHash, Permission, ProtectedRange, SheetProtection, WorkbookProtection,
};
use std::borrow::Cow;
use std::fmt::Write as _;

/// The attributes of `<sheetProtection>` that allow or forbid an action,
/// in the order of the schema, which is the order Excel writes them in.
const SHEET_ATTRIBUTES: [&str; 15] = [
    "objects",
    "scenarios",
    "formatCells",
    "formatColumns",
    "formatRows",
    "insertColumns",
    "insertRows",
    "insertHyperlinks",
    "deleteColumns",
    "deleteRows",
    "selectLockedCells",
    "sort",
    "autoFilter",
    "pivotTables",
    "selectUnlockedCells",
];

/// Most rounds of hashing the schema allows (`ST_SpinCount`).
const MAX_SPIN_COUNT: u32 = 10_000_000;

/// The name of a password attribute: in the workbook part they start with
/// `workbook` (`workbookPassword`, `workbookHashValue`).
fn password_attr(name: &str, workbook: bool) -> Cow<'_, str> {
    if !workbook {
        return Cow::Borrowed(name);
    }
    let mut prefixed = String::from("workbook");
    prefixed.extend(name.chars().next().map(|c| c.to_ascii_uppercase()));
    prefixed.push_str(&name[1..]);
    Cow::Owned(prefixed)
}

fn read_password(tag: &Tag, workbook: bool) -> Option<PasswordHash> {
    let attr = |name: &str| {
        tag.attr(&password_attr(name, workbook))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    // The salted hash is the stronger of the two; a file carrying both
    // (none should) is opened by the password it was made from.
    if let (Some(algorithm), Some(hash)) = (attr("algorithmName"), attr("hashValue")) {
        let spin_count = attr("spinCount").and_then(|v| v.parse().ok()).unwrap_or(0);
        // More rounds than the format allows is no hash anyone made:
        // checking a password against it would take hours.
        return (spin_count <= MAX_SPIN_COUNT).then(|| PasswordHash::Salted {
            algorithm,
            hash,
            salt: attr("saltValue").unwrap_or_default(),
            spin_count,
        });
    }
    u16::from_str_radix(&attr("password")?, 16)
        .ok()
        .map(PasswordHash::Legacy)
}

fn push_password(x: &mut String, password: &PasswordHash, workbook: bool) {
    let mut attr = |name: &str, value: &str| {
        w!(x, " {}=\"", password_attr(name, workbook));
        push_attr(x, value);
        x.push('"');
    };
    match password {
        PasswordHash::Legacy(hash) => attr("password", &format!("{hash:04X}")),
        PasswordHash::Salted {
            algorithm,
            hash,
            salt,
            spin_count,
        } => {
            attr("algorithmName", algorithm);
            attr("hashValue", hash);
            attr("saltValue", salt);
            attr("spinCount", &spin_count.to_string());
        }
    }
}

/// The protection a `<sheetProtection>` element stands for; `None` when
/// it does not protect the cells (`sheet` is off or missing: protecting
/// only the objects or the scenarios of a sheet is not modelled).
pub(crate) fn read_sheet(tag: &Tag) -> Option<SheetProtection> {
    if !tag.attr_bool("sheet").unwrap_or(false) {
        return None;
    }
    let mut protection = SheetProtection::from_attributes(|name| tag.attr_bool(name));
    protection.password = read_password(tag, false);
    Some(protection)
}

/// Append `<sheetProtection>`.
pub(crate) fn write_sheet(x: &mut String, protection: &SheetProtection) {
    x.push_str("<sheetProtection");
    if let Some(password) = &protection.password {
        push_password(x, password, false);
    }
    x.push_str(" sheet=\"1\"");
    // Like Excel, say only what differs from what a missing attribute
    // means: objects, scenarios and selecting cells are allowed then, the
    // rest is forbidden.
    for name in SHEET_ATTRIBUTES {
        let Some(permission) = Permission::ALL
            .into_iter()
            .find(|permission| permission.ooxml_name() == name)
        else {
            continue;
        };
        let forbidden = !protection.allows(permission);
        if forbidden == permission.allowed_when_absent() {
            w!(x, " {name}=\"{}\"", forbidden as u8);
        }
    }
    x.push_str("/>");
}

/// Read `<protectedRanges>`: the ranges of a protected sheet that may be
/// edited (Review > Allow Edit Ranges).
pub(crate) fn read_ranges<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Vec<ProtectedRange> {
    let mut ranges = Vec::new();
    if tag.empty {
        return ranges;
    }
    loop {
        match r.next() {
            Event::Open(t) => {
                if t.name == "protectedRange" {
                    let cells: Vec<Rect> = t
                        .attr("sqref")
                        .map(|sqref| {
                            sqref
                                .split_whitespace()
                                .filter_map(|part| Rect::parse(&part.replace('$', "")))
                                .collect()
                        })
                        .unwrap_or_default();
                    if let (Some(name), false) = (t.attr("name"), cells.is_empty()) {
                        let mut range = ProtectedRange::new(&name, cells, read_password(&t, false));
                        range.security_descriptor =
                            t.attr("securityDescriptor").map(|text| text.into_owned());
                        ranges.push(range);
                    }
                }
                r.skip(&t);
            }
            Event::Close(_) | Event::Eof => break,
            _ => {}
        }
    }
    ranges
}

/// Append `<protectedRanges>`, with the attributes in the order Excel
/// writes them: the password, the cells, the title.
pub(crate) fn write_ranges(x: &mut String, ranges: &[ProtectedRange]) {
    if ranges.is_empty() {
        return;
    }
    x.push_str("<protectedRanges>");
    for range in ranges {
        x.push_str("<protectedRange");
        if let Some(password) = &range.password {
            push_password(x, password, false);
        }
        x.push_str(" sqref=\"");
        for (n, rect) in range.ranges.iter().enumerate() {
            if n > 0 {
                x.push(' ');
            }
            super::write::push_range(x, *rect);
        }
        x.push_str("\" name=\"");
        push_attr(x, &range.name);
        x.push('"');
        if let Some(descriptor) = &range.security_descriptor {
            x.push_str(" securityDescriptor=\"");
            push_attr(x, descriptor);
            x.push('"');
        }
        x.push_str("/>");
    }
    x.push_str("</protectedRanges>");
}

/// The protection a `<workbookProtection>` element stands for. Tracking
/// of revisions (`lockRevision` and its password) is not modelled.
pub(crate) fn read_workbook(tag: &Tag) -> WorkbookProtection {
    WorkbookProtection {
        lock_structure: tag.attr_bool("lockStructure").unwrap_or(false),
        lock_windows: tag.attr_bool("lockWindows").unwrap_or(false),
        password: read_password(tag, true),
    }
}

/// Append `<workbookProtection>` when the workbook is protected.
pub(crate) fn write_workbook(x: &mut String, protection: &WorkbookProtection) {
    if !protection.is_protected() {
        return;
    }
    x.push_str("<workbookProtection");
    if let Some(password) = &protection.password {
        push_password(x, password, true);
    }
    if protection.lock_structure {
        x.push_str(" lockStructure=\"1\"");
    }
    if protection.lock_windows {
        x.push_str(" lockWindows=\"1\"");
    }
    x.push_str("/>");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::xlsx::xml::{Event, Reader};

    fn tag(xml: &str) -> Tag<'_> {
        match Reader::new(xml).next() {
            Event::Open(tag) => tag,
            _ => panic!("no element in {xml}"),
        }
    }

    fn sheet_xml(protection: &SheetProtection) -> String {
        let mut x = String::new();
        write_sheet(&mut x, protection);
        x
    }

    #[test]
    fn sheet_attributes_are_those_excel_writes() {
        // What the Protect Sheet dialog proposes.
        assert_eq!(
            sheet_xml(&SheetProtection::default()),
            r#"<sheetProtection sheet="1" objects="1" scenarios="1"/>"#
        );
        // Everything allowed.
        assert_eq!(
            sheet_xml(&SheetProtection::allowing(&Permission::ALL)),
            concat!(
                r#"<sheetProtection sheet="1" formatCells="0" formatColumns="0" formatRows="0" "#,
                r#"insertColumns="0" insertRows="0" insertHyperlinks="0" deleteColumns="0" "#,
                r#"deleteRows="0" sort="0" autoFilter="0" pivotTables="0"/>"#
            )
        );
        // Nothing allowed.
        assert_eq!(
            sheet_xml(&SheetProtection::allowing(&[])),
            r#"<sheetProtection sheet="1" objects="1" scenarios="1" selectLockedCells="1" selectUnlockedCells="1"/>"#
        );
        // Every single permission survives on its own.
        for permission in Permission::ALL {
            for others in [false, true] {
                let mut protection = if others {
                    SheetProtection::allowing(&Permission::ALL)
                } else {
                    SheetProtection::allowing(&[])
                };
                protection.set(permission, !others);
                let xml = sheet_xml(&protection);
                assert_eq!(read_sheet(&tag(&xml)), Some(protection), "{xml}");
            }
        }
    }

    #[test]
    fn passwords() {
        let mut protection = SheetProtection::default();
        protection.password = Some(PasswordHash::legacy("password"));
        let xml = sheet_xml(&protection);
        assert_eq!(
            xml,
            r#"<sheetProtection password="83AF" sheet="1" objects="1" scenarios="1"/>"#
        );
        assert_eq!(read_sheet(&tag(&xml)), Some(protection.clone()));

        protection.password = Some(PasswordHash::salted("secret", b"0123456789abcdef", 1000));
        let xml = sheet_xml(&protection);
        assert!(
            xml.starts_with(r#"<sheetProtection algorithmName="SHA-512" hashValue=""#),
            "{xml}"
        );
        assert!(
            xml.contains(r#" saltValue="MDEyMzQ1Njc4OWFiY2RlZg==" spinCount="1000" sheet="1""#)
        );
        let back = read_sheet(&tag(&xml)).unwrap();
        assert_eq!(back, protection);
        let hash = back.password.unwrap();
        assert!(hash.verify("secret") && !hash.verify("Secret"));

        // A hash with a leading zero keeps its four digits.
        protection.password = Some(PasswordHash::Legacy(0x00AB));
        assert!(sheet_xml(&protection).contains(r#"password="00AB""#));
    }

    #[test]
    fn what_files_say() {
        // Not protecting the cells is no protection.
        assert_eq!(
            read_sheet(&tag(r#"<sheetProtection objects="1" scenarios="1"/>"#)),
            None
        );
        assert_eq!(
            read_sheet(&tag(r#"<sheetProtection sheet="0" password="83AF"/>"#)),
            None
        );
        // A password that is not a hash is no password; both forms at
        // once give the salted one.
        let read = |xml: &str| read_sheet(&tag(xml)).unwrap().password;
        assert_eq!(read(r#"<sheetProtection sheet="1" password="xyz"/>"#), None);
        assert_eq!(read(r#"<sheetProtection sheet="true" password=""/>"#), None);
        assert_eq!(
            read(r#"<sheetProtection sheet="1" algorithmName="SHA-512" saltValue="AA=="/>"#),
            None
        );
        assert_eq!(
            read(r#"<sheetProtection sheet="1" password="cc1a"/>"#),
            Some(PasswordHash::Legacy(0xCC1A))
        );
        assert_eq!(
            read(
                r#"<sheetProtection sheet="1" algorithmName="SHA-512" hashValue="aA==" spinCount="10000001"/>"#
            ),
            None
        );
        assert_eq!(
            read(
                r#"<sheetProtection password="83AF" algorithmName="SHA-1" hashValue="aGFzaA==" saltValue="c2FsdA==" spinCount="x" sheet="1"/>"#
            ),
            Some(PasswordHash::Salted {
                algorithm: "SHA-1".into(),
                hash: "aGFzaA==".into(),
                salt: "c2FsdA==".into(),
                spin_count: 0,
            })
        );
    }

    #[test]
    fn workbook() {
        let mut x = String::new();
        write_workbook(&mut x, &WorkbookProtection::default());
        assert_eq!(x, "", "an unprotected workbook says nothing");

        let protection = WorkbookProtection {
            lock_structure: true,
            lock_windows: false,
            password: Some(PasswordHash::legacy("test")),
        };
        write_workbook(&mut x, &protection);
        assert_eq!(
            x,
            r#"<workbookProtection workbookPassword="CBEB" lockStructure="1"/>"#
        );
        assert_eq!(read_workbook(&tag(&x)), protection);

        let protection = WorkbookProtection {
            lock_structure: true,
            lock_windows: true,
            password: Some(PasswordHash::salted("test", b"salt", 10)),
        };
        let mut x = String::new();
        write_workbook(&mut x, &protection);
        assert!(
            x.starts_with(
                r#"<workbookProtection workbookAlgorithmName="SHA-512" workbookHashValue=""#
            ),
            "{x}"
        );
        assert!(x.ends_with(
            r#" workbookSaltValue="c2FsdA==" workbookSpinCount="10" lockStructure="1" lockWindows="1"/>"#
        ));
        assert_eq!(read_workbook(&tag(&x)), protection);
        // What Excel 2010 and later write.
        let read = read_workbook(&tag(
            r#"<workbookProtection workbookAlgorithmName="SHA-512" workbookHashValue="aA==" workbookSaltValue="cw==" workbookSpinCount="100000" lockStructure="1" lockRevision="1" revisionsPassword="83AF"/>"#,
        ));
        assert!(read.lock_structure && !read.lock_windows);
        assert!(matches!(
            read.password,
            Some(PasswordHash::Salted {
                spin_count: 100_000,
                ..
            })
        ));
    }
}
