//! The background picture of a sheet: `<picture r:id="rId1"/>` in the
//! worksheet part, a relationship of the worksheet to an image file in
//! `xl/media/`, as Excel stores it.

use super::parts::{Extras, SheetLinks};
use super::read::{Package, relationships};
use super::write::NS_DOC_REL;
use super::xml::{Event, Reader};
use opensheet_engine::Workbook;
use opensheet_engine::drawing::Picture;
use opensheet_engine::workbook::Sheet;
use std::io::{Read, Seek};

/// The name in `xl/media/` of the file of a picture: the file that is
/// there already when a picture or another background shows the same
/// image, else a new one, numbered after those the package has.
fn media_name(picture: &Picture, extras: &mut Extras) -> String {
    let known = extras
        .parts
        .iter()
        .filter_map(|part| Some((part.name.strip_prefix("xl/media/")?, &part.data)))
        .find(|(_, data)| data.as_slice() == &*picture.data);
    if let Some((name, _)) = known {
        return name.to_string();
    }
    let extension = picture.format.extension();
    let taken = |n: usize| {
        let stem = format!("xl/media/image{n}.");
        extras.parts.iter().any(|part| part.name.starts_with(&stem))
    };
    let number = (1..).find(|n| !taken(*n)).unwrap_or(1);
    let name = format!("image{number}.{extension}");
    extras.add_default(extension, picture.format.mime());
    extras.add_part(format!("xl/media/{name}"), None, picture.data.to_vec());
    name
}

/// The image files of the backgrounds and what relates the worksheets to
/// them. The pictures of the drawings are in the package by now.
pub(crate) fn add_parts(wb: &Workbook, extras: &mut Extras) {
    for (index, sheet) in wb.sheets.iter().enumerate() {
        let (Some(picture), Some(id)) = (&sheet.background, SheetLinks::of(sheet).background)
        else {
            continue;
        };
        let name = media_name(picture, extras);
        extras.relate_sheet(
            index,
            id,
            &format!("{NS_DOC_REL}/image"),
            &format!("../media/{name}"),
        );
    }
}

/// Read the background of a worksheet from its XML and its relationships.
/// A picture that is missing or cannot be shown is no background.
pub(super) fn read_sheet<R: Read + Seek>(
    pkg: &mut Package<R>,
    part: &str,
    xml: &str,
    sheet: &mut Sheet,
) {
    // The element comes after the cells; most sheets have none.
    let Some(start) = xml.rfind("<picture ") else {
        return;
    };
    let mut id = None;
    let mut r = Reader::new(&xml[start..]);
    if let Event::Open(t) = r.next() {
        if t.name == "picture" {
            id = t.attr_local("id").map(|id| id.into_owned());
        }
    }
    let Some(id) = id else {
        return;
    };
    let target = relationships(pkg, part)
        .into_iter()
        .find(|rel| rel.id == id && rel.kind == "image")
        .map(|rel| rel.target);
    let Some(target) = target else {
        return;
    };
    sheet.background = pkg
        .read_bytes(&target)
        .ok()
        .flatten()
        .and_then(Picture::new);
}
