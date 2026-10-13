//! The Code Editor pane, docked right of the grid as Excel's: the list of
//! scripts (the user's own, then the samples), the page of one script
//! (Run, Edit and the "…" menu), and the editor with its Output and
//! Problems lists.

use super::editor::{CodeEditor, EditorEvent};
use super::record::Recorder;
use super::run::{self, OutputKind, OutputLine, Runs};
use super::samples::SAMPLES;
use super::scripts;
use super::syntax::Language;
use crate::app::{ExcelApp, MenuEntry};
use crate::i18n::t;
use crate::shell::{Alert, Modal};
use crate::text_input::TextInput;
use crate::theme::{self, c};
use crate::view::{icon, tooltip};
use crate::widgets::{self, Button};
use gpui::{
    AnyElement, App, Context, Entity, Focusable, FontWeight, PathPromptOptions, SharedString, Subscription, Task, WeakEntity, Window, div,
    prelude::*, px,
};
use opensheet_script::{LogLevel, Script, ScriptError};
use std::cell::RefCell;
use std::time::Duration;

/// How long after the last key the text is checked.
const CHECK_DELAY: Duration = Duration::from_millis(400);

/// A script of the pane.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ScriptRef {
    /// One of the user's, by its name.
    Mine(String),
    /// A sample, by its place in `SAMPLES`.
    Sample(usize),
}

/// What the pane shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Page {
    List,
    /// The page of a script: its name, Run, Edit.
    Script(ScriptRef),
    /// The editor, on the script named by `CodeEditorPane::open`.
    Editor,
    /// Record Actions: the actions recorded so far, and Stop.
    Recording,
}

/// The lists under the editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Bottom {
    Output,
    Problems,
}

thread_local! {
    /// The script being edited and its text when it has unsaved changes.
    /// It outlives the pane, which goes when another workbook is opened:
    /// the pane that comes next takes the text up again.
    static DRAFT: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
}

