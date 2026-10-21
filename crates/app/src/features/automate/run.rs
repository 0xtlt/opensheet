//! Running a script on the workbook of the window.
//!
//! The engine (`opensheet_script`) is reached through [`Engine`], so that
//! the interface can be driven in tests with an engine that does what the
//! test says.
//!
//! A script may run for a long time (the engine stops it at its time
//! limit, two minutes by default). So that the window never waits for it,
//! the script runs in another thread on a copy of the workbook
//! (`Workbook::fork`). When it ends well the workbook takes over what the
//! copy has become, as one undo step; when it fails, or is stopped, the
//! copy is dropped and nothing has changed. What the user changes in the
//! workbook while a script runs wins: the result of the script is then
//! refused rather than laid over it.

use crate::app::ExcelApp;
use crate::features::Cx;
use crate::i18n::t;
use gpui::Window;
use opensheet_engine::protect::Action;
use opensheet_engine::{Rect, Workbook};
use opensheet_script::{Context, ErrorKind, LogLevel, Outcome, ScriptError};
use std::cell::RefCell;
use std::sync::Arc;

/// What runs and checks scripts.
pub(crate) trait Engine: Send + Sync {
    /// What is wrong with the text of a script, without running it.
    fn check(&self, source: &str) -> Vec<ScriptError>;
    /// Run a script on a workbook whose snapshot the caller has taken.
    fn run(&self, source: &str, wb: &mut Workbook, context: &Context) -> Outcome;
}

/// The engine of the application.
struct OfficeScripts;

impl Engine for OfficeScripts {
    fn check(&self, source: &str) -> Vec<ScriptError> {
        opensheet_script::check(source)
    }

    fn run(&self, source: &str, wb: &mut Workbook, context: &Context) -> Outcome {
        opensheet_script::run(source, wb, context)
    }
}

thread_local! {
    static ENGINE: RefCell<Arc<dyn Engine>> = RefCell::new(Arc::new(OfficeScripts));
}

pub(crate) fn engine() -> Arc<dyn Engine> {
    ENGINE.with(|engine| engine.borrow().clone())
}

/// Use another engine on this thread: each test brings its own.
#[cfg(test)]
pub(crate) fn use_engine(engine: Arc<dyn Engine>) {
    ENGINE.with(|slot| *slot.borrow_mut() = engine);
}

/// What a line of the Output list is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutputKind {
    Log(LogLevel),
    /// Why the script stopped.
    Error,
    /// What the application says about the run.
    Note,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OutputLine {
    pub kind: OutputKind,
    pub text: String,
    /// The line of the script it is about (1-based).
    pub line: Option<u32>,
    pub column: Option<u32>,
}

impl OutputLine {
    fn note(text: impl Into<String>) -> OutputLine {
        OutputLine {
            kind: OutputKind::Note,
            text: text.into(),
            line: None,
            column: None,
        }
    }

    fn error(text: impl Into<String>, line: Option<u32>, column: Option<u32>) -> OutputLine {
        OutputLine {
            kind: OutputKind::Error,
            text: text.into(),
            line,
            column,
        }
    }
}

/// A script that is running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Running {
    id: u64,
    pub name: String,
    /// `Workbook::generation` when the copy was made.
    generation: u64,
}

/// What the window knows about the scripts run on its workbook.
#[derive(Default)]
pub(crate) struct Runs {
    next_id: u64,
    pub running: Option<Running>,
    /// The Output list: what the last run logged and how it ended.
    pub output: Vec<OutputLine>,
    /// The script the output is about.
    pub output_of: String,
    /// Where the last run failed (line, column), for the editor to go to.
    pub failed_at: Option<(u32, Option<u32>)>,
    /// Increased at the end of every run, for the pane to notice.
    pub finished: u64,
}

pub(crate) fn is_running(app: &ExcelApp) -> bool {
    app.ext::<Runs>().is_some_and(|runs| runs.running.is_some())
}

