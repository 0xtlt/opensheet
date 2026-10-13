//! The Automate tab — Office Scripts — and the VBA macros of
//! macro-enabled workbooks, which are kept and shown but not run.
//!
//! * `scripts.rs`: where the scripts live (`.osts` files in a folder).
//! * `samples.rs`: the sample scripts of the gallery.
//! * `run.rs`: running a script on the workbook, away from the window.
//! * `pane.rs`: the Code Editor pane; `editor.rs`, `buffer.rs` and
//!   `syntax.rs`: the editor in it.
//! * `macros.rs`: the Macro dialog, the viewer of VBA modules, the
//!   message bar of a workbook that came with macros.
//!
//! * `record/`: Record Actions, which writes a script from what the user
//!   does in the window.
//!
//! "Record Macro" of View ▸ Macros is not here: it would write VBA, which
//! OpenSheet does not run.

mod buffer;
mod editor;
mod macros;
mod pane;
mod record;
mod run;
mod samples;
#[cfg(test)]
mod sample_tests;
mod scripts;
mod syntax;
#[cfg(test)]
mod ui_tests;

pub(crate) use macros::{default_extension, loses_macros, macro_free_alert, message_bar};
pub(crate) use scripts::folder as scripts_folder;

use self::pane::{CodeEditorPane, ScriptRef};
use self::samples::SAMPLES;
use super::{Command, Cx, Feature};
use crate::app::ExcelApp;
use crate::i18n::t;
use crate::ribbon::{group, row};
use crate::theme::{self, c};
use crate::view::{icon, tooltip};
use gpui::{AnyElement, App, Context, Entity, KeyBinding, Stateful, Window, actions, div, prelude::*, px};
use std::path::PathBuf;

pub struct Automate;

commands! {
    /// What the user can ask of scripts and macros.
    pub enum Cmd {
        RecordActions = "automate-record", t("Record Actions", "Enregistrer les actions");
        NewScript = "automate-new", t("New Script", "Nouveau script");
        AllScripts = "automate-script", t("All Scripts", "Tous les scripts");
        ViewMacros = "automate-macro", t("Macros", "Macros");
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
        if app.is_editing() && !app.commit_edit(None, window, cx) {
            return;
        }
        match self {
            Cmd::RecordActions => {
                let recorder = record::Recorder::start(app);
                let pane = open_pane(app, cx);
                pane.update(cx, |pane, cx| pane.start_recording(recorder, window, cx));
            }
            Cmd::NewScript => {
                let pane = open_pane(app, cx);
                pane.update(cx, |pane, cx| pane.new_script(window, cx));
            }
            Cmd::AllScripts => {
                let pane = open_pane(app, cx);
                pane.update(cx, |pane, cx| pane.show_list(window, cx));
            }
            Cmd::ViewMacros => {
                macros::open_dialog(app, window, cx);
            }
        }
    }
}

actions!(automate, [ViewMacros]);

pub fn bind_keys(cx: &mut App) {
    editor::bind_keys(cx);
    // ⌥F8 opens the Macro dialog, as in Excel.
    cx.bind_keys([KeyBinding::new("alt-f8", ViewMacros, Some("Workbook"))]);
}

/// Make the window answer the shortcut and the menu item.
pub fn key_actions(root: Stateful<gpui::Div>, cx: &Context<ExcelApp>) -> Stateful<gpui::Div> {
    root.on_action(cx.listener(|app, _: &ViewMacros, window, cx| app.run_command(Cmd::ViewMacros, window, cx)))
}

// ---------------------------------------------------------------- the pane

const PANE_ID: &str = "code-editor";

/// What the window keeps of the Code Editor: the pane lives on while it
/// is closed, with the script being edited.
#[derive(Default)]
struct Ui {
    pane: Option<Entity<CodeEditorPane>>,
}

/// Show the Code Editor pane.
pub(crate) fn open_pane(app: &mut ExcelApp, cx: &mut Cx) -> Entity<CodeEditorPane> {
    let pane = match app.ext::<Ui>().and_then(|ui| ui.pane.clone()) {
        Some(pane) => pane,
        None => {
            let weak = cx.weak_entity();
            let pane = cx.new(|cx| CodeEditorPane::new(weak, cx));
            app.ext_mut::<Ui>().pane = Some(pane.clone());
            pane
        }
    };
    app.open_pane(PANE_ID, t("Code Editor", "Éditeur de code"), pane.clone(), cx);
    pane
}