pub(crate) struct CodeEditorPane {
    app: WeakEntity<ExcelApp>,
    pub(crate) page: Page,
    pub(crate) editor: Entity<CodeEditor>,
    /// The script whose text the editor holds.
    pub(crate) open: Option<String>,
    /// What `check` found wrong with the text of the editor.
    pub(crate) problems: Vec<ScriptError>,
    pub(crate) bottom: Bottom,
    /// What went wrong with a file, shown until the next action.
    pub(crate) trouble: Option<String>,
    /// The check waiting for the typing to pause.
    check: Option<Task<()>>,
    /// `Runs::finished` as last seen.
    seen_finished: u64,
    /// What records the actions of the user, while Record Actions is on.
    pub(crate) recorder: Option<Recorder>,
    /// Hears of every change of the window while recording.
    record_watch: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl CodeEditorPane {
    pub fn new(app: WeakEntity<ExcelApp>, cx: &mut Context<Self>) -> Self {
        let editor = cx.new(|cx| CodeEditor::new("", Language::TypeScript, cx));
        let mut subscriptions = vec![cx.subscribe(&editor, Self::editor_event)];
        if let Some(app) = app.upgrade() {
            subscriptions.push(cx.observe(&app, Self::app_changed));
        }
        let mut pane = CodeEditorPane {
            app,
            page: Page::List,
            editor,
            open: None,
            problems: Vec::new(),
            bottom: Bottom::Output,
            trouble: None,
            check: None,
            seen_finished: 0,
            recorder: None,
            record_watch: None,
            _subscriptions: subscriptions,
        };
        // Unsaved text left by the pane of another workbook.
        if let Some((name, text)) = DRAFT.with(|draft| draft.borrow().clone()) {
            if let Ok(script) = scripts::load(&name) {
                pane.editor.update(cx, |editor, cx| {
                    editor.set_text(&script.body, cx);
                    editor.replace_all(&text, cx);
                });
                pane.open = Some(name);
                pane.page = Page::Editor;
            }
        }
        pane
    }

    /// Do something with the window once whatever is going on is over:
    /// the pane is often asked for something from inside a command of
    /// the window, which cannot be given another one before it returns.
    fn with_app(&self, cx: &mut Context<Self>, f: impl FnOnce(&mut ExcelApp, &mut Context<ExcelApp>) + 'static) {
        let app = self.app.clone();
        cx.defer(move |cx| {
            app.update(cx, f).ok();
        });
    }

    /// The same, for what needs the window itself.
    fn with_app_in(&self, window: &mut Window, cx: &mut Context<Self>, f: impl FnOnce(&mut ExcelApp, &mut Window, &mut Context<ExcelApp>) + 'static) {
        let app = self.app.clone();
        window.defer(cx, move |window, cx| {
            app.update(cx, |app, cx| f(app, window, cx)).ok();
        });
    }

    /// The script whose page or text is shown.
    pub(crate) fn current(&self) -> Option<ScriptRef> {
        match &self.page {
            Page::List | Page::Recording => None,
            Page::Script(script) => Some(script.clone()),
            Page::Editor => self.open.clone().map(ScriptRef::Mine),
        }
    }

    /// True when the editor holds changes that are not in the file.
    pub(crate) fn unsaved(&self, cx: &App) -> bool {
        self.open.is_some() && self.editor.read(cx).is_dirty()
    }

    fn remember_draft(&self, cx: &App) {
        let draft = self.open.clone().filter(|_| self.unsaved(cx)).map(|name| (name, self.editor.read(cx).text().to_string()));
        DRAFT.with(|slot| *slot.borrow_mut() = draft);
    }

    // ---- the editor ------------------------------------------------------

    fn editor_event(&mut self, _: Entity<CodeEditor>, event: &EditorEvent, cx: &mut Context<Self>) {
        match event {
            EditorEvent::Changed => {
                self.remember_draft(cx);
                // Check once the typing pauses.
                self.check = Some(cx.spawn(async move |pane, cx| {
                    cx.background_executor().timer(CHECK_DELAY).await;
                    let Ok(text) = pane.update(cx, |pane, cx| pane.editor.read(cx).text().to_string()) else {
                        return;
                    };
                    let engine = run::engine();
                    let checked = text.clone();
                    let problems = cx.background_executor().spawn(async move { engine.check(&checked) }).await;
                    pane.update(cx, |pane, cx| {
                        // Typed on since: the next check will tell.
                        if pane.editor.read(cx).text() == text {
                            pane.set_problems(problems, cx);
                        }
                    })
                    .ok();
                }));
                cx.notify();
            }
            EditorEvent::Save => self.save(cx),
        }
    }

    fn set_problems(&mut self, problems: Vec<ScriptError>, cx: &mut Context<Self>) {
        let marks = problems.iter().filter_map(|problem| Some((problem.line?, SharedString::from(problem.message.clone())))).collect();
        self.editor.update(cx, |editor, cx| editor.set_marks(marks, cx));
        // A first problem brings its list forward; the last one solved
        // gives the place back to the output.
        if !problems.is_empty() && self.problems.is_empty() {
            self.bottom = Bottom::Problems;
        } else if problems.is_empty() && self.bottom == Bottom::Problems {
            self.bottom = Bottom::Output;
        }
        self.problems = problems;
        cx.notify();
    }

    /// Check the text of the editor now, without waiting for a pause.
    fn check_now(&mut self, cx: &mut Context<Self>) {
        self.check = None;
        let problems = run::engine().check(self.editor.read(cx).text());
        self.set_problems(problems, cx);
    }

    /// The window changed: a run may have ended.
    fn app_changed(&mut self, app: Entity<ExcelApp>, cx: &mut Context<Self>) {
        let (finished, failed_at, of) = {
            let app = app.read(cx);
            match app.ext::<Runs>() {
                Some(runs) => (runs.finished, runs.failed_at, runs.output_of.clone()),
                None => (0, None, String::new()),
            }
        };
        if finished != self.seen_finished {
            self.seen_finished = finished;
            self.bottom = Bottom::Output;
            // An error puts the caret where it happened.
            if let (Some((line, column)), Page::Editor) = (failed_at, &self.page) {
                if self.open.as_deref() == Some(of.as_str()) {
                    self.editor.update(cx, |editor, cx| editor.go_to_line(line, column, cx));
                }
            }
        }
        cx.notify();
    }

    // ---- moving between pages ---------------------------------------------

    /// Do `then` right away when the editor has nothing to lose, else ask
    /// first, as Excel does when a changed script is left.
    fn leave_script(&mut self, window: &mut Window, cx: &mut Context<Self>, then: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static) {
        // While actions are recorded the pane shows them: Stop comes first.
        if self.recorder.is_some() {
            self.page = Page::Recording;
            cx.notify();
            return;
        }
        if !self.unsaved(cx) {
            then(self, window, cx);
            return;
        }
        let name = self.open.clone().unwrap_or_default();
        let pane = cx.weak_entity();
        let then = std::rc::Rc::new(then);
        let (save_then, discard_then) = (then.clone(), then);
        let (save_pane, discard_pane) = (pane.clone(), pane);
        let alert = Alert::new(
            "OpenSheet",
            t("Want to save your changes to '{}'?", "Voulez-vous enregistrer les modifications que vous avez apportées à « {} » ?").replace("{}", &name),
        )
        .warning()
        .button(t("Save", "Enregistrer"), move |_, window, cx| {
            save_pane
                .update(cx, |pane, cx| {
                    pane.save(cx);
                    // A script that could not be saved is not left.
                    if !pane.unsaved(cx) {
                        save_then(pane, window, cx);
                    }
                })
                .ok();
        })
        .button(t("Don't Save", "Ne pas enregistrer"), move |_, window, cx| {
            discard_pane
                .update(cx, |pane, cx| {
                    pane.discard(cx);
                    discard_then(pane, window, cx);
                })
                .ok();
        })
        .close_button(t("Cancel", "Annuler"));
        self.with_app(cx, move |app, cx| app.alert(alert, cx));
    }

    /// Forget the changes of the editor: it shows the file again.
    fn discard(&mut self, cx: &mut Context<Self>) {
        if let Some(script) = self.open.as_deref().and_then(|name| scripts::load(name).ok()) {
            self.editor.update(cx, |editor, cx| editor.set_text(&script.body, cx));
        }
        self.problems.clear();
        self.remember_draft(cx);
    }

    /// Back to the list of scripts.
    pub(crate) fn show_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_script(window, cx, |pane, _, cx| {
            pane.page = Page::List;
            pane.trouble = None;
            cx.notify();
        });
    }

    /// Show the page of a script.
    pub(crate) fn show_script(&mut self, script: ScriptRef, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_script(window, cx, move |pane, _, cx| {
            pane.page = Page::Script(script.clone());
            pane.trouble = None;
            cx.notify();
        });
    }

    /// The script of a reference: read from its file, or the sample.
    fn script_of(&self, script: &ScriptRef) -> Result<Script, String> {
        match script {
            ScriptRef::Mine(name) => scripts::load(name),
            ScriptRef::Sample(index) => SAMPLES.get(*index).map(|sample| sample.script()).ok_or_else(String::new),
        }
    }