/// What the script must not have done to a protected sheet or to a
/// workbook whose structure is protected, comparing the workbook with the
/// copy the script worked on. `None` when it kept to what is allowed.
fn protection_refusal(before: &Workbook, after: &Workbook) -> Option<&'static str> {
    let structure = t(
        "The script tried to change the structure of a protected workbook. Nothing was changed.",
        "Le script a tenté de modifier la structure d'un classeur protégé. Rien n'a été modifié.",
    );
    let protected = t(
        "The script tried to change a protected sheet. Nothing was changed.",
        "Le script a tenté de modifier une feuille protégée. Rien n'a été modifié.",
    );
    let same_sheets = before.sheets.len() == after.sheets.len()
        && before
            .sheets
            .iter()
            .zip(&after.sheets)
            .all(|(a, b)| a.name == b.name);
    if !same_sheets {
        if before.permits(&Action::InsertSheet).is_err() {
            return Some(structure);
        }
        // Sheets came, went or changed place: a protected sheet must
        // still be there, under its name, as it was.
        for sheet in before
            .sheets
            .iter()
            .filter(|sheet| sheet.protection.is_some())
        {
            if !after.sheets.iter().any(|other| other.name == sheet.name) {
                return Some(protected);
            }
        }
    }
    for (index, sheet) in before.sheets.iter().enumerate() {
        if sheet.protection.is_none() {
            continue;
        }
        let Some(other) = after.sheets.iter().find(|other| other.name == sheet.name) else {
            continue;
        };
        // The cells whose content changed, and whether a format did.
        let mut edited: Vec<Rect> = Vec::new();
        let mut formatted = false;
        let positions = sheet.cells.keys().chain(
            other
                .cells
                .keys()
                .filter(|pos| !sheet.cells.contains_key(pos)),
        );
        for pos in positions {
            let (old, new) = (sheet.cells.get(pos), other.cells.get(pos));
            if old.map(|cell| &cell.content) != new.map(|cell| &cell.content) {
                edited.push(Rect::cell(pos.0, pos.1));
            }
            if old.map(|cell| cell.style) != new.map(|cell| cell.style) {
                formatted = true;
            }
        }
        formatted |= sheet.col_styles != other.col_styles
            || sheet.row_styles != other.row_styles
            || sheet.merges != other.merges;
        let columns =
            sheet.col_widths != other.col_widths || sheet.hidden_cols != other.hidden_cols;
        let rows = sheet.row_heights != other.row_heights || sheet.hidden_rows != other.hidden_rows;
        let refused = (!edited.is_empty()
            && before
                .permits(&Action::EditCells {
                    sheet: index,
                    ranges: &edited,
                })
                .is_err())
            || (formatted
                && before
                    .permits(&Action::FormatCells { sheet: index })
                    .is_err())
            || (columns
                && before
                    .permits(&Action::FormatColumns { sheet: index })
                    .is_err())
            || (rows
                && before
                    .permits(&Action::FormatRows { sheet: index })
                    .is_err())
            || (sheet.tables.len() != other.tables.len()
                && before
                    .permits(&Action::ChangeTable { sheet: index })
                    .is_err());
        if refused {
            return Some(protected);
        }
    }
    None
}

/// The text of an error of a script, as the Output list shows it.
fn error_text(error: &ScriptError) -> String {
    match error.kind {
        ErrorKind::Timeout => t(
            "The script ran for too long and was stopped.",
            "Le script s'est exécuté trop longtemps et a été arrêté.",
        )
        .to_string(),
        _ => error.message.clone(),
    }
}

