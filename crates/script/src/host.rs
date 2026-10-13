//! The workbook as the bindings see it while a script runs: the sheets
//! behind the Worksheet objects of the script, what the window shows,
//! when to calculate, what protection allows.

use serde_json::Value as J;

use opensheet_engine::features::Edit;
use opensheet_engine::locale::Locale;
use opensheet_engine::protect::Action;
use opensheet_engine::recalc::CalcMode;
use opensheet_engine::refs::{MAX_COLS, MAX_ROWS, Rect};
use opensheet_engine::value::Value;
use opensheet_engine::workbook::Workbook;

use crate::api::{Call, Res, object};
use crate::{Autofit, Context, Log};

/// The most cells one call reads or writes as an array (Excel refuses
/// larger requests too).
const MAX_ARRAY_CELLS: u64 = 5_000_000;
/// The most cells a run may write: a script gone wrong must not fill the
/// memory of the application.
const MAX_WRITTEN_CELLS: u64 = 20_000_000;
/// The most sheets a script may leave in a workbook, for the same reason.
pub(crate) const MAX_SHEETS: usize = 5_000;
/// The log lines kept.
pub(crate) const MAX_LOGS: usize = 5_000;

pub(crate) struct Host {
    pub wb: Workbook,
    /// The identifier of each sheet, in the order of the sheets: a
    /// Worksheet object of the script keeps meaning the same sheet when
    /// sheets are added, moved or deleted.
    sheet_ids: Vec<i64>,
    next_sheet_id: i64,
    /// The sheet in front, by identifier.
    active: i64,
    pub selection: Rect,
    pub active_cell: (u32, u32),
    pub activated: bool,
    pub selected: bool,
    pub logs: Vec<Log>,
    pub workbook_name: String,
    pub user_name: String,
    pub autofits: Vec<(i64, bool, u32, u32)>,
    /// Cells were changed since formulas were calculated.
    stale: bool,
    /// Sheets were changed otherwise than cell by cell since then.
    restructured: bool,
    /// Formulas were calculated during the run.
    pub calculated: bool,
    /// A member that can change the workbook was called.
    pub edited: bool,
    /// A sheet was protected or unprotected.
    pub protection_changed: bool,
    written: u64,
    /// The engine failed under a call: the run is lost whatever the
    /// script does next.
    pub fatal: Option<String>,
    /// The conditional formats the script holds: for each handle, the
    /// sheet (by identifier) and the place of the rule in its priority
    /// order; `None` once the rule is deleted. The engine names rules by
    /// their place, which moves when the script adds, deletes or
    /// reorders rules: the handles follow.
    cond_formats: Vec<Option<(i64, usize)>>,
}

impl Host {
    pub fn new(wb: Workbook, context: &Context) -> Host {
        let count = wb.sheets.len() as i64;
        let active = (context.active_sheet as i64).min(count - 1).max(0);
        Host {
            wb,
            sheet_ids: (0..count).collect(),
            next_sheet_id: count,
            active,
            selection: context.selection,
            active_cell: context.active_cell,
            activated: false,
            selected: false,
            logs: Vec::new(),
            workbook_name: context.workbook_name.clone(),
            user_name: context.user_name.clone(),
            autofits: Vec::new(),
            stale: false,
            restructured: false,
            calculated: false,
            edited: false,
            protection_changed: false,
            written: 0,
            fatal: None,
            cond_formats: Vec::new(),
        }
    }

    // ---- sheets --------------------------------------------------------

    /// The index of the sheet a Worksheet object stands for.
    pub fn sheet(&self, call: &Call, id: i64) -> Res<usize> {
        self.sheet_index(id).ok_or_else(|| call.gone())
    }

    pub fn sheet_index(&self, id: i64) -> Option<usize> {
        self.sheet_ids.iter().position(|s| *s == id)
    }