    /// Put a script of the user in the editor.
    fn open_in_editor(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        match scripts::load(name) {
            Ok(script) => {
                self.editor.update(cx, |editor, cx| editor.set_text(&script.body, cx));
                self.open = Some(script.name);
                self.page = Page::Editor;
                self.trouble = None;
                self.problems.clear();
                self.bottom = Bottom::Output;
                self.remember_draft(cx);
                self.check_now(cx);
                window.focus(&self.editor.focus_handle(cx));
            }
            Err(error) => self.trouble = Some(error),
        }
        cx.notify();
    }

    /// New Script: a script holding what Excel starts one with, in the
    /// editor.
    pub(crate) fn new_script(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_script(window, cx, |pane, window, cx| match scripts::create() {
            Ok(script) => pane.open_in_editor(&script.name, window, cx),
            Err(error) => {
                pane.trouble = Some(error);
                cx.notify();
            }
        });
    }

    /// Edit: the script of the page goes into the editor. A sample is
    /// first copied among the user's scripts, under its own name.
    pub(crate) fn edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Page::Script(script) = self.page.clone() else {
            return;
        };
        let name = match &script {
            ScriptRef::Mine(name) => Ok(name.clone()),
            ScriptRef::Sample(_) => self.script_of(&script).and_then(|mut copy| {
                copy.name = scripts::free_name(&copy.name);
                scripts::save(&copy).map(|()| copy.name)
            }),
        };
        match name {
            Ok(name) => self.open_in_editor(&name, window, cx),
            Err(error) => {
                self.trouble = Some(error);
                cx.notify();
            }
        }
    }

    // ---- the commands of a script -----------------------------------------

    /// ⌘S and the Save button: the text of the editor goes to its file.
    pub(crate) fn save(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.open.clone() else {
            return;
        };
        // The description and whatever else the file holds stay.
        let mut script = scripts::load(&name).unwrap_or_else(|_| Script::new(&name));
        script.body = self.editor.read(cx).text().to_string();
        match scripts::save(&script) {
            Ok(()) => {
                self.editor.update(cx, |editor, cx| editor.mark_saved(cx));
                self.trouble = None;
            }
            Err(error) => self.trouble = Some(error),
        }
        self.remember_draft(cx);
        cx.notify();
    }

    /// Run the script shown: the text of the editor as it is, saved or
    /// not, or the script of the page.
    pub(crate) fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let script = match (&self.page, self.current()) {
            (Page::Editor, Some(ScriptRef::Mine(name))) => Ok(Script {
                body: self.editor.read(cx).text().to_string(),
                ..Script::new(&name)
            }),
            (_, Some(script)) => self.script_of(&script),
            (_, None) => return,
        };
        match script {
            Ok(script) => {
                self.trouble = None;
                self.bottom = Bottom::Output;
                self.with_app_in(window, cx, move |app, window, cx| {
                    run::start(app, &script.name, &script.body, window, cx);
                });
            }
            Err(error) => self.trouble = Some(error),
        }
        cx.notify();
    }

    pub(crate) fn stop(&mut self, cx: &mut Context<Self>) {
        self.with_app(cx, run::stop);
    }

    /// Give the script another name. False (and `trouble` says why) when
    /// the name cannot be taken.
    pub(crate) fn rename(&mut self, old: &str, new: &str, cx: &mut Context<Self>) -> bool {
        match scripts::rename(old, new) {
            Ok(new) => {
                if self.open.as_deref() == Some(old) {
                    self.open = Some(new.clone());
                    self.remember_draft(cx);
                }
                if self.page == Page::Script(ScriptRef::Mine(old.to_string())) {
                    self.page = Page::Script(ScriptRef::Mine(new));
                }
                self.trouble = None;
                cx.notify();
                true
            }
            Err(error) => {
                self.trouble = Some(error);
                cx.notify();
                false
            }
        }
    }

    /// Delete a script: its file goes to the Trash and the pane goes back
    /// to the list.
    pub(crate) fn delete(&mut self, name: &str, cx: &mut Context<Self>) {
        match scripts::delete(name) {
            Ok(()) => {
                if self.open.as_deref() == Some(name) {
                    self.open = None;
                    self.editor.update(cx, |editor, cx| editor.set_text("", cx));
                    self.problems.clear();
                    self.remember_draft(cx);
                }
                self.page = Page::List;
                self.trouble = None;
            }
            Err(error) => self.trouble = Some(error),
        }
        cx.notify();
    }

    /// Make a copy: the same script under a name of its own, shown on its
    /// page.
    pub(crate) fn make_copy(&mut self, script: &ScriptRef, cx: &mut Context<Self>) {
        let copy = match script {
            ScriptRef::Mine(name) => scripts::duplicate(name),
            ScriptRef::Sample(_) => self.script_of(script).and_then(|mut copy| {
                copy.name = scripts::free_name(&copy.name);
                scripts::save(&copy).map(|()| copy.name)
            }),
        };
        match copy {
            Ok(name) => {
                self.page = Page::Script(ScriptRef::Mine(name));
                self.trouble = None;
            }
            Err(error) => self.trouble = Some(error),
        }
        cx.notify();
    }

    /// Bring in an `.osts` file; the script is shown on its page.
    pub(crate) fn import(&mut self, file: &std::path::Path, cx: &mut Context<Self>) {
        match scripts::import(file) {
            Ok(name) => {
                self.page = Page::Script(ScriptRef::Mine(name));
                self.trouble = None;
            }
            Err(error) => self.trouble = Some(error),
        }
        cx.notify();
    }

    /// Write a copy of a script to a file the user chose.
    pub(crate) fn export(&mut self, script: &ScriptRef, target: &std::path::Path, cx: &mut Context<Self>) {
        let written = self.script_of(script).and_then(|mut script| {
            // The text being edited is what is exported.
            if self.open.as_deref() == Some(script.name.as_str()) && self.page == Page::Editor {
                script.body = self.editor.read(cx).text().to_string();
            }
            scripts::export(&script, target)
        });
        self.trouble = written.err();
        cx.notify();
    }

    /// Ask for a file to import.
    fn ask_import(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn(async move |pane, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                if let Some(path) = paths.into_iter().next() {
                    pane.update(cx, |pane, cx| pane.import(&path, cx)).ok();
                }
            }
        })
        .detach();
    }

    /// Ask where to export a script to.
    fn ask_export(&mut self, script: ScriptRef, cx: &mut Context<Self>) {
        let name = match &script {
            ScriptRef::Mine(name) => name.clone(),
            ScriptRef::Sample(index) => SAMPLES[*index].name().to_string(),
        };
        let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
        let target = cx.prompt_for_new_path(&home, Some(&format!("{name}.{}", scripts::EXTENSION)));
        cx.spawn(async move |pane, cx| {
            if let Ok(Ok(Some(mut path))) = target.await {
                if path.extension().is_none() {
                    path.set_extension(scripts::EXTENSION);
                }
                pane.update(cx, |pane, cx| pane.export(&script, &path, cx)).ok();
            }
        })
        .detach();
    }

    /// The "…" menu of a script: Rename, Delete, Make a copy, Export.
    pub(crate) fn menu_entries(&self, script: &ScriptRef, cx: &mut Context<Self>) -> Vec<MenuEntry> {
        let pane = cx.weak_entity();
        let mine = matches!(script, ScriptRef::Mine(_));
        let name = match script {
            ScriptRef::Mine(name) => name.clone(),
            ScriptRef::Sample(_) => String::new(),
        };
        let entry = |label: &'static str, icon_name: &'static str, action: std::rc::Rc<dyn Fn(&mut Self, &mut Window, &mut Context<Self>)>| {
            let pane = pane.clone();
            let entry = MenuEntry::new(label, move |_, window, cx| {
                pane.update(cx, |pane, cx| action(pane, window, cx)).ok();
            });
            if icon_name.is_empty() { entry } else { entry.icon(icon_name) }
        };
        let (rename_name, delete_name) = (name.clone(), name);
        let (copy_script, export_script) = (script.clone(), script.clone());
        vec![
            entry(
                t("Rename", "Renommer"),
                "",
                std::rc::Rc::new(move |pane, window, cx| pane.ask_rename(&rename_name, window, cx)),
            )
            .disabled(!mine),
            entry(t("Delete", "Supprimer"), "trash", std::rc::Rc::new(move |pane, _, cx| pane.ask_delete(&delete_name, cx))).disabled(!mine),
            entry(t("Make a Copy", "Créer une copie"), "copy", std::rc::Rc::new(move |pane, _, cx| pane.make_copy(&copy_script, cx))),
            MenuEntry::separator(),
            entry(t("Export…", "Exporter…"), "", std::rc::Rc::new(move |pane, _, cx| pane.ask_export(export_script.clone(), cx))),
        ]
    }

    fn open_menu(&mut self, script: ScriptRef, window: &mut Window, cx: &mut Context<Self>) {
        let entries = self.menu_entries(&script, cx);
        let position = window.mouse_position();
        self.with_app(cx, move |app, cx| app.open_context_menu(position, entries, cx));
    }

    /// The Rename dialog.
    pub(crate) fn ask_rename(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let pane = cx.weak_entity();
        let name = name.to_string();
        self.with_app_in(window, cx, move |app, window, cx| {
            app.open_modal(window, cx, |modal, _, cx| RenameDialog::new(modal, pane, name, cx));
        });
    }

    /// Excel's question before a script is deleted.
    pub(crate) fn ask_delete(&mut self, name: &str, cx: &mut Context<Self>) {
        let pane = cx.weak_entity();
        let target = name.to_string();
        let alert = Alert::new(
            t("Delete Script", "Supprimer le script"),
            t("Are you sure you want to delete '{}'?", "Êtes-vous sûr de vouloir supprimer « {} » ?").replace("{}", name),
        )
        .warning()
        .button(t("Delete", "Supprimer"), move |_, _, cx| {
            pane.update(cx, |pane, cx| pane.delete(&target, cx)).ok();
        })
        .close_button(t("Cancel", "Annuler"));
        self.with_app(cx, move |app, cx| app.alert(alert, cx));
    }

    // ---- Record Actions ------------------------------------------------------

    /// Start recording: from now on every change of the workbook is an
    /// action of the list. `recorder` was made by the window, which
    /// cannot be read from here while it runs a command.
    pub(crate) fn start_recording(&mut self, recorder: Recorder, window: &mut Window, cx: &mut Context<Self>) {
        if self.recorder.is_some() {
            self.page = Page::Recording;
            cx.notify();
            return;
        }
        self.leave_script(window, cx, {
            let recorder = RefCell::new(Some(recorder));
            move |pane, window, cx| {
                let Some(recorder) = recorder.borrow_mut().take() else {
                    return;
                };
                pane.recorder = Some(recorder);
                pane.record_watch = pane.app.upgrade().map(|app| cx.observe_in(&app, window, Self::workbook_watched));
                pane.page = Page::Recording;
                pane.trouble = None;
                cx.notify();
            }
        });
    }

    /// The window changed while recording: the recorder looks at what
    /// was done to the workbook.
    fn workbook_watched(&mut self, app: Entity<ExcelApp>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(recorder) = &mut self.recorder else {
            return;
        };
        let changed = {
            let app = app.read(cx);
            // A script that runs is not an action to record, nor what is
            // being typed in a cell.
            if run::is_running(app) {
                return;
            }
            let fonts = app.fonts.clone();
            let ts = window.text_system().clone();
            recorder.observe(app, &|wb, outcome| {
                run::refit(wb, outcome, &fonts, &ts);
                wb.commit("");
                wb.clear_history();
            })
        };
        if changed {
            cx.notify();
        }
    }

    /// Stop: the actions recorded become a script of the user's, saved
    /// under a name of its own and shown in the editor.
    pub(crate) fn stop_recording(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(recorder) = self.recorder.take() else {
            return;
        };
        self.record_watch = None;
        self.page = Page::List;
        let saved = scripts::create().and_then(|mut script| {
            script.body = recorder.script();
            scripts::save(&script).map(|()| script.name)
        });
        match saved {
            Ok(name) => self.open_in_editor(&name, window, cx),
            Err(error) => self.trouble = Some(error),
        }
        cx.notify();
    }

    /// "Copy as code": the statements of one action go to the clipboard.
    pub(crate) fn copy_as_code(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(action) = self.recorder.as_ref().and_then(|recorder| recorder.actions.get(index)) {
            cx.write_to_clipboard(gpui::ClipboardItem::new_string(action.code()));
        }
    }

    fn render_recording(&self, cx: &mut Context<Self>) -> AnyElement {
        let actions = self.recorder.as_ref().map(|recorder| recorder.actions.as_slice()).unwrap_or_default();
        let mut rows: Vec<AnyElement> = Vec::new();
        if actions.is_empty() {
            rows.push(
                div()
                    .px(px(12.))
                    .py(px(6.))
                    .text_color(c(theme::TEXT_MUTED))
                    .child(t(
                        "Make changes in the workbook: each action appears here as it is recorded.",
                        "Modifiez le classeur : chaque action apparaît ici à mesure qu'elle est enregistrée.",
                    ))
                    .into_any_element(),
            );
        }
        for (index, action) in actions.iter().enumerate() {
            let details: Vec<AnyElement> = if action.recorded {
                action.steps.iter().map(|step| div().truncate().child(step.comment.clone()).into_any_element()).collect()
            } else {
                vec![div().text_color(c(0xA4262C)).child(t("This action was not recorded.", "Cette action n'a pas été enregistrée.")).into_any_element()]
            };
            rows.push(
                div()
                    .id(("ce-rec-action", index))
                    .debug_selector(move || format!("ce-rec-action-{index}"))
                    .mx(px(6.))
                    .px(px(6.))
                    .py(px(5.))
                    .flex()
                    .items_start()
                    .gap(px(8.))
                    .rounded(px(4.))
                    .hover(|s| s.bg(c(theme::HOVER)))
                    .child(div().flex_none().w(px(18.)).text_color(c(theme::TEXT_MUTED)).child(format!("{}", index + 1)))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .child(div().truncate().child(action.label.clone()))
                            .child(div().text_size(px(11.)).text_color(c(theme::TEXT_MUTED)).children(details)),
                    )
                    .when(action.recorded, |d| {
                        d.child(
                            div()
                                .id(("ce-rec-copy", index))
                                .debug_selector(move || format!("ce-rec-copy-{index}"))
                                .flex_none()
                                .h(px(22.))
                                .px(px(5.))
                                .flex()
                                .items_center()
                                .rounded(px(4.))
                                .hover(|s| s.bg(c(theme::PRESSED)))
                                .child(icon("copy", 13.0, theme::TEXT))
                                .tooltip(tooltip(t("Copy as code", "Copier en tant que code")))
                                .on_click(cx.listener(move |pane, _, _, cx| pane.copy_as_code(index, cx))),
                        )
                    })
                    .into_any_element(),
            );
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .px(px(12.))
                    .py(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(div().flex_none().size(px(9.)).rounded(px(5.)).bg(c(0xD13438)))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .debug_selector(|| "ce-recording".to_string())
                            .child(format!("{}…", t("Recording", "Enregistrement"))),
                    )
                    .child(Self::tool("ce-rec-stop", "automate-stop", t("Stop", "Arrêter"), t("Stop Recording", "Arrêt de l'enregistrement"), true, cx, |pane, window, cx| {
                        pane.stop_recording(window, cx)
                    })),
            )
            .children(self.render_trouble())
            .child(Self::heading(t("Recorded actions", "Actions enregistrées")))
            .child(div().id("ce-rec-list").flex_1().min_h(px(0.)).overflow_y_scroll().pb(px(8.)).children(rows))
            .into_any_element()
    }

    // ---- drawing -----------------------------------------------------------

    /// A flat button of the pane: an icon, a label or both.
    fn tool(
        id: &'static str,
        icon_name: &'static str,
        label: &'static str,
        tip: &'static str,
        enabled: bool,
        cx: &mut Context<Self>,
        action: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        let color = if enabled { theme::TEXT } else { theme::TEXT_DISABLED };
        div()
            .id(id)
            .debug_selector(|| id.to_string())
            .h(px(24.))
            .px(px(6.))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(4.))
            .rounded(px(4.))
            .text_color(c(color))
            .when(!icon_name.is_empty(), |d| d.child(icon(icon_name, 14.0, color)))
            .when(!label.is_empty(), |d| d.child(div().whitespace_nowrap().child(label)))
            .when(enabled, |d| {
                d.hover(|s| s.bg(c(theme::HOVER)))
                    .active(|s| s.bg(c(theme::PRESSED)))
                    .on_click(cx.listener(move |pane, _, window, cx| action(pane, window, cx)))
            })
            .tooltip(tooltip(tip))
            .into_any_element()
    }

    fn heading(text: &'static str) -> gpui::Div {
        div()
            .px(px(12.))
            .pt(px(10.))
            .pb(px(4.))
            .text_size(px(11.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(c(theme::TEXT_MUTED))
            .child(text)
    }

    /// One script of a list: its name over its description.
    fn row(index: usize, script: ScriptRef, name: SharedString, description: SharedString, cx: &mut Context<Self>) -> AnyElement {
        let selector = match &script {
            ScriptRef::Mine(_) => format!("ce-script-{index}"),
            ScriptRef::Sample(_) => format!("ce-sample-{index}"),
        };
        let mine = matches!(script, ScriptRef::Mine(_));
        div()
            .id((if mine { "ce-script" } else { "ce-sample" }, index))
            .debug_selector(move || selector)
            .mx(px(6.))
            .px(px(6.))
            .py(px(5.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(4.))
            .hover(|s| s.bg(c(theme::HOVER)))
            .child(icon("automate-script", 16.0, theme::ACCENT_DARK))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .child(div().truncate().child(name))
                    .when(!description.is_empty(), |d| {
                        d.child(div().truncate().text_size(px(11.)).text_color(c(theme::TEXT_MUTED)).child(description))
                    }),
            )
            .on_click(cx.listener(move |pane, _, window, cx| pane.show_script(script.clone(), window, cx)))
            .into_any_element()
    }

    fn render_trouble(&self) -> Option<AnyElement> {
        self.trouble.as_ref().map(|trouble| {
            div()
                .mx(px(12.))
                .my(px(4.))
                .px(px(8.))
                .py(px(5.))
                .rounded(px(4.))
                .bg(c(0xFDE7E9))
                .text_color(c(0xA4262C))
                .child(trouble.clone())
                .into_any_element()
        })
    }

    fn render_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let mine = scripts::list();
        let mut rows: Vec<AnyElement> = Vec::new();
        rows.push(Self::heading(t("My Scripts", "Mes scripts")).into_any_element());
        if mine.is_empty() {
            rows.push(
                div()
                    .px(px(12.))
                    .py(px(4.))
                    .text_color(c(theme::TEXT_MUTED))
                    .child(t("You have no scripts yet. Select New Script to write one.", "Vous n'avez pas encore de script. Sélectionnez Nouveau script pour en écrire un."))
                    .into_any_element(),
            );
        }
        for (index, entry) in mine.into_iter().enumerate() {
            rows.push(Self::row(index, ScriptRef::Mine(entry.name.clone()), entry.name.into(), entry.description.into(), cx));
        }
        rows.push(Self::heading(t("Samples", "Exemples")).into_any_element());
        for (index, sample) in SAMPLES.iter().enumerate() {
            rows.push(Self::row(index, ScriptRef::Sample(index), sample.name().into(), sample.description().into(), cx));
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .child(Self::tool("ce-new", "plus", t("New Script", "Nouveau script"), t("New Script", "Nouveau script"), true, cx, |pane, window, cx| {
                        pane.new_script(window, cx)
                    }))
                    .child(Self::tool(
                        "ce-import",
                        "folder",
                        t("Import…", "Importer…"),
                        t("Add a script from an .osts file", "Ajouter un script à partir d'un fichier .osts"),
                        true,
                        cx,
                        |pane, _, cx| pane.ask_import(cx),
                    )),
            )
            .children(self.render_trouble())
            .child(div().id("ce-list").flex_1().min_h(px(0.)).overflow_y_scroll().pb(px(8.)).children(rows))
            .child(
                div()
                    .flex_none()
                    .px(px(12.))
                    .py(px(5.))
                    .border_t_1()
                    .border_color(c(theme::BORDER))
                    .text_size(px(10.5))
                    .text_color(c(theme::TEXT_MUTED))
                    .child(div().truncate().child(scripts::folder().to_string_lossy().to_string())),
            )
            .into_any_element()
    }

    /// Run, or what replaces it while a script runs.
    fn run_controls(&self, running: Option<&str>, cx: &mut Context<Self>) -> Vec<AnyElement> {
        match running {
            Some(name) => vec![
                div()
                    .debug_selector(|| "ce-running".to_string())
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .text_color(c(theme::ACCENT_DARK))
                    .child(icon("automate-play", 12.0, theme::ACCENT_DARK))
                    .child(div().truncate().child(format!("{} {name}…", t("Running", "Exécution de"))))
                    .into_any_element(),
                Self::tool("ce-stop", "close", t("Stop", "Arrêter"), t("Stop the script", "Arrêter le script"), true, cx, |pane, _, cx| pane.stop(cx)),
            ],
            None => vec![Self::tool("ce-run", "automate-play", t("Run", "Exécuter"), t("Run Script", "Exécuter le script"), true, cx, |pane, window, cx| {
                pane.run(window, cx)
            })],
        }
    }

    fn render_script(&self, script: &ScriptRef, running: Option<&str>, output: Vec<OutputLine>, cx: &mut Context<Self>) -> AnyElement {
        let (name, description) = match self.script_of(script) {
            Ok(found) => (found.name, found.description),
            Err(_) => (String::new(), String::new()),
        };
        let menu_script = script.clone();
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_none().px(px(8.)).flex().child(Self::tool(
                "ce-back",
                "chevron-left",
                t("All Scripts", "Tous les scripts"),
                t("All Scripts", "Tous les scripts"),
                true,
                cx,
                |pane, window, cx| pane.show_list(window, cx),
            )))
            .child(
                div()
                    .flex_none()
                    .px(px(14.))
                    .pt(px(8.))
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .debug_selector(|| "ce-script-name".to_string())
                    .child(name),
            )
            .when(!description.is_empty(), |d| d.child(div().flex_none().px(px(14.)).pt(px(3.)).text_color(c(theme::TEXT_MUTED)).child(description)))
            .child(
                div()
                    .flex_none()
                    .px(px(8.))
                    .py(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .children(self.run_controls(running, cx))
                    .child(Self::tool("ce-edit", "ink-pencil", t("Edit", "Modifier"), t("Edit Code", "Modifier le code"), true, cx, |pane, window, cx| {
                        pane.edit(window, cx)
                    }))
                    .child(Self::tool("ce-more", "more", "", t("More options", "Autres options"), true, cx, move |pane, window, cx| {
                        pane.open_menu(menu_script.clone(), window, cx)
                    })),
            )
            .children(self.render_trouble())
            .child(Self::heading(t("Output", "Sortie")))
            .child(div().id("ce-page-output").flex_1().min_h(px(0.)).overflow_y_scroll().children(Self::output_rows(&output, false, cx)))
            .into_any_element()
    }

    /// The lines of the Output list. In the editor a line that names a
    /// line of the script takes the caret there.
    fn output_rows(output: &[OutputLine], clickable: bool, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if output.is_empty() {
            return vec![
                div()
                    .px(px(12.))
                    .py(px(4.))
                    .text_color(c(theme::TEXT_MUTED))
                    .child(t("Run the script to see its output here.", "Exécutez le script pour afficher sa sortie ici."))
                    .into_any_element(),
            ];
        }
        output
            .iter()
            .enumerate()
            .map(|(index, entry)| {
                let (mark, color) = match entry.kind {
                    OutputKind::Log(LogLevel::Log) => ("", theme::TEXT),
                    OutputKind::Log(LogLevel::Warn) => ("⚠", 0x8A6100),
                    OutputKind::Log(LogLevel::Error) | OutputKind::Error => ("✕", 0xA4262C),
                    OutputKind::Note => ("", theme::TEXT_MUTED),
                };
                let place = entry.line.map(|line| crate::i18n::labelled(t("Line", "Ligne"), line).replace(": ", " ").replace(" : ", " "));
                let (line, column) = (entry.line, entry.column);
                div()
                    .id(("ce-output", index))
                    .debug_selector(move || format!("ce-output-{index}"))
                    .px(px(12.))
                    .py(px(2.))
                    .flex()
                    .items_start()
                    .gap(px(6.))
                    .text_color(c(color))
                    .when(!mark.is_empty(), |d| d.child(div().flex_none().child(mark)))
                    .child(div().flex_1().min_w(px(0.)).child(entry.text.clone()))
                    .children(place.map(|place| div().flex_none().text_size(px(11.)).text_color(c(theme::TEXT_MUTED)).child(place)))
                    .when(clickable && line.is_some(), |d| {
                        d.hover(|s| s.bg(c(theme::HOVER))).on_click(cx.listener(move |pane, _, window, cx| {
                            if let Some(line) = line {
                                pane.go_to(line, column, window, cx);
                            }
                        }))
                    })
                    .into_any_element()
            })
            .collect()
    }

    /// Put the caret on a line of the script and give the editor the keys.
    pub(crate) fn go_to(&mut self, line: u32, column: Option<u32>, window: &mut Window, cx: &mut Context<Self>) {
        self.editor.update(cx, |editor, cx| editor.go_to_line(line, column, cx));
        window.focus(&self.editor.focus_handle(cx));
    }

    fn problem_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if self.problems.is_empty() {
            return vec![
                div()
                    .px(px(12.))
                    .py(px(4.))
                    .text_color(c(theme::TEXT_MUTED))
                    .child(t("No problems have been detected.", "Aucun problème n'a été détecté."))
                    .into_any_element(),
            ];
        }
        self.problems
            .iter()
            .enumerate()
            .map(|(index, problem)| {
                let (line, column) = (problem.line, problem.column);
                let place = match (line, column) {
                    (Some(line), Some(column)) => format!("[{line}, {column}]"),
                    (Some(line), None) => format!("[{line}]"),
                    _ => String::new(),
                };
                div()
                    .id(("ce-problem", index))
                    .debug_selector(move || format!("ce-problem-{index}"))
                    .px(px(12.))
                    .py(px(2.))
                    .flex()
                    .items_start()
                    .gap(px(6.))
                    .hover(|s| s.bg(c(theme::HOVER)))
                    .child(div().flex_none().text_color(c(0xA4262C)).child("✕"))
                    .child(div().flex_1().min_w(px(0.)).child(problem.message.clone()))
                    .child(div().flex_none().text_size(px(11.)).text_color(c(theme::TEXT_MUTED)).child(place))
                    .on_click(cx.listener(move |pane, _, window, cx| {
                        if let Some(line) = line {
                            pane.go_to(line, column, window, cx);
                        }
                    }))
                    .into_any_element()
            })
            .collect()
    }

    fn render_editor(&self, running: Option<&str>, output: Vec<OutputLine>, cx: &mut Context<Self>) -> AnyElement {
        let name = self.open.clone().unwrap_or_default();
        let unsaved = self.unsaved(cx);
        let (line, column) = self.editor.read(cx).caret_position();
        let menu_script = ScriptRef::Mine(name.clone());
        let tab = |id: &'static str, label: String, which: Bottom, cx: &mut Context<Self>| {
            let on = self.bottom == which;
            div()
                .id(id)
                .debug_selector(|| id.to_string())
                .h(px(24.))
                .px(px(8.))
                .flex()
                .items_center()
                .border_b_2()
                .border_color(if on { c(theme::ACCENT) } else { gpui::transparent_black() })
                .when(on, |d| d.font_weight(FontWeight::SEMIBOLD))
                .when(!on, |d| d.text_color(c(theme::TEXT_MUTED)).hover(|s| s.bg(c(theme::HOVER))))
                .child(label)
                .on_click(cx.listener(move |pane, _, _, cx| {
                    pane.bottom = which;
                    cx.notify();
                }))
        };
        let count = |label: &'static str, n: usize| if n == 0 { label.to_string() } else { format!("{label} ({n})") };
        let rows = match self.bottom {
            Bottom::Output => Self::output_rows(&output, true, cx),
            Bottom::Problems => self.problem_rows(cx),
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .child(Self::tool("ce-back", "chevron-left", "", t("All Scripts", "Tous les scripts"), true, cx, |pane, window, cx| pane.show_list(window, cx)))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .debug_selector(|| "ce-editor-name".to_string())
                            // The mark of unsaved changes.
                            .child(if unsaved { format!("{name} *") } else { name }),
                    )
                    .children(self.run_controls(running, cx))
                    .child(Self::tool("ce-save", "save", "", t("Save Script", "Enregistrer le script"), unsaved, cx, |pane, _, cx| pane.save(cx)))
                    .child(Self::tool("ce-more", "more", "", t("More options", "Autres options"), true, cx, move |pane, window, cx| {
                        pane.open_menu(menu_script.clone(), window, cx)
                    })),
            )
            .children(self.render_trouble())
            .child(
                div()
                    .flex_1()
                    .min_h(px(60.))
                    .border_t_1()
                    .border_b_1()
                    .border_color(c(theme::BORDER))
                    .debug_selector(|| "ce-editor".to_string())
                    .child(self.editor.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .px(px(4.))
                    .flex()
                    .items_center()
                    .child(tab("ce-tab-output", count(t("Output", "Sortie"), output.len()), Bottom::Output, cx))
                    .child(tab("ce-tab-problems", count(t("Problems", "Problèmes"), self.problems.len()), Bottom::Problems, cx))
                    .child(div().flex_1())
                    .child(
                        div()
                            .px(px(8.))
                            .text_size(px(11.))
                            .text_color(c(theme::TEXT_MUTED))
                            .child(format!("{} {line}, {} {column}", t("Ln", "Li"), t("Col", "Col"))),
                    ),
            )
            .child(div().id("ce-bottom").flex_none().h(px(120.)).overflow_y_scroll().py(px(3.)).children(rows))
            .into_any_element()
    }
}