/// The pane, when it was opened in this window.
#[cfg(test)]
pub(crate) fn pane(app: &ExcelApp) -> Option<Entity<CodeEditorPane>> {
    app.ext::<Ui>().and_then(|ui| ui.pane.clone())
}

/// Bring the Automate tab forward.
pub(crate) fn show_tab(app: &mut ExcelApp, cx: &mut Cx) {
    app.context_tab = None;
    app.ribbon_tab = TAB;
    cx.notify();
}

/// The place of the Automate tab among the tabs of the ribbon.
const TAB: usize = 9;

// ---------------------------------------------------------------- the ribbon

/// The controls of the gallery: a drop-down hangs from a control by its
/// name, and names are fixed.
const TILE_IDS: [&str; 6] = ["au-tile-0", "au-tile-1", "au-tile-2", "au-tile-3", "au-tile-4", "au-tile-5"];

/// One script of the gallery: a click shows its page in the Code Editor,
/// where it runs.
fn tile(app: &ExcelApp, index: usize, script: ScriptRef, name: String, cx: &mut Cx) -> AnyElement {
    let id = TILE_IDS[index];
    let (line1, line2) = {
        let (a, b) = crate::ribbon::split_label(&name);
        (a.to_string(), b.to_string())
    };
    div()
        .id(id)
        .debug_selector(|| id.to_string())
        .relative()
        .h(px(68.))
        .w(px(92.))
        .px(px(3.))
        .pt(px(4.))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(2.))
        .rounded(px(4.))
        .hover(|s| s.bg(c(theme::HOVER)))
        .active(|s| s.bg(c(theme::PRESSED)))
        .child(icon("automate-script", 26.0, theme::ACCENT_DARK))
        .child(
            div()
                .w_full()
                .flex()
                .flex_col()
                .items_center()
                .text_size(px(11.))
                .line_height(px(13.))
                .child(div().max_w_full().truncate().child(line1))
                .child(div().max_w_full().truncate().child(line2)),
        )
        .child(app.anchor(id))
        .tooltip(tooltip(name))
        .on_click(cx.listener(move |app, _, window, cx| {
            let pane = open_pane(app, cx);
            pane.update(cx, |pane, cx| pane.show_script(script.clone(), window, cx));
        }))
        .into_any_element()
}

/// The scripts of the gallery: the user's most recent ones, then the
/// samples, fewer of each as the window narrows.
fn gallery(app: &ExcelApp, cx: &mut Cx) -> Vec<AnyElement> {
    let each = match crate::ribbon_fold::level() {
        0 => 3,
        1 => 2,
        2 => 1,
        _ => 0,
    };
    let recent = scripts::list().into_iter().take(each).map(|entry| (ScriptRef::Mine(entry.name.clone()), entry.name));
    let samples = SAMPLES.iter().enumerate().take(each).map(|(index, sample)| (ScriptRef::Sample(index), sample.name().to_string()));
    recent.chain(samples).enumerate().map(|(index, (script, name))| tile(app, index, script, name, cx)).collect()
}

/// The groups of the Automate tab.
pub fn automate_tab(app: &ExcelApp, cx: &mut Cx) -> Vec<AnyElement> {
    let mut scripts = gallery(app, cx);
    scripts.push(app.cmd_large("au-all", Cmd::AllScripts, false, cx));
    vec![
        group(t("Scripting Tools", "Outils de script"), row(vec![app.cmd_large("au-record", Cmd::RecordActions, false, cx), app.cmd_large("au-new", Cmd::NewScript, false, cx)])).into_any_element(),
        group(t("Office Scripts", "Scripts Office"), row(scripts)).border_r_0().into_any_element(),
    ]
}

/// The Macros group of the View tab.
pub fn macros_group(app: &ExcelApp, cx: &mut Cx) -> gpui::Div {
    group(t("Macros", "Macros"), row(vec![app.cmd_large("vw-macros", Cmd::ViewMacros, false, cx)]))
}

// ------------------------------------------------------------------ saving

/// Before a workbook is written to a file: when the file is of a kind
/// that cannot hold the macros the workbook carries, Excel's question is
/// asked and the answer is true — the file is written, or not, by the
/// answer to the question.
pub(crate) fn asks_before_saving(app: &mut ExcelApp, path: &std::path::Path, cx: &mut Cx) -> bool {
    if !loses_macros(app, path) {
        return false;
    }
    app.alert(macro_free_alert(PathBuf::from(path)), cx);
    true
}

impl Feature for Automate {
    fn name(&self) -> &'static str {
        "automate"
    }
}