/// Run a script on the workbook of the window. False when it did not
/// start: another script is running, or the entry being typed in a cell
/// was refused.
pub(crate) fn start(
    app: &mut ExcelApp,
    name: &str,
    source: &str,
    window: &mut Window,
    cx: &mut Cx,
) -> bool {
    if is_running(app) {
        return false;
    }
    // What is being typed in a cell is stored first, as before any command.
    if app.is_editing() && !app.commit_edit(None, window, cx) {
        return false;
    }
    let selection = app.sel().clone();
    let context = Context {
        active_sheet: app.sheet,
        selection: selection
            .ranges
            .last()
            .copied()
            .unwrap_or(Rect::cell(selection.active.0, selection.active.1)),
        active_cell: selection.active,
        workbook_name: app
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .map_or_else(
                || app.title.clone(),
                |name| name.to_string_lossy().into_owned(),
            ),
        // The author of the comments and notes the script writes.
        user_name: crate::features::preferences::user_name()
            .unwrap_or_else(|| Context::default().user_name),
        ..Context::default()
    };
    let mut fork = app.wb.fork();
    let engine = engine();
    let source = source.to_string();
    let generation = app.wb.generation;
    let runs = app.ext_mut::<Runs>();
    runs.next_id += 1;
    let id = runs.next_id;
    runs.running = Some(Running {
        id,
        name: name.to_string(),
        generation,
    });
    runs.output.clear();
    runs.output_of = name.to_string();
    runs.failed_at = None;
    cx.notify();

    let task = cx.background_executor().spawn(async move {
        // The contract of the engine: a snapshot before, then its changes
        // are kept or dropped as a whole. Here the whole copy is.
        fork.snapshot();
        let outcome = engine.run(&source, &mut fork, &context);
        (fork, outcome)
    });
    cx.spawn_in(window, async move |app, cx| {
        let (fork, outcome) = task.await;
        app.update_in(cx, |app, window, cx| {
            finish(app, id, fork, outcome, window, cx)
        })
        .ok();
    })
    .detach();
    true
}

/// Stop waiting for the script that runs: whatever it does from now on
/// is dropped. (The engine itself stops at its time limit.)
pub(crate) fn stop(app: &mut ExcelApp, cx: &mut Cx) {
    let runs = app.ext_mut::<Runs>();
    if runs.running.take().is_some() {
        runs.output.push(OutputLine::note(t(
            "The script was stopped. Nothing was changed.",
            "Le script a été arrêté. Rien n'a été modifié.",
        )));
        runs.finished += 1;
        cx.notify();
    }
}

/// The columns the script asked to fit were sized by the engine without
/// fonts: fit them again with the ones the window draws with.
pub(crate) fn refit(
    wb: &mut Workbook,
    outcome: &Outcome,
    fonts: &crate::grid::FontResolver,
    ts: &gpui::WindowTextSystem,
) {
    for fit in outcome.autofit.iter().filter(|fit| fit.columns) {
        let Some(sheet) = wb.sheets.get(fit.sheet) else {
            continue;
        };
        let columns: std::collections::BTreeSet<u32> = sheet
            .cells
            .keys()
            .map(|(_, column)| *column)
            .filter(|column| (fit.first..=fit.last).contains(column))
            .collect();
        for column in columns {
            if let Some(width) = crate::grid::autofit_col_width(wb, fit.sheet, column, fonts, ts) {
                wb.set_col_width(fit.sheet, column, Some(width));
            }
        }
    }
}

