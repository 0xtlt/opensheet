//! Custom views (View ▸ Custom Views): named snapshots of how the workbook
//! is looked at.
//!
//! A view keeps, for every sheet, the state of its window (zoom, frozen
//! panes, selection, scroll position, view mode, gridlines and headings)
//! and, when asked, the hidden rows and columns with the filter, and the
//! print settings. Showing a view puts all of that back.
//!
//! As in a file, a view is stored sheet by sheet (`Sheet::custom_views`):
//! each sheet holds its part of every view, under the identifier of the
//! view. That way a view follows its sheets through renaming, moving and
//! undo without anything to keep in step; a sheet added after a view was
//! made has no part in it and is left as it is when the view is shown.
//!
//! The window itself belongs to the application: it hands its state over
//! as [`WindowState`] values when a view is added and gets them back when
//! one is shown.

use crate::filter::AutoFilter;
use crate::page::PageSetup;
use crate::refs::Rect;
use crate::workbook::{Sheet, Workbook};
use std::collections::BTreeSet;

/// How a sheet is displayed (`view` of a sheet view in files).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ViewKind {
    #[default]
    Normal,
    PageLayout,
    PageBreakPreview,
}

impl ViewKind {
    pub fn ooxml_name(self) -> &'static str {
        match self {
            ViewKind::Normal => "normal",
            ViewKind::PageLayout => "pageLayout",
            ViewKind::PageBreakPreview => "pageBreakPreview",
        }
    }

    pub fn from_ooxml_name(name: &str) -> ViewKind {
        match name {
            "pageLayout" => ViewKind::PageLayout,
            "pageBreakPreview" => ViewKind::PageBreakPreview,
            _ => ViewKind::Normal,
        }
    }
}

/// The window of one sheet, as a view keeps it.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowState {
    /// Zoom in percent.
    pub zoom: u32,
    pub active: (u32, u32),
    /// The selected ranges; the last one holds the active cell.
    pub ranges: Vec<Rect>,
    /// The first cell shown in the part of the window that scrolls.
    pub top_left: (u32, u32),
    pub frozen_rows: u32,
    pub frozen_cols: u32,
    pub show_gridlines: bool,
    pub show_headers: bool,
    pub mode: ViewKind,
    /// This sheet was the one in front.
    pub selected: bool,
}

impl Default for WindowState {
    fn default() -> WindowState {
        WindowState {
            zoom: 100,
            active: (0, 0),
            ranges: vec![Rect::cell(0, 0)],
            top_left: (0, 0),
            frozen_rows: 0,
            frozen_cols: 0,
            show_gridlines: true,
            show_headers: true,
            mode: ViewKind::Normal,
            selected: false,
        }
    }
}

/// "Hidden rows, columns and filter settings" of a view, for one sheet.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HiddenState {
    pub rows: BTreeSet<u32>,
    pub cols: BTreeSet<u32>,
    /// The sheet's filter with its criteria; the rows it hides are found
    /// again when the view is shown.
    pub filter: Option<AutoFilter>,
}

/// The part of a custom view one sheet holds.
#[derive(Clone, Debug, PartialEq)]
pub struct CustomSheetView {
    /// Identifier of the view, the same on every sheet: `{8-4-4-4-12}`
    /// hexadecimal digits in upper case, as in files.
    pub guid: String,
    pub name: String,
    pub window: WindowState,
    /// `None`: the view was made without "Hidden rows, columns and filter
    /// settings".
    pub hidden: Option<HiddenState>,
    /// `None`: the view was made without "Print settings".
    pub page: Option<PageSetup>,
}

/// A custom view of the workbook, as the dialog lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewInfo {
    pub name: String,
    pub guid: String,
    pub print_settings: bool,
    pub hidden_settings: bool,
}

/// Why a view cannot be added.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewError {
    /// The name is empty.
    NoName,
    /// A sheet holds a table: Excel has no custom views then.
    Tables,
    /// One state per sheet is needed.
    Sheets,
}

/// What showing a view asks of the window: the state of each sheet that
/// has a part in the view, by index.
pub type Shown = Vec<(usize, WindowState)>;

/// True for a well-formed identifier of a view.
pub fn is_guid(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 38
        && bytes[0] == b'{'
        && bytes[37] == b'}'
        && bytes[1..37].iter().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => *b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}