impl Render for CodeEditorPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (running, output, output_of) = match self.app.upgrade() {
            Some(app) => {
                let app = app.read(cx);
                match app.ext::<Runs>() {
                    Some(runs) => (runs.running.as_ref().map(|running| running.name.clone()), runs.output.clone(), runs.output_of.clone()),
                    None => (None, Vec::new(), String::new()),
                }
            }
            None => (None, Vec::new(), String::new()),
        };
        // The output shown is that of the script shown.
        let shown = match self.current() {
            Some(ScriptRef::Mine(name)) => name,
            Some(ScriptRef::Sample(index)) => SAMPLES[index].name().to_string(),
            None => String::new(),
        };
        let output = if shown == output_of { output } else { Vec::new() };
        let content = match self.page.clone() {
            Page::List => self.render_list(cx),
            Page::Script(script) => self.render_script(&script, running.as_deref(), output, cx),
            Page::Editor => self.render_editor(running.as_deref(), output, cx),
            Page::Recording => self.render_recording(cx),
        };
        div().size_full().child(content)
    }
}

// ------------------------------------------------------------------ rename

/// The dialog that gives a script another name.
pub(crate) struct RenameDialog {
    modal: Modal,
    pane: WeakEntity<CodeEditorPane>,
    old: String,
    pub(crate) name: Entity<TextInput>,
    /// Why the name typed cannot be taken.
    pub(crate) problem: Option<String>,
}