/// The script ended: show what it logged, and take over its changes or
/// say why not.
fn finish(
    app: &mut ExcelApp,
    id: u64,
    fork: Workbook,
    outcome: Outcome,
    window: &mut Window,
    cx: &mut Cx,
) {
    // Stopped meanwhile, or the window shows another workbook now.
    let Some(running) = app
        .ext::<Runs>()
        .and_then(|runs| runs.running.clone())
        .filter(|running| running.id == id)
    else {
        return;
    };
    let mut output: Vec<OutputLine> = outcome
        .logs
        .iter()
        .map(|log| OutputLine {
            kind: OutputKind::Log(log.level),
            text: log.text.clone(),
            line: log.line,
            column: None,
        })
        .collect();
    let mut failed_at = None;
    let refusal = match &outcome.error {
        Some(error) => {
            failed_at = error.line.map(|line| (line, error.column));
            Some(OutputLine::error(
                error_text(error),
                error.line,
                error.column,
            ))
        }
        // The workbook is no longer the one the script worked on.
        None if app.wb.generation != running.generation => Some(OutputLine::error(
            t(
                "The workbook changed while the script was running. The changes of the script were not applied.",
                "Le classeur a été modifié pendant l'exécution du script. Les modifications du script n'ont pas été appliquées.",
            ),
            None,
            None,
        )),
        // A script that changed the protection itself was allowed each
        // of its steps by the engine: the workbook as it was protected
        // before says nothing of what it may have done since.
        None if outcome.protection_changed => None,
        None => {
            protection_refusal(&app.wb, &fork).map(|message| OutputLine::error(message, None, None))
        }
    };
    match refusal {
        Some(line) => output.push(line),
        None => {
            app.wb.adopt(fork);
            let fonts = app.fonts.clone();
            refit(&mut app.wb, &outcome, &fonts, window.text_system());
            app.commit(t("Run Script", "Exécuter le script"), cx);
            // The window follows what the script put in front.
            if let Some(sheet) = outcome
                .activated
                .filter(|sheet| *sheet < app.wb.sheets.len() && *sheet != app.sheet)
            {
                app.switch_sheet(sheet, window, cx);
            }
            if let Some((sheet, rect)) = outcome
                .selected
                .filter(|(sheet, _)| *sheet < app.wb.sheets.len())
            {
                if sheet != app.sheet {
                    app.switch_sheet(sheet, window, cx);
                }
                app.select_rect(rect, cx);
            }
            if let Some(value) = &outcome.returned {
                output.push(OutputLine::note(crate::i18n::labelled(
                    t("Returned value", "Valeur renvoyée"),
                    value,
                )));
            }
            output.push(OutputLine::note(t(
                "The script ran successfully.",
                "Le script a été exécuté.",
            )));
            // Protecting a sheet forgets the history, by hand or by script.
            if outcome.protection_changed {
                output.push(OutputLine::note(t(
                    "The script changed the protection of a sheet: its changes cannot be undone.",
                    "Le script a modifié la protection d'une feuille : ses modifications ne peuvent pas être annulées.",
                )));
            }
        }
    }
    let runs = app.ext_mut::<Runs>();
    runs.running = None;
    runs.output = output;
    runs.failed_at = failed_at;
    runs.finished += 1;
    cx.notify();
}

#[cfg(test)]
pub(crate) mod testing {
    //! An engine for the tests of the interface: its scripts are a few
    //! commands, one per line, enough to see what the window does with
    //! each outcome.

    use super::*;
    use opensheet_script::Log;

    /// `set A1 text` types into a cell of the active sheet, `bold A1`
    /// makes one bold, `log text` logs, `add-sheet` adds a sheet and puts
    /// it in front, `select B2:C3` selects, `return 42` returns, `throw
    /// text` fails at its line, `hide-column 0` hides a column. A line
    /// holding `syntax error` is one for `check`, a line holding `unknown
    /// member` one it only warns about.
    pub(crate) struct FakeEngine;

    fn rect(text: &str) -> Rect {
        let cell = |name: &str| opensheet_engine::refs::parse_cell_name(name).expect("a cell name");
        match text.split_once(':') {
            Some((a, b)) => {
                let (a, b) = (cell(a), cell(b));
                Rect::new(a.0, a.1, b.0, b.1)
            }
            None => {
                let a = cell(text);
                Rect::cell(a.0, a.1)
            }
        }
    }

    impl Engine for FakeEngine {
        fn check(&self, source: &str) -> Vec<ScriptError> {
            source
                .lines()
                .enumerate()
                .filter(|(_, line)| {
                    line.contains("syntax error") || line.contains("unknown member")
                })
                .map(|(index, line)| ScriptError {
                    kind: if line.contains("syntax error") {
                        ErrorKind::Syntax
                    } else {
                        ErrorKind::Check
                    },
                    message: format!("Unexpected token in {:?}", line.trim()),
                    line: Some(index as u32 + 1),
                    column: Some(1),
                })
                .collect()
        }