    pub fn sheet_id(&self, index: usize) -> i64 {
        self.sheet_ids.get(index).copied().unwrap_or(-1)
    }

    pub fn worksheet(&self, index: usize) -> J {
        object("Worksheet", vec![self.sheet_id(index).into()])
    }

    pub fn range(&self, sheet: usize, rect: Rect) -> J {
        object(
            "Range",
            vec![self.sheet_id(sheet).into(), rect.r0.into(), rect.c0.into(), rect.r1.into(), rect.c1.into()],
        )
    }

    pub fn active_sheet(&self) -> usize {
        self.sheet_index(self.active).unwrap_or(0)
    }

    pub fn activate(&mut self, sheet: usize) {
        if self.sheet_id(sheet) != self.active {
            self.active = self.sheet_id(sheet);
            self.selection = Rect::cell(0, 0);
            self.active_cell = (0, 0);
        }
        self.activated = true;
    }

    pub fn sheet_added(&mut self, index: usize) {
        self.sheet_ids.insert(index.min(self.sheet_ids.len()), self.next_sheet_id);
        self.next_sheet_id += 1;
        self.restructured();
    }

    pub fn sheet_deleted(&mut self, index: usize) {
        let id = self.sheet_ids.remove(index);
        if id == self.active {
            // As in Excel, the next sheet with a tab comes in front (the
            // one before for the last).
            let next = self.wb.nearest_visible_sheet(index.min(self.sheet_ids.len() - 1));
            self.active = self.sheet_id(next);
            self.selection = Rect::cell(0, 0);
            self.active_cell = (0, 0);
            self.activated = true;
        }
        self.restructured();
    }

    /// A sheet was hidden: when it was the one in front, the nearest
    /// sheet with a tab takes its place, as when it is hidden by hand.
    pub fn sheet_hidden(&mut self, index: usize) {
        if self.sheet_id(index) == self.active {
            let next = self.wb.nearest_visible_sheet(index);
            self.activate(next);
        }
    }

    pub fn sheet_moved(&mut self, from: usize, to: usize) {
        let id = self.sheet_ids.remove(from);
        self.sheet_ids.insert(to, id);
        self.restructured();
    }

    // ---- conditional formats ------------------------------------------------

    /// The handle of the rule at a place of a sheet: the one the script
    /// already holds, or a new one.
    pub fn cond_handle(&mut self, sheet: usize, index: usize) -> i64 {
        let key = Some((self.sheet_id(sheet), index));
        let at = self.cond_formats.iter().position(|held| *held == key).unwrap_or_else(|| {
            self.cond_formats.push(key);
            self.cond_formats.len() - 1
        });
        at as i64
    }

    /// The sheet and the place of the rule a handle stands for.
    pub fn cond_format(&self, call: &Call, handle: i64) -> Res<(usize, usize)> {
        let held = usize::try_from(handle).ok().and_then(|at| self.cond_formats.get(at).copied().flatten());
        let (id, index) = held.ok_or_else(|| call.gone())?;
        let sheet = self.sheet(call, id)?;
        if index >= self.wb.sheets[sheet].cond_formats.len() {
            return Err(call.gone());
        }
        Ok((sheet, index))
    }

    /// The rules of a sheet changed places: `moved` says where the rule
    /// at a place went, `None` when it is gone.
    pub fn cond_formats_moved(&mut self, sheet: usize, moved: impl Fn(usize) -> Option<usize>) {
        let id = self.sheet_id(sheet);
        for held in &mut self.cond_formats {
            if let Some((of, index)) = *held {
                if of == id {
                    *held = moved(index).map(|to| (id, to));
                }
            }
        }
    }

    // ---- calculation -----------------------------------------------------

    /// Cells were written: formulas are calculated before the next read.
    pub fn changed(&mut self) {
        self.stale = true;
    }

    /// Something else than the content of cells changed (rows, sheets,
    /// tables, names): everything is calculated before the next read.
    pub fn restructured(&mut self) {
        self.stale = true;
        self.restructured = true;
    }

