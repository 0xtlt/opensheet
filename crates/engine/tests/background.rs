//! The background picture of a sheet: set, removed, undone, copied with
//! the sheet.

use opensheet_engine::drawing::{ImageFormat, Picture};
use opensheet_engine::{Locale, Workbook};

/// A PNG image of one pixel.
const PIXEL: [u8; 67] = [
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00,
    0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44, 0x41, 0x54, 0x78,
    0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44,
    0xAE, 0x42, 0x60, 0x82,
];

#[test]
fn a_background_is_set_removed_and_undone() {
    let mut wb = Workbook::new(Locale::en());
    let picture = Picture::new(PIXEL.to_vec()).expect("a PNG image");
    assert_eq!((picture.format, picture.pixels), (ImageFormat::Png, (1, 1)));
    assert!(Picture::new(b"not an image".to_vec()).is_none());

    assert!(wb.set_sheet_background(0, Some(picture.clone())));
    assert!(wb.commit("Background"));
    assert_eq!(wb.sheets[0].background, Some(picture.clone()));
    assert!(!wb.set_sheet_background(7, None));

    // The copy of a sheet has its background, the very same bytes.
    let copy = wb.duplicate_sheet(0).expect("a copy");
    wb.commit("Copy");
    let copied = wb.sheets[copy].background.as_ref().expect("the background of the copy");
    assert!(std::sync::Arc::ptr_eq(&copied.data, &picture.data));

    assert!(wb.set_sheet_background(0, None));
    assert!(wb.commit("Delete Background"));
    assert_eq!(wb.sheets[0].background, None);
    assert!(wb.sheets[copy].background.is_some());

    assert!(wb.undo());
    assert_eq!(wb.sheets[0].background, Some(picture.clone()));
    assert!(wb.undo() && wb.undo());
    assert_eq!((wb.sheets.len(), wb.sheets[0].background.is_none()), (1, true));
    assert!(wb.redo());
    assert_eq!(wb.sheets[0].background, Some(picture));
}