        fn run(&self, source: &str, wb: &mut Workbook, context: &Context) -> Outcome {
            let mut outcome = Outcome::default();
            let mut sheet = context.active_sheet;
            for (index, line) in source.lines().enumerate() {
                let number = Some(index as u32 + 1);
                let (command, rest) = line.trim().split_once(' ').unwrap_or((line.trim(), ""));
                match command {
                    "set" => {
                        let (cell, text) = rest.split_once(' ').unwrap_or((rest, ""));
                        let at = rect(cell);
                        wb.set_cell_text(sheet, at.r0, at.c0, text)
                            .expect("a value");
                    }
                    "bold" => wb.update_style(sheet, rect(rest), &|style| style.font.bold = true),
                    "hide-column" => {
                        wb.snapshot();
                        wb.sheets[sheet]
                            .hidden_cols
                            .insert(rest.parse().expect("a column"));
                    }
                    "log" => outcome.logs.push(Log {
                        level: LogLevel::Log,
                        text: rest.to_string(),
                        line: number,
                    }),
                    "warn" => outcome.logs.push(Log {
                        level: LogLevel::Warn,
                        text: rest.to_string(),
                        line: number,
                    }),
                    "add-sheet" => {
                        sheet = wb.add_sheet(wb.sheets.len());
                        outcome.activated = Some(sheet);
                    }
                    "select" => outcome.selected = Some((sheet, rect(rest))),
                    "return" => outcome.returned = Some(rest.to_string()),
                    "throw" => {
                        outcome.error = Some(ScriptError {
                            kind: ErrorKind::Runtime,
                            message: rest.to_string(),
                            line: number,
                            column: Some(3),
                        });
                        break;
                    }
                    "timeout" => {
                        outcome.error = Some(ScriptError {
                            kind: ErrorKind::Timeout,
                            message: "timeout".to_string(),
                            line: None,
                            column: None,
                        });
                        break;
                    }
                    _ => {}
                }
            }
            outcome
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::Locale;
    use opensheet_engine::protect::SheetProtection;

    fn workbook() -> Workbook {
        let mut wb = Workbook::new(Locale::en());
        wb.set_cell_text(0, 0, 0, "1").unwrap();
        wb.commit("setup");
        wb
    }

    #[test]
    fn a_protected_sheet_refuses_what_a_script_did_to_it() {
        let mut wb = workbook();
        // Unprotected: anything goes.
        let mut fork = wb.fork();
        fork.set_cell_text(0, 0, 0, "2").unwrap();
        assert_eq!(protection_refusal(&wb, &fork), None);

        wb.snapshot();
        wb.sheets[0].protection = Some(SheetProtection::default());
        wb.commit("protect");
        let unchanged = wb.fork();
        assert_eq!(protection_refusal(&wb, &unchanged), None);
        // A locked cell changed.
        let mut fork = wb.fork();
        fork.set_cell_text(0, 0, 0, "2").unwrap();
        assert!(protection_refusal(&wb, &fork).is_some());
        // A new value in an empty locked cell, a format, a hidden column.
        let mut fork = wb.fork();
        fork.set_cell_text(0, 5, 5, "x").unwrap();
        assert!(protection_refusal(&wb, &fork).is_some());
        let mut fork = wb.fork();
        fork.update_style(0, Rect::cell(0, 0), &|style| style.font.bold = true);
        assert!(protection_refusal(&wb, &fork).is_some());
        let mut fork = wb.fork();
        fork.sheets[0].hidden_cols.insert(2);
        assert!(protection_refusal(&wb, &fork).is_some());
        // Another sheet of the workbook is free.
        let mut fork = wb.fork();
        let added = fork.add_sheet(1);
        fork.set_cell_text(added, 0, 0, "free").unwrap();
        assert_eq!(protection_refusal(&wb, &fork), None);
        // The protected sheet cannot be deleted.
        let mut fork = wb.fork();
        fork.add_sheet(1);
        fork.delete_sheet(0);
        assert!(protection_refusal(&wb, &fork).is_some());
    }

    #[test]
    fn a_protected_structure_refuses_new_sheets() {
        let mut wb = workbook();
        wb.protection.lock_structure = true;
        let mut fork = wb.fork();
        fork.set_cell_text(0, 0, 0, "2").unwrap();
        assert_eq!(protection_refusal(&wb, &fork), None);
        fork.add_sheet(1);
        assert!(protection_refusal(&wb, &fork).is_some());
        let mut fork = wb.fork();
        fork.rename_sheet(0, "Other");
        assert!(protection_refusal(&wb, &fork).is_some());
    }
}