    /// Bring the results of formulas up to date before reading them, as
    /// Excel does between two calls of a script. With manual calculation
    /// they wait for `Application.calculate`.
    pub fn calculate(&mut self) {
        if self.stale && self.wb.calc_mode() != CalcMode::Manual {
            self.calculate_now();
        }
    }

    pub fn calculate_now(&mut self) {
        if self.restructured {
            self.wb.recalc();
        } else {
            self.wb.calculate_now();
        }
        self.stale = false;
        self.restructured = false;
        self.calculated = true;
    }

    // ---- limits and permissions --------------------------------------------

    /// Refuse arrays no application would hold.
    pub fn sized(&self, call: &Call, rect: Rect) -> Res<()> {
        if rect.rows() as u64 * rect.cols() as u64 > MAX_ARRAY_CELLS {
            return Err(call.fail("The request is too large: it covers more than 5,000,000 cells."));
        }
        Ok(())
    }

    /// Count cells about to be written.
    pub fn spend(&mut self, call: &Call, cells: u64) -> Res<()> {
        self.written += cells;
        if self.written > MAX_WRITTEN_CELLS {
            return Err(call.fail("The script wrote more than 20,000,000 cells and was stopped."));
        }
        Ok(())
    }

    /// What sheet protection says of an edit, as when it is made by hand.
    pub fn permit_edit(&self, call: &Call, edit: &Edit) -> Res<()> {
        self.wb
            .permits_edit(edit)
            .map_err(|denied| call.fail(denied.message(Locale::en())))
    }

    pub fn permit(&self, call: &Call, action: &Action) -> Res<()> {
        self.wb
            .permits(action)
            .map_err(|denied| call.fail(denied.message(Locale::en())))
    }

    /// Run something with the workbook reading and writing formulas,
    /// numbers and dates the English way, as the API does whatever the
    /// language of the workbook.
    pub fn english<T>(&mut self, f: impl FnOnce(&mut Workbook) -> T) -> T {
        let own = self.wb.locale;
        self.wb.locale = Locale::en();
        let out = f(&mut self.wb);
        self.wb.locale = own;
        out
    }

    /// What `autofitColumns` and `autofitRows` did, for the window to do
    /// again with the fonts it draws with.
    pub fn autofits(&self) -> Vec<Autofit> {
        self.autofits
            .iter()
            .filter_map(|(id, columns, first, last)| {
                Some(Autofit {
                    sheet: self.sheet_index(*id)?,
                    columns: *columns,
                    first: *first,
                    last: *last,
                })
            })
            .collect()
    }

    pub fn selected(&self) -> Option<(usize, Rect)> {
        self.selected.then(|| (self.active_sheet(), self.selection))
    }
}

/// A cell value as scripts see it: text, number or boolean, an empty
/// cell as empty text, an error as its text.
pub(crate) fn to_script(value: &Value) -> J {
    match value {
        Value::Empty => J::from(""),
        Value::Number(n) => serde_json::Number::from_f64(*n).map_or(J::Null, J::Number),
        Value::Text(s) => J::from(s.as_ref()),
        Value::Bool(b) => J::from(*b),
        Value::Error(e) => J::from(e.as_str()),
        Value::Array(a) => to_script(a.get(0, 0)),
    }
}

/// Clip a rectangle given as numbers by the script to the sheet.
pub(crate) fn rect_from(r0: i64, c0: i64, rows: i64, cols: i64) -> Option<Rect> {
    if r0 < 0 || c0 < 0 || rows < 1 || cols < 1 {
        return None;
    }
    let (r1, c1) = (r0 + rows - 1, c0 + cols - 1);
    (r1 < MAX_ROWS as i64 && c1 < MAX_COLS as i64).then(|| Rect::new(r0 as u32, c0 as u32, r1 as u32, c1 as u32))
}