impl RenameDialog {
    fn new(modal: Modal, pane: WeakEntity<CodeEditorPane>, old: String, cx: &mut Context<Self>) -> Self {
        let name = widgets::input(cx, old.clone());
        name.update(cx, |input, cx| input.select_all_text(cx));
        RenameDialog {
            modal,
            pane,
            old,
            name,
            problem: None,
        }
    }

    pub(crate) fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let new = self.name.read(cx).text().to_string();
        if let Some(problem) = scripts::name_problem(&new, Some(&self.old)) {
            self.problem = Some(problem.to_string());
            cx.notify();
            return;
        }
        let old = self.old.clone();
        let renamed = self.pane.update(cx, |pane, cx| pane.rename(&old, &new, cx)).unwrap_or(false);
        if renamed {
            self.modal.close(window, cx);
        } else {
            self.problem = self.pane.update(cx, |pane, _| pane.trouble.take()).ok().flatten();
            cx.notify();
        }
    }
}

impl Render for RenameDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        widgets::dialog(&self.modal, t("Rename", "Renommer"))
            .width(340.0)
            .on_ok(cx.listener(Self::ok))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(widgets::label(t("Script name", "Nom du script")))
                    .child(div().flex().child(widgets::TextField::new(&self.name)))
                    .children(self.problem.clone().map(|problem| div().text_color(c(0xA4262C)).child(problem))),
            )
            .footer(
                div()
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(Button::new("rename-cancel", t("Cancel", "Annuler")).on_click({
                        let modal = self.modal.clone();
                        move |_, window, cx| modal.close(window, cx)
                    }))
                    .child(Button::new("rename-ok", t("Rename", "Renommer")).primary().on_click(cx.listener(Self::ok))),
            )
    }
}
