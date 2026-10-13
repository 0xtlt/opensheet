//! Record Actions: what the user does in the window becomes a script.
//!
//! The recorder does not listen to commands. It keeps a copy of the
//! workbook as it was after the last action and, each time the window
//! says the workbook changed (a step was committed, undone or redone),
//! looks at what the action did:
//!
//! 1. where the action is one Excel's recorder writes in one statement
//!    and that comparing two workbooks would not show as such — rows,
//!    columns or cells inserted and deleted, a fill, a paste, a sort, a
//!    clear, an autofit, a sheet copied — the statement is guessed from
//!    where the selection was (`candidates.rs`) and kept when running it
//!    on the copy really gives the workbook the action gave, or comes
//!    closer to it than doing nothing;
//! 2. what is left is written from the comparison of the two workbooks
//!    (`diff.rs`);
//! 3. the statements are run on the copy, through the script engine, and
//!    the copy compared with the workbook (`compare.rs`). An action whose
//!    script does not give the very same workbook is listed as not
//!    recorded, and has no statements.
//!
//! So nothing depends on which command, dialog, key or mouse gesture made
//! a change, a new command is recorded without anyone thinking of it, and
//! a statement the engine would refuse, or that would do something else,
//! never reaches a script.

mod candidates;
mod code;
mod compare;
pub(crate) mod diff;
#[cfg(test)]
mod tests;

pub(crate) use code::Step;
#[cfg(test)]
pub(crate) use compare::differences;

use crate::app::ExcelApp;
use code::Scope;
use opensheet_engine::{Rect, Workbook};

/// One action of the user, as the pane lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Recorded {
    /// What the action is called: the name of its undo step.
    pub label: String,
    /// What it does in the script; empty when it could not be written.
    pub steps: Vec<Step>,
    pub recorded: bool,
    /// How many steps could be undone once it was done.
    depth: usize,
    /// How sheets read in the script before it.
    scope_before: Scope,
}

impl Recorded {
    /// The action as it reads in a script: its comments and statements.
    pub fn code(&self) -> String {
        self.steps.iter().map(|step| code::text_of(step, "")).collect()
    }
}

/// What the window showed before an action: where the statement of the
/// action is guessed from.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Before {
    pub sheet: usize,
    pub selection: Rect,
    pub active: (u32, u32),
    /// The cells waiting on the clipboard: sheet, range, cut rather than
    /// copied.
    pub clip: Option<(usize, Rect, bool)>,
}

impl Before {
    fn of(app: &ExcelApp) -> Before {
        let selection = app.sel();
        Before {
            sheet: app.sheet,
            selection: selection.current(),
            active: selection.active,
            clip: app.clip.as_ref().map(|state| (state.clip.sheet, state.clip.rect, state.cut)),
        }
    }
}

/// What tells that the workbook changed: its generation and the depths
/// of its undo and redo stacks.
type Mark = (u64, usize, usize);

fn mark(wb: &Workbook) -> Mark {
    let (undo, redo) = wb.history_depth();
    (wb.generation, undo, redo)
}

pub(crate) struct Recorder {
    /// The workbook as it was after the last action seen.
    baseline: Workbook,
    scope: Scope,
    pub actions: Vec<Recorded>,
    seen: Mark,
    before: Before,
}

impl Recorder {
    /// Start recording what is done in a window.
    pub fn start(app: &ExcelApp) -> Recorder {
        Recorder {
            baseline: app.wb.fork(),
            scope: Scope {
                start: app.wb.sheets.get(app.sheet).map(|sheet| sheet.name.clone()),
            },
            actions: Vec::new(),
            seen: mark(&app.wb),
            before: Before::of(app),
        }
    }

    /// The window changed: record what was done to the workbook, if
    /// anything was. `refit` is what the window does to a workbook a
    /// script just ran on (see `run::refit`). True when the list of
    /// actions changed.
    pub fn observe(&mut self, app: &ExcelApp, refit: diff::Refit) -> bool {
        let now = mark(&app.wb);
        if now == self.seen {
            // Nothing done yet: this is what the next action starts from.
            self.before = Before::of(app);
            return false;
        }
        let was = std::mem::replace(&mut self.seen, now);
        let undone = now.1 < was.1 && now.2 > was.2;
        let after = app.wb.fork();
        let changed = if undone && self.actions.last().is_some_and(|last| last.depth == was.1) {
            // The last action was taken back: it leaves the script.
            if let Some(last) = self.actions.pop() {
                self.scope = last.scope_before;
            }
            self.baseline = after;
            true
        } else {
            let label = if undone {
                crate::i18n::t("Undo", "Annuler").to_string()
            } else {
                app.wb.undo_label().unwrap_or_default().to_string()
            };
            self.record(label, after, Before::of(app), now.1, refit)
        };
        self.before = Before::of(app);
        changed
    }

    /// Write the action that led from the baseline to `after`.
    fn record(&mut self, label: String, after: Workbook, now: Before, depth: usize, refit: diff::Refit) -> bool {
        if compare::distance(&self.baseline, &after) == 0 {
            self.baseline = after;
            return false;
        }
        let scope_before = self.scope.clone();
        let mut work = std::mem::replace(&mut self.baseline, after);
        let after = &self.baseline;
        // The statement Excel would write, when one is found that does it.
        let mut steps = Vec::new();
        let mut left = compare::distance(&work, after);
        let mut best: Option<Workbook> = None;
        for candidate in candidates::of(&work, after, &self.before, &now, &self.scope) {
            let mut trial = work.fork();
            if diff::run(&mut trial, &self.scope, &candidate, refit).is_err() {
                continue;
            }
            let distance = compare::distance(&trial, after);
            if distance < left {
                left = distance;
                best = Some(trial);
                steps = candidate;
                if distance == 0 {
                    break;
                }
            }
        }
        if let Some(trial) = best {
            work = trial;
        }
        let mut scope = self.scope.clone();
        let recorded = match diff::reach(&mut work, after, &mut scope, refit) {
            Ok(more) => {
                steps.extend(more);
                true
            }
            Err(_) => {
                steps.clear();
                // `selectedSheet` cannot follow a sheet through an action
                // that is not in the script.
                scope = self.scope.clone();
                if scope.start.as_deref().is_some_and(|name| !after.sheets.iter().any(|sheet| sheet.name == name)) {
                    scope.start = None;
                }
                false
            }
        };
        self.scope = scope;
        self.actions.push(Recorded {
            label,
            steps,
            recorded,
            depth,
            scope_before,
        });
        true
    }

    /// The script of what was recorded so far.
    pub fn script(&self) -> String {
        let steps: Vec<Step> = self.actions.iter().flat_map(|action| action.steps.iter().cloned()).collect();
        code::script(&steps)
    }
}