impl Workbook {
    /// Whether custom views can be used: not while a sheet holds a table.
    pub fn custom_views_available(&self) -> bool {
        self.sheets.iter().all(|sheet| sheet.tables.is_empty())
    }

    /// The custom views, in the order they were added.
    pub fn custom_views(&self) -> Vec<ViewInfo> {
        let mut out: Vec<ViewInfo> = Vec::new();
        for view in self.sheets.iter().flat_map(|sheet| &sheet.custom_views) {
            if !out.iter().any(|known| known.guid == view.guid) {
                out.push(ViewInfo {
                    name: view.name.clone(),
                    guid: view.guid.clone(),
                    print_settings: view.page.is_some(),
                    hidden_settings: view.hidden.is_some(),
                });
            }
        }
        out
    }

    /// The view of that name (names differ by more than case).
    pub fn custom_view(&self, name: &str) -> Option<ViewInfo> {
        let wanted = name.trim().to_lowercase();
        self.custom_views()
            .into_iter()
            .find(|view| view.name.to_lowercase() == wanted)
    }

    /// An identifier no view of the workbook has. It only has to be
    /// unique in the file: it is made from the views there are.
    fn new_view_guid(&self, name: &str) -> String {
        let taken: Vec<String> = self
            .custom_views()
            .into_iter()
            .map(|view| view.guid)
            .collect();
        // FNV-1a over the name, then a counter until the identifier is free.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in name.bytes().chain(self.generation.to_le_bytes()) {
            hash = (hash ^ byte as u64).wrapping_mul(0x0000_0100_0000_01b3);
        }
        let mut salt = taken.len() as u64;
        loop {
            let (a, b) = (
                hash,
                hash.rotate_left(29) ^ salt.wrapping_mul(0x9e37_79b9_7f4a_7c15),
            );
            let guid = format!(
                "{{{:08X}-{:04X}-4{:03X}-{:04X}-{:012X}}}",
                (a >> 32) as u32,
                (a >> 16) as u16,
                a as u16 & 0x0FFF,
                (b >> 48) as u16 & 0x3FFF | 0x8000,
                b & 0xFFFF_FFFF_FFFF
            );
            if !taken.contains(&guid) {
                return guid;
            }
            salt += 1;
        }
    }

    /// Add a view of the workbook as it is now; `windows` gives the window
    /// of each sheet, in order. A view of the same name is replaced.
    /// Recorded for undo.
    pub fn add_custom_view(
        &mut self,
        name: &str,
        print_settings: bool,
        hidden_settings: bool,
        windows: &[WindowState],
    ) -> Result<(), ViewError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ViewError::NoName);
        }
        if !self.custom_views_available() {
            return Err(ViewError::Tables);
        }
        if windows.len() != self.sheets.len() {
            return Err(ViewError::Sheets);
        }
        self.delete_custom_view(name);
        let guid = self.new_view_guid(name);
        self.snapshot();
        for (sheet, window) in self.sheets.iter_mut().zip(windows) {
            let mut window = window.clone();
            // What the sheet itself knows is taken from it.
            window.frozen_rows = sheet.frozen_rows;
            window.frozen_cols = sheet.frozen_cols;
            window.show_gridlines = sheet.show_gridlines;
            window.show_headers = sheet.show_headers;
            let view = CustomSheetView {
                guid: guid.clone(),
                name: name.to_string(),
                window,
                hidden: hidden_settings.then(|| HiddenState {
                    rows: sheet.hidden_rows.clone(),
                    cols: sheet.hidden_cols.clone(),
                    filter: sheet.auto_filter.clone(),
                }),
                page: print_settings.then(|| sheet.page.clone()),
            };
            sheet.custom_views.push(view);
        }
        Ok(())
    }

    /// Delete a view; false when there is none of that name.
    pub fn delete_custom_view(&mut self, name: &str) -> bool {
        let Some(view) = self.custom_view(name) else {
            return false;
        };
        self.snapshot();
        for sheet in &mut self.sheets {
            sheet.custom_views.retain(|part| part.guid != view.guid);
        }
        true
    }

    /// Show a view: every sheet that has a part in it gets its hidden rows
    /// and columns, its filter and its print settings back (what the view
    /// kept of them), its frozen panes, gridlines and headings. Returns
    /// what is left for the window to do; `None` when there is no view of
    /// that name. Recorded for undo.
    pub fn show_custom_view(&mut self, name: &str) -> Option<Shown> {
        let view = self.custom_view(name)?;
        self.snapshot();
        let mut shown = Vec::new();
        for index in 0..self.sheets.len() {
            let sheet = &mut self.sheets[index];
            let Some(part) = sheet
                .custom_views
                .iter()
                .find(|part| part.guid == view.guid)
                .cloned()
            else {
                continue;
            };
            sheet.frozen_rows = part.window.frozen_rows;
            sheet.frozen_cols = part.window.frozen_cols;
            sheet.show_gridlines = part.window.show_gridlines;
            sheet.show_headers = part.window.show_headers;
            if let Some(page) = part.page {
                sheet.page = page;
            }
            if let Some(hidden) = part.hidden {
                sheet.hidden_rows = hidden.rows;
                sheet.hidden_cols = hidden.cols;
                sheet.auto_filter = hidden.filter;
                // The rows the filter hides, with the values of today.
                self.reapply_filters(index);
            }
            shown.push((index, part.window));
        }
        Some(shown)
    }
}

