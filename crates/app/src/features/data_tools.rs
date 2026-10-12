//! The data tools of the Data tab: Text to Columns, Flash Fill, the
//! Advanced Filter, Consolidate, What-If Analysis (Goal Seek, the Scenario
//! Manager, Data Table), Get Data from a text file and Refresh All.
//!
//! Each tool has its file in `data_tools/`; the logic is in the engine
//! (`text_columns.rs`, `flashfill.rs`, `filter.rs`, `consolidate.rs`,
//! `whatif.rs`) and the files here hold the dialogs. `ui_tests.rs` drives
//! them in GPUI's headless window.

mod flash;
mod text_wizard;
#[cfg(test)]
mod ui_tests;

use super::{Command, Cx, Feature};
use crate::app::ExcelApp;
use crate::i18n::t;
use crate::shell::Alert;
use gpui::{App, Context, KeyBinding, SharedString, Stateful, Window, actions, prelude::*};
use opensheet_engine::Rect;

pub struct DataTools;

commands! {
    /// What the user can ask of the data tools.
    pub enum Cmd {
        TextToColumns = "columns", t("Text to Columns", "Convertir");
        FlashFill = "fill-down", t("Flash Fill", "Remplissage instantané");
    }
}

impl Command for Cmd {
    fn label(self) -> &'static str {
        self.face().1
    }

    fn icon(self) -> &'static str {
        self.face().0
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        match self {
            Cmd::TextToColumns => {
                text_wizard::open_text_to_columns(app, window, cx);
            }
            Cmd::FlashFill => flash::flash_fill(app, cx),
        }
    }
}

impl Feature for DataTools {
    fn name(&self) -> &'static str {
        "data-tools"
    }
}

// ---------------------------------------------------------------- shortcuts

actions!(data_tools, [FlashFill]);

/// The shortcuts of the data tools.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("ctrl-e", FlashFill, Some("Workbook"))]);
}

/// Make the window answer the shortcuts of `bind_keys`.
pub fn key_actions(root: Stateful<gpui::Div>, cx: &Context<ExcelApp>) -> Stateful<gpui::Div> {
    root.on_action(cx.listener(|app, _: &FlashFill, window, cx| app.run_command(Cmd::FlashFill, window, cx)))
}

/// Tell the user why what was asked cannot be done.
fn warn(app: &mut ExcelApp, message: impl Into<SharedString>, cx: &mut Cx) {
    app.alert(Alert::new("OpenSheet", message).warning(), cx);
}

/// The single range a reference typed in a dialog stands for, with its
/// sheet; `None` when the text is not a reference to one range.
fn typed_range(app: &ExcelApp, text: &str) -> Option<(usize, Rect)> {
    let (row, col) = app.active();
    let areas = app.wb.reference_areas(app.sheet, row, col, text)?;
    match areas.as_slice() {
        [area] => Some((area.sheet, area.rect)),
        _ => None,
    }
}