/// See `features::on_sheet_duplicated`. (Rows and columns inserted or
/// deleted since a view was made are not followed: a view is a picture of
/// the sheet as it was.)
pub(crate) fn on_sheet_duplicated(wb: &mut Workbook, copy: usize, _original: &str) {
    // The copy has no part in the views of the workbook: it was not there
    // when they were made.
    wb.sheets[copy].custom_views.clear();
}

impl Sheet {
    /// The part this sheet has in the view of an identifier.
    pub fn custom_view(&self, guid: &str) -> Option<&CustomSheetView> {
        self.custom_views.iter().find(|part| part.guid == guid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locale::Locale;
    use crate::page::Orientation;

    fn workbook() -> Workbook {
        let mut wb = Workbook::new(Locale::en());
        wb.add_sheet(1);
        for row in 0..6 {
            wb.set_cell_text(
                0,
                row,
                0,
                if row == 0 {
                    "Name"
                } else if row % 2 == 0 {
                    "a"
                } else {
                    "b"
                },
            )
            .unwrap();
        }
        wb.commit("setup");
        wb
    }

    fn windows(wb: &Workbook) -> Vec<WindowState> {
        vec![WindowState::default(); wb.sheets.len()]
    }

    #[test]
    fn a_view_brings_back_what_it_kept() {
        let mut wb = workbook();
        wb.snapshot();
        wb.sheets[0].hidden_rows.insert(3);
        wb.sheets[0].hidden_cols.insert(2);
        wb.sheets[0].frozen_rows = 1;
        wb.sheets[0].show_gridlines = false;
        wb.sheets[0].page.orientation = Orientation::Landscape;
        wb.commit("setup");
        let mut state = windows(&wb);
        state[0].zoom = 150;
        state[0].active = (4, 1);
        state[0].ranges = vec![Rect::new(4, 1, 5, 2)];
        state[0].selected = true;
        assert_eq!(wb.add_custom_view(" Narrow ", true, true, &state), Ok(()));
        wb.commit("Custom View");
        let views = wb.custom_views();
        assert_eq!(views.len(), 1);
        assert_eq!(
            (
                views[0].name.as_str(),
                views[0].print_settings,
                views[0].hidden_settings
            ),
            ("Narrow", true, true)
        );
        assert!(is_guid(&views[0].guid), "{}", views[0].guid);
        // Everything changes...
        wb.snapshot();
        let sheet = &mut wb.sheets[0];
        sheet.hidden_rows.clear();
        sheet.hidden_cols.clear();
        sheet.frozen_rows = 0;
        sheet.show_gridlines = true;
        sheet.page.orientation = Orientation::Portrait;
        wb.commit("changes");
        // ... and the view puts it back.
        let shown = wb.show_custom_view("narrow").expect("the view");
        wb.commit("Custom View");
        let sheet = &wb.sheets[0];
        assert!(sheet.hidden_rows.contains(&3) && sheet.hidden_cols.contains(&2));
        assert_eq!(
            (
                sheet.frozen_rows,
                sheet.show_gridlines,
                sheet.page.orientation
            ),
            (1, false, Orientation::Landscape)
        );
        assert_eq!(shown.len(), 2);
        assert_eq!(
            (shown[0].1.zoom, shown[0].1.active, shown[0].1.selected),
            (150, (4, 1), true)
        );
        assert_eq!(shown[0].1.frozen_rows, 1);
        // Showing a view is one undo step.
        assert!(wb.undo());
        assert!(wb.sheets[0].hidden_rows.is_empty());
        assert_eq!(wb.show_custom_view("nothing"), None);
    }

    #[test]
    fn a_view_keeps_only_what_was_asked() {
        let mut wb = workbook();
        wb.snapshot();
        wb.sheets[0].hidden_rows.insert(2);
        wb.sheets[0].page.orientation = Orientation::Landscape;
        wb.commit("setup");
        wb.add_custom_view("Window only", false, false, &windows(&wb))
            .unwrap();
        wb.commit("Custom View");
        wb.snapshot();
        wb.sheets[0].hidden_rows.clear();
        wb.sheets[0].page.orientation = Orientation::Portrait;
        wb.commit("changes");
        wb.show_custom_view("Window only").unwrap();
        // Neither the hidden row nor the orientation came back.
        assert!(wb.sheets[0].hidden_rows.is_empty());
        assert_eq!(wb.sheets[0].page.orientation, Orientation::Portrait);
    }

    #[test]
    fn a_view_keeps_the_filter_and_filters_again() {
        let mut wb = workbook();
        wb.set_auto_filter(0, Rect::new(0, 0, 5, 0)).unwrap();
        let only_a = crate::filter::Criteria::Values(crate::filter::ValueFilter {
            values: vec!["a".to_string()],
            blanks: false,
            dates: Vec::new(),
        });
        assert!(wb.set_filter_criteria(0, crate::filter::FilterScope::Sheet, 0, Some(only_a)));
        wb.commit("Filter");
        let filtered = wb.sheets[0].filtered_rows.clone();
        assert_eq!(filtered.len(), 3);
        wb.add_custom_view("Only a", false, true, &windows(&wb))
            .unwrap();
        wb.commit("Custom View");
        wb.remove_auto_filter(0);
        wb.commit("Filter");
        assert!(wb.sheets[0].filtered_rows.is_empty());
        wb.show_custom_view("Only a").unwrap();
        wb.commit("Custom View");
        assert!(
            wb.sheets[0]
                .auto_filter
                .as_ref()
                .is_some_and(AutoFilter::is_filtered)
        );
        assert_eq!(wb.sheets[0].filtered_rows, filtered);
    }

    #[test]
    fn views_are_replaced_deleted_and_refused() {
        let mut wb = workbook();
        assert_eq!(
            wb.add_custom_view("  ", true, true, &windows(&wb)),
            Err(ViewError::NoName)
        );
        assert_eq!(
            wb.add_custom_view("One", true, true, &[]),
            Err(ViewError::Sheets)
        );
        wb.add_custom_view("One", true, true, &windows(&wb))
            .unwrap();
        wb.add_custom_view("Two", false, true, &windows(&wb))
            .unwrap();
        // The same name again, in another case: the view is replaced and
        // goes to the end.
        wb.add_custom_view("ONE", false, false, &windows(&wb))
            .unwrap();
        wb.commit("Custom View");
        let names: Vec<String> = wb
            .custom_views()
            .into_iter()
            .map(|view| view.name)
            .collect();
        assert_eq!(names, vec!["Two", "ONE"]);
        let guids: Vec<String> = wb
            .custom_views()
            .into_iter()
            .map(|view| view.guid)
            .collect();
        assert_ne!(guids[0], guids[1]);
        assert!(wb.delete_custom_view("two"));
        assert!(!wb.delete_custom_view("two"));
        wb.commit("Custom View");
        assert_eq!(wb.custom_views().len(), 1);
        // Undo brings the deleted view back.
        wb.undo();
        assert_eq!(wb.custom_views().len(), 2);
        // A sheet added later has no part in the views; a copy neither.
        wb.add_sheet(2);
        wb.duplicate_sheet(0);
        wb.commit("sheets");
        assert_eq!(
            wb.sheets
                .iter()
                .filter(|sheet| !sheet.custom_views.is_empty())
                .count(),
            2
        );
        let shown = wb.show_custom_view("Two").unwrap();
        assert_eq!(shown.len(), 2);
        // With a table in the workbook there are no custom views.
        wb.add_table(0, Rect::new(0, 0, 5, 0), true).unwrap();
        assert!(!wb.custom_views_available());
        assert_eq!(
            wb.add_custom_view("Three", true, true, &windows(&wb)),
            Err(ViewError::Tables)
        );
    }
}
