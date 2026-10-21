//! What-If Analysis: Goal Seek, scenarios and data tables.
//!
//! All three ask "what would this formula give if that cell held something
//! else", and answer the same way: the cell is given the value, what
//! depends on it is calculated (`Workbook::calculate_changes`, which
//! follows the dependency graph), and the result is read.
//!
//! * **Goal Seek** searches the value of one cell for which a formula gives
//!   a target, and leaves it in the cell as an uncommitted change.
//! * **Scenarios** (`Sheet::scenarios`) are named sets of values for a few
//!   cells of a sheet.
//! * **Data tables** (`Sheet::data_tables`) show a formula for a row and /
//!   or a column of trial values. Their cells hold plain values that are
//!   worked out again whenever the workbook is calculated (not after each
//!   change under "Automatic except for data tables"); to the user they
//!   read `{=TABLE(row,col)}` and cannot be changed one by one.

use crate::ast::StructuralEdit;
use crate::filter::AutoFilter;
use crate::recalc::CalcMode;
use crate::refs::{MAX_COLS, MAX_ROWS, Rect, cell_name};
use crate::style::Color;
use crate::value::Value;
use crate::workbook::{Cell, Content, Sheet, Workbook};
use std::sync::Arc;

/// A cell: sheet, row, column.
pub type CellPos = (usize, u32, u32);

impl Workbook {
    /// Replace a cell behind the back of the undo history, telling the
    /// calculation what changed: for the trial values of a search.
    fn poke(&mut self, at: CellPos, cell: Option<Cell>) {
        let (sheet, row, col) = at;
        let Some(sh) = self.sheets.get_mut(sheet) else {
            return;
        };
        let replaced = sh
            .cells
            .get(&(row, col))
            .is_some_and(|c| matches!(c.content, Content::Formula(_)));
        match cell {
            Some(cell) => {
                sh.cells.insert((row, col), cell);
            }
            None => {
                sh.cells.remove(&(row, col));
            }
        }
        self.calc.touch(sheet, row, col, replaced);
    }

    /// Give a cell a trial value, keeping its formatting.
    fn poke_value(&mut self, at: CellPos, value: Value) {
        let style = self.style_id_at(at.0, at.1, at.2);
        let content = if value.is_empty() {
            Content::Empty
        } else {
            Content::Value(value.clone())
        };
        self.poke(
            at,
            Some(Cell {
                content,
                value,
                style,
            }),
        );
    }

    fn cell_at(&self, at: CellPos) -> Option<&Cell> {
        self.cell(at.0, at.1, at.2)
    }
}

// ---------------------------------------------------------------- Goal Seek

/// Why Goal Seek cannot start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoalSeekError {
    /// "Set cell" does not hold a formula.
    NotFormula,
    /// "By changing cell" holds a formula or text.
    NotValue,
}

/// What Goal Seek arrived at ("Goal Seek Status").
#[derive(Clone, Debug, PartialEq)]
pub struct GoalSeek {
    /// The formula reaches the target (within 0.001, Excel's "maximum
    /// change").
    pub found: bool,
    pub target: f64,
    /// What the formula gives now.
    pub current: Value,
    /// What the changing cell holds now.
    pub input: f64,
}

/// Calculations Goal Seek allows itself (Excel's "maximum iterations").
const GOAL_SEEK_STEPS: usize = 100;
/// How close to the target counts as found.
const GOAL_SEEK_CLOSE: f64 = 0.001;

impl Workbook {
    /// Look for the value of `changing` that makes the formula in `set`
    /// give `target`. The best value found is left in the cell as an
    /// uncommitted change: `commit` keeps it (OK in the status dialog),
    /// [`Workbook::goal_seek_restore`] puts the old one back (Cancel).
    pub fn goal_seek(
        &mut self,
        set: CellPos,
        target: f64,
        changing: CellPos,
    ) -> Result<GoalSeek, GoalSeekError> {
        if !self
            .cell_at(set)
            .is_some_and(|cell| matches!(cell.content, Content::Formula(_)))
        {
            return Err(GoalSeekError::NotFormula);
        }
        let original = self.cell_at(changing).cloned();
        let start = match original.as_ref().map(|cell| &cell.content) {
            None | Some(Content::Empty) => 0.0,
            Some(Content::Value(Value::Number(n))) => *n,
            _ => return Err(GoalSeekError::NotValue),
        };
        // How far the formula is from the target with `x` in the cell;
        // `None` when it gives no number there.
        let miss = |wb: &mut Workbook, x: f64| -> Option<f64> {
            wb.poke_value(changing, Value::Number(x));
            wb.calculate_changes();
            match wb.value_at(set.0, set.1, set.2) {
                Value::Number(n) if n.is_finite() => Some(n - target),
                _ => None,
            }
        };
        let exact = 1e-10 * target.abs().max(1.0);
        let mut best: Option<(f64, f64)> = None;
        let consider = |x: f64, f: Option<f64>, best: &mut Option<(f64, f64)>| {
            if let Some(f) = f {
                if best.is_none_or(|(_, known)| f.abs() < known.abs()) {
                    *best = Some((x, f));
                }
            }
        };
        let (mut x0, mut f0) = (start, miss(self, start));
        consider(x0, f0, &mut best);
        let step = if start == 0.0 {
            0.01
        } else {
            start.abs() * 0.01
        };
        let mut x1 = start + step;
        let mut steps = 1;
        // The secant method, as long as both ends give numbers; once the
        // target lies between two trials, they are brought together.
        while steps < GOAL_SEEK_STEPS && !best.is_some_and(|(_, f)| f.abs() <= exact) {
            let f1 = miss(self, x1);
            steps += 1;
            consider(x1, f1, &mut best);
            let (Some(a), Some(b)) = (f0, f1) else {
                // No number there: come back towards the best trial.
                let Some((x, f)) = best else { break };
                let far = if f1.is_none() { x1 } else { x0 };
                (x0, f0) = (x, Some(f));
                x1 = x + (far - x) / 2.0;
                if (x1 - x).abs() < f64::EPSILON * x.abs().max(1.0) {
                    break;
                }
                continue;
            };
            if a * b < 0.0 {
                // Bracketed: false position, with the Illinois correction
                // against a stuck end.
                let (mut lo, mut flo, mut hi, mut fhi) = (x0, a, x1, b);
                let mut side = 0i8;
                while steps < GOAL_SEEK_STEPS * 2
                    && (hi - lo).abs() > f64::EPSILON * lo.abs().max(hi.abs()).max(1.0)
                {
                    let mut x = (lo * fhi - hi * flo) / (fhi - flo);
                    if !x.is_finite() || (x - lo) * (x - hi) > 0.0 {
                        x = (lo + hi) / 2.0;
                    }
                    let f = miss(self, x);
                    steps += 1;
                    consider(x, f, &mut best);
                    let Some(f) = f else { break };
                    if f.abs() <= exact {
                        break;
                    }
                    if f * fhi > 0.0 {
                        (hi, fhi) = (x, f);
                        if side == -1 {
                            flo /= 2.0;
                        }
                        side = -1;
                    } else {
                        (lo, flo) = (x, f);
                        if side == 1 {
                            fhi /= 2.0;
                        }
                        side = 1;
                    }
                }
                break;
            }
            let slope = (b - a) / (x1 - x0);
            let mut next = if slope == 0.0 || !slope.is_finite() {
                x1 + (x1 - x0) * 2.0
            } else {
                x1 - b / slope
            };
            // A flat stretch must not throw the search out of sight.
            let reach = (x1 - x0).abs().max(x1.abs()).max(1.0) * 100.0;
            if !next.is_finite() || (next - x1).abs() > reach {
                next = x1 + reach.copysign(if next.is_finite() { next - x1 } else { x1 - x0 });
            }
            if next == x1 {
                break;
            }
            (x0, f0) = (x1, f1);
            x1 = next;
        }
        // The old value comes back unrecorded, then the result is entered
        // as one change.
        let input = best.map_or(start, |(x, _)| x);
        self.poke(changing, original);
        self.set_value(changing.0, changing.1, changing.2, Value::Number(input));
        self.calculate_changes();
        let current = self.value_at(set.0, set.1, set.2);
        Ok(GoalSeek {
            found: best.is_some_and(|(_, f)| f.abs() < GOAL_SEEK_CLOSE),
            target,
            current,
            input,
        })
    }

    /// Cancel in the Goal Seek Status dialog: the changing cell gets its
    /// value back.
    pub fn goal_seek_restore(&mut self) {
        self.rollback();
        self.calculate_changes();
    }
}

// ---------------------------------------------------------------- scenarios

/// The value a scenario gives one cell: what would be typed into it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScenarioCell {
    pub row: u32,
    pub col: u32,
    pub value: String,
}

/// A named set of values for the changing cells of a sheet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scenario {
    pub name: String,
    pub comment: String,
    /// Who created or last changed it.
    pub user: String,
    /// "Prevent changes" and "Hide": they count once the sheet is
    /// protected.
    pub locked: bool,
    pub hidden: bool,
    pub cells: Vec<ScenarioCell>,
}

/// Excel's limit on the changing cells of a scenario.
pub const MAX_SCENARIO_CELLS: usize = 32;

/// Why a scenario cannot be stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScenarioError {
    NoName,
    /// Another scenario of the sheet has that name.
    Duplicate,
    NoCells,
    /// More than [`MAX_SCENARIO_CELLS`] changing cells.
    TooManyCells,
    /// No scenario at that index.
    Missing,
}

/// The words of a Scenario Summary sheet, in the interface language.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SummaryLabels {
    /// Also the name of the sheet.
    pub title: String,
    pub current_values: String,
    pub changing_cells: String,
    pub result_cells: String,
    /// The lines of the note under the report.
    pub notes: Vec<String>,
}

impl Workbook {
    fn check_scenario(
        &self,
        sheet: usize,
        scenario: &Scenario,
        replacing: Option<usize>,
    ) -> Result<(), ScenarioError> {
        let name = scenario.name.trim();
        if name.is_empty() {
            return Err(ScenarioError::NoName);
        }
        let taken = self.sheets[sheet]
            .scenarios
            .iter()
            .enumerate()
            .any(|(i, other)| {
                Some(i) != replacing && other.name.to_lowercase() == name.to_lowercase()
            });
        if taken {
            return Err(ScenarioError::Duplicate);
        }
        match scenario.cells.len() {
            0 => Err(ScenarioError::NoCells),
            n if n > MAX_SCENARIO_CELLS => Err(ScenarioError::TooManyCells),
            _ => Ok(()),
        }
    }

    /// Add a scenario to a sheet. The caller commits.
    pub fn scenario_add(&mut self, sheet: usize, scenario: Scenario) -> Result<(), ScenarioError> {
        if sheet >= self.sheets.len() {
            return Err(ScenarioError::Missing);
        }
        self.check_scenario(sheet, &scenario, None)?;
        self.snapshot();
        self.sheets[sheet].scenarios.push(scenario);
        Ok(())
    }

    /// Replace the scenario at `index`. The caller commits.
    pub fn scenario_edit(
        &mut self,
        sheet: usize,
        index: usize,
        scenario: Scenario,
    ) -> Result<(), ScenarioError> {
        if !self
            .sheets
            .get(sheet)
            .is_some_and(|sh| index < sh.scenarios.len())
        {
            return Err(ScenarioError::Missing);
        }
        self.check_scenario(sheet, &scenario, Some(index))?;
        self.snapshot();
        self.sheets[sheet].scenarios[index] = scenario;
        Ok(())
    }

    /// Delete the scenario at `index`. The caller commits.
    pub fn scenario_delete(&mut self, sheet: usize, index: usize) -> bool {
        if !self
            .sheets
            .get(sheet)
            .is_some_and(|sh| index < sh.scenarios.len())
        {
            return false;
        }
        self.snapshot();
        self.sheets[sheet].scenarios.remove(index);
        true
    }

    /// Show a scenario: its values are entered into its changing cells.
    /// Returns the cells changed. The caller commits.
    pub fn scenario_show(&mut self, sheet: usize, index: usize) -> Option<Vec<Rect>> {
        let scenario = self.sheets.get(sheet)?.scenarios.get(index)?.clone();
        for cell in &scenario.cells {
            self.enter_scenario_value(sheet, cell);
        }
        Some(
            scenario
                .cells
                .iter()
                .map(|cell| Rect::cell(cell.row, cell.col))
                .collect(),
        )
    }

    fn enter_scenario_value(&mut self, sheet: usize, cell: &ScenarioCell) {
        // A scenario holds constants: what reads as a formula stays text.
        let constant = !cell.value.starts_with('=');
        if !constant
            || self
                .set_cell_text(sheet, cell.row, cell.col, &cell.value)
                .is_err()
        {
            self.set_content(
                sheet,
                cell.row,
                cell.col,
                Content::Value(Value::text(&cell.value)),
            );
        }
    }

    /// Scenario Manager > Summary: a new sheet, before `sheet`, listing
    /// for the current values and for each scenario what the changing
    /// cells hold and what the result cells give. Returns the index of
    /// the new sheet (`sheet` is then one further). The caller commits.
    pub fn scenario_summary(
        &mut self,
        sheet: usize,
        results: &[(u32, u32)],
        labels: &SummaryLabels,
    ) -> Option<usize> {
        let scenarios = self.sheets.get(sheet)?.scenarios.clone();
        if scenarios.is_empty() {
            return None;
        }
        // The changing cells of all the scenarios, in order of appearance.
        let mut changing: Vec<(u32, u32)> = Vec::new();
        for cell in scenarios.iter().flat_map(|scenario| &scenario.cells) {
            if !changing.contains(&(cell.row, cell.col)) {
                changing.push((cell.row, cell.col));
            }
        }
        let read = |wb: &Workbook, cells: &[(u32, u32)]| -> Vec<(Value, Arc<str>)> {
            cells
                .iter()
                .map(|(row, col)| {
                    (
                        wb.value_at(sheet, *row, *col),
                        wb.style_at(sheet, *row, *col).num_fmt.clone(),
                    )
                })
                .collect()
        };
        // One column per state of the sheet: now, then under each
        // scenario. The scenarios are tried and taken back.
        let mut columns = vec![(read(self, &changing), read(self, results))];
        for scenario in &scenarios {
            for cell in &scenario.cells {
                self.enter_scenario_value(sheet, cell);
            }
            self.calculate_changes();
            columns.push((read(self, &changing), read(self, results)));
            self.rollback();
            self.calculate_changes();
        }

        let report = self.add_sheet(sheet);
        let mut name = labels.title.clone();
        let mut n = 2;
        while self.sheet_index(&name).is_some() {
            name = format!("{} {n}", labels.title);
            n += 1;
        }
        self.sheets[report].name = name;
        self.sheets[report].show_gridlines = false;
        let (navy, grey) = (Color::rgb(0x1F, 0x38, 0x64), Color::rgb(0xD9, 0xD9, 0xD9));
        let text = |wb: &mut Workbook, row: u32, col: u32, text: &str| {
            wb.set_content(report, row, col, Content::Value(Value::text(text)));
        };
        let heading = |wb: &mut Workbook, row: u32, c0: u32, c1: u32| {
            wb.update_style(report, Rect::new(row, c0, row, c1), &|s| {
                s.font.bold = true;
                s.font.color = Some(Color::WHITE);
                s.fill = Some(navy);
            });
        };
        let last_col = 3 + scenarios.len() as u32;
        // Row 2: the title; row 3: what each column stands for.
        text(self, 1, 1, &labels.title);
        heading(self, 1, 1, last_col);
        text(self, 2, 3, &labels.current_values);
        for (i, scenario) in scenarios.iter().enumerate() {
            text(self, 2, 4 + i as u32, &scenario.name);
        }
        heading(self, 2, 1, last_col);
        self.update_style(report, Rect::new(2, 3, 2, last_col), &|s| {
            s.h_align = crate::style::HAlign::Right
        });
        let mut row = 3;
        let sections = [
            (&labels.changing_cells, &changing, true),
            (&labels.result_cells, &results.to_vec(), false),
        ];
        // The rows of each section, for the outline.
        let mut groups: Vec<(u32, u32)> = Vec::new();
        for (title, cells, is_changing) in sections {
            if cells.is_empty() {
                continue;
            }
            text(self, row, 1, title);
            self.update_style(report, Rect::new(row, 1, row, last_col), &|s| {
                s.font.bold = true
            });
            row += 1;
            groups.push((row, row + cells.len() as u32 - 1));
            for (i, (r, c)) in cells.iter().enumerate() {
                let name = cell_name(*r, *c);
                let letters = name.trim_end_matches(|ch: char| ch.is_ascii_digit());
                text(self, row, 2, &format!("${letters}${}", r + 1));
                for (j, column) in columns.iter().enumerate() {
                    let (value, format) = if is_changing {
                        &column.0[i]
                    } else {
                        &column.1[i]
                    };
                    let col = 3 + j as u32;
                    let content = if value.is_empty() {
                        Content::Empty
                    } else {
                        Content::Value(value.clone())
                    };
                    self.set_content(report, row, col, content);
                    let format = format.clone();
                    // What a scenario changes stands out in grey.
                    let changed = is_changing
                        && j > 0
                        && scenarios[j - 1]
                            .cells
                            .iter()
                            .any(|cell| (cell.row, cell.col) == (*r, *c));
                    self.update_style(report, Rect::cell(row, col), &move |s| {
                        s.num_fmt = format.clone();
                        if changed {
                            s.fill = Some(grey);
                        }
                    });
                }
                row += 1;
            }
        }
        for line in &labels.notes {
            text(self, row, 1, line);
            row += 1;
        }
        // Excel's report comes with an outline: the cells of each section
        // fold under its heading.
        self.sheets[report].outline.summary_below = false;
        for (first, last) in groups {
            self.outline_group(report, crate::outline::Axis::Rows, first, last)
                .ok()?;
        }
        Some(report)
    }
}

// -------------------------------------------------------------- data tables

/// A data table: the cells that show the results (the selection without
/// its first row and first column) and the cells the trial values are
/// substituted into. The trial values sit in the row above the results
/// (for the row input cell) and in the column left of them (for the
/// column input cell); the formula is in the corner when both are used,
/// else at the head of each column or of each row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DataTable {
    /// The result cells.
    pub range: Rect,
    pub row_input: Option<(u32, u32)>,
    pub col_input: Option<(u32, u32)>,
}

/// Why a data table cannot be created.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataTableError {
    /// The selection is not at least two rows by two columns.
    BadSelection,
    /// Neither input cell was given.
    NoInput,
    /// An input cell lies inside the table.
    InputInside,
    /// The results would cover part of another data table.
    Overlap,
}

impl DataTable {
    /// The cell whose formula gives the result at (`row`, `col`).
    fn formula_cell(&self, row: u32, col: u32) -> (u32, u32) {
        match (self.row_input, self.col_input) {
            (Some(_), Some(_)) => (self.range.r0 - 1, self.range.c0 - 1),
            (None, _) => (self.range.r0 - 1, col),
            (Some(_), None) => (row, self.range.c0 - 1),
        }
    }

    /// `=TABLE(row,col)` as Excel shows it, with the separator of a locale.
    pub fn formula_text(&self, arg_sep: char) -> String {
        let name = |cell: Option<(u32, u32)>| {
            cell.map(|(row, col)| cell_name(row, col))
                .unwrap_or_default()
        };
        format!(
            "=TABLE({}{arg_sep}{})",
            name(self.row_input),
            name(self.col_input)
        )
    }
}

impl Workbook {
    /// The data table whose results include a cell.
    pub fn data_table_at(&self, sheet: usize, row: u32, col: u32) -> Option<&DataTable> {
        self.sheets
            .get(sheet)?
            .data_tables
            .iter()
            .find(|table| table.range.contains(row, col))
    }

    /// What the formula bar shows for a result cell of a data table:
    /// `{=TABLE(A1,B1)}`.
    pub fn data_table_text(&self, sheet: usize, row: u32, col: u32) -> Option<String> {
        self.data_table_at(sheet, row, col)
            .map(|table| format!("{{{}}}", table.formula_text(self.locale.arg_sep)))
    }

    /// True when changing the cells of `rect` would change part of a data
    /// table without replacing all of it ("Cannot change part of a data
    /// table").
    pub fn splits_data_table(&self, sheet: usize, rect: Rect) -> bool {
        self.sheets.get(sheet).is_some_and(|sh| {
            sh.data_tables.iter().any(|table| {
                table.range.intersects(&rect)
                    && rect.intersection(&table.range) != Some(table.range)
            })
        })
    }

    /// Forget the data tables whose results lie entirely in `rect` (they
    /// were cleared or overwritten as a whole). The caller commits.
    pub fn remove_data_tables_in(&mut self, sheet: usize, rect: Rect) -> bool {
        let inside = |table: &DataTable| rect.intersection(&table.range) == Some(table.range);
        if !self
            .sheets
            .get(sheet)
            .is_some_and(|sh| sh.data_tables.iter().any(inside))
        {
            return false;
        }
        self.snapshot();
        self.sheets[sheet]
            .data_tables
            .retain(|table| !inside(table));
        true
    }

    /// Data > What-If Analysis > Data Table on the selection `range`
    /// (trial values and formulas in its first row and first column).
    /// The results are calculated at once. The caller commits.
    pub fn data_table_create(
        &mut self,
        sheet: usize,
        range: Rect,
        row_input: Option<(u32, u32)>,
        col_input: Option<(u32, u32)>,
    ) -> Result<Rect, DataTableError> {
        let Some(sh) = self.sheets.get(sheet) else {
            return Err(DataTableError::BadSelection);
        };
        if range.rows() < 2 || range.cols() < 2 {
            return Err(DataTableError::BadSelection);
        }
        if row_input.is_none() && col_input.is_none() {
            return Err(DataTableError::NoInput);
        }
        if [row_input, col_input]
            .into_iter()
            .flatten()
            .any(|(row, col)| range.contains(row, col))
        {
            return Err(DataTableError::InputInside);
        }
        let results = Rect::new(range.r0 + 1, range.c0 + 1, range.r1, range.c1);
        // An identical table is simply calculated again.
        if sh
            .data_tables
            .iter()
            .any(|table| table.range.intersects(&results) && table.range != results)
        {
            return Err(DataTableError::Overlap);
        }
        self.snapshot();
        self.clear(sheet, results, crate::workbook::ClearMode::Contents);
        let tables = &mut self.sheets[sheet].data_tables;
        tables.retain(|table| table.range != results);
        tables.push(DataTable {
            range: results,
            row_input,
            col_input,
        });
        self.calculate_changes();
        self.calculate_data_tables();
        Ok(results)
    }

    /// Work out the results of every data table again. Called after each
    /// calculation of the workbook, unless the mode leaves data tables
    /// out.
    pub(crate) fn calculate_data_tables(&mut self) {
        let tables: Vec<(usize, DataTable)> = self
            .sheets
            .iter()
            .enumerate()
            .flat_map(|(sheet, sh)| sh.data_tables.iter().map(move |table| (sheet, *table)))
            .collect();
        for (sheet, table) in tables {
            let inputs: Vec<(u32, u32)> = [table.row_input, table.col_input]
                .into_iter()
                .flatten()
                .collect();
            let saved: Vec<Option<Cell>> = inputs
                .iter()
                .map(|(row, col)| self.cell(sheet, *row, *col).cloned())
                .collect();
            let mut results =
                Vec::with_capacity(table.range.rows() as usize * table.range.cols() as usize);
            for row in table.range.r0..=table.range.r1 {
                for col in table.range.c0..=table.range.c1 {
                    if let Some((r, c)) = table.row_input {
                        let trial = self.value_at(sheet, table.range.r0 - 1, col);
                        self.poke_value((sheet, r, c), trial);
                    }
                    if let Some((r, c)) = table.col_input {
                        let trial = self.value_at(sheet, row, table.range.c0 - 1);
                        self.poke_value((sheet, r, c), trial);
                    }
                    self.calculate_changes();
                    let (r, c) = table.formula_cell(row, col);
                    results.push(self.value_at(sheet, r, c));
                }
            }
            for ((row, col), cell) in inputs.iter().zip(saved) {
                self.poke((sheet, *row, *col), cell);
            }
            let mut results = results.into_iter();
            for row in table.range.r0..=table.range.r1 {
                for col in table.range.c0..=table.range.c1 {
                    let value = results.next().unwrap_or_default();
                    if self.value_at(sheet, row, col) != value
                        || self.cell(sheet, row, col).is_none()
                    {
                        self.poke_value((sheet, row, col), value);
                    }
                }
            }
            self.calculate_changes();
        }
    }

    /// What `changed` does about data tables: they follow every change
    /// under automatic calculation only.
    pub(crate) fn data_tables_after_change(&mut self) {
        if self.calc_mode() == CalcMode::Automatic
            && self.sheets.iter().any(|sh| !sh.data_tables.is_empty())
        {
            self.calculate_data_tables();
        }
    }
}

// ---------------------------------------------------------- structural edits

/// Where a cell goes when rows or columns are inserted or deleted; `None`
/// when it is deleted.
fn moved_cell(cell: (u32, u32), edit: StructuralEdit) -> Option<(u32, u32)> {
    let mut range = AutoFilter::new(Rect::cell(cell.0, cell.1));
    let (row, col) = cell;
    let gone = match edit {
        StructuralEdit::DeleteRows { at, count } => row >= at && row < at + count,
        StructuralEdit::DeleteCols { at, count } => col >= at && col < at + count,
        StructuralEdit::InsertRows { at, count } => row >= at && row + count >= MAX_ROWS,
        StructuralEdit::InsertCols { at, count } => col >= at && col + count >= MAX_COLS,
    };
    (!gone && range.adjust(edit)).then(|| (range.range.r0, range.range.c0))
}

/// See `features::on_structural_edit`. Scenarios and data tables name
/// cells of their own sheet only.
pub(crate) fn on_structural_edit(sheet: &mut Sheet, edit: StructuralEdit, edited_sheet: &str) {
    if sheet.name != edited_sheet {
        return;
    }
    for scenario in &mut sheet.scenarios {
        scenario
            .cells
            .retain_mut(|cell| match moved_cell((cell.row, cell.col), edit) {
                Some((row, col)) => {
                    (cell.row, cell.col) = (row, col);
                    true
                }
                None => false,
            });
    }
    sheet
        .scenarios
        .retain(|scenario| !scenario.cells.is_empty());
    sheet.data_tables.retain_mut(|table| {
        // The row and the column of trial values move with the results:
        // a table cut off from them, or from an input cell, is no table.
        let whole = Rect::new(
            table.range.r0 - 1,
            table.range.c0 - 1,
            table.range.r1,
            table.range.c1,
        );
        let mut moved = AutoFilter::new(whole);
        if !moved.adjust(edit) || moved.range.rows() < 2 || moved.range.cols() < 2 {
            return false;
        }
        let inputs = [table.row_input, table.col_input]
            .map(|input| input.map(|cell| moved_cell(cell, edit)));
        if inputs.iter().any(|input| *input == Some(None)) {
            return false;
        }
        let whole = moved.range;
        table.range = Rect::new(whole.r0 + 1, whole.c0 + 1, whole.r1, whole.c1);
        table.row_input = inputs[0].flatten();
        table.col_input = inputs[1].flatten();
        true
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{val, wb};

    fn n(x: f64) -> Value {
        Value::Number(x)
    }

    fn close(value: &Value, expected: f64) -> bool {
        matches!(value, Value::Number(x) if (x - expected).abs() < 1e-6)
    }

    // ---- Goal Seek ------------------------------------------------------

    #[test]
    fn goal_seek_on_a_line_and_a_curve() {
        let mut w = wb(&[("A1", "1"), ("B1", "=A1*3+2"), ("C1", "=B1*2")]);
        // Through an intermediate formula.
        let found = w.goal_seek((0, 0, 2), 28.0, (0, 0, 0)).unwrap();
        assert!(found.found);
        assert!(close(&found.current, 28.0));
        assert!((found.input - 4.0).abs() < 1e-9);
        w.commit("Goal Seek");
        assert!(close(&val(&w, "A1"), 4.0));
        assert!(close(&val(&w, "B1"), 14.0));
        // One step back.
        w.undo();
        assert_eq!(val(&w, "A1"), n(1.0));
        assert_eq!(val(&w, "C1"), n(10.0));

        let mut w = wb(&[("A1", "1"), ("B1", "=A1^3-2*A1")]);
        let found = w.goal_seek((0, 0, 1), 21.0, (0, 0, 0)).unwrap();
        assert!(found.found, "{found:?}");
        assert!((found.input - 3.0).abs() < 1e-6);
    }

    #[test]
    fn goal_seek_loan_payment() {
        // Which rate gives a payment of 900 on 100 000 over 180 months?
        let mut w = wb(&[
            ("A1", "0.05"),
            ("A2", "180"),
            ("A3", "100000"),
            ("A4", "=PMT(A1/12,A2,-A3)"),
        ]);
        let found = w.goal_seek((0, 3, 0), 900.0, (0, 0, 0)).unwrap();
        assert!(found.found, "{found:?}");
        assert!(close(&found.current, 900.0));
        assert!((found.input - 0.07025).abs() < 1e-4, "{}", found.input);
    }

    #[test]
    fn goal_seek_cancel_restores() {
        let mut w = wb(&[("A1", "2"), ("B1", "=A1*A1")]);
        let found = w.goal_seek((0, 0, 1), 49.0, (0, 0, 0)).unwrap();
        assert!(found.found);
        assert!(close(&val(&w, "A1"), 7.0));
        w.goal_seek_restore();
        assert_eq!(val(&w, "A1"), n(2.0));
        assert_eq!(val(&w, "B1"), n(4.0));
        assert!(!w.commit("nothing"));
        // An empty changing cell starts from zero and keeps its emptiness on Cancel.
        let mut w = wb(&[("B1", "=A1+5")]);
        let found = w.goal_seek((0, 0, 1), 12.0, (0, 0, 0)).unwrap();
        assert!(found.found);
        assert!((found.input - 7.0).abs() < 1e-9);
        w.goal_seek_restore();
        assert_eq!(val(&w, "A1"), Value::Empty);
        assert_eq!(val(&w, "B1"), n(5.0));
    }

    #[test]
    fn goal_seek_without_a_solution_and_refusals() {
        // A square never gives -1: the closest value is kept, not found.
        let mut w = wb(&[("A1", "3"), ("B1", "=A1*A1"), ("C1", "text"), ("D1", "=1")]);
        let found = w.goal_seek((0, 0, 1), -1.0, (0, 0, 0)).unwrap();
        assert!(!found.found);
        w.goal_seek_restore();
        assert_eq!(val(&w, "A1"), n(3.0));
        // The formula does not depend on the cell.
        let found = w.goal_seek((0, 0, 3), 5.0, (0, 0, 0)).unwrap();
        assert!(!found.found);
        w.goal_seek_restore();
        assert_eq!(
            w.goal_seek((0, 0, 0), 5.0, (0, 0, 0)),
            Err(GoalSeekError::NotFormula)
        );
        assert_eq!(
            w.goal_seek((0, 0, 1), 5.0, (0, 0, 2)),
            Err(GoalSeekError::NotValue)
        );
        assert_eq!(
            w.goal_seek((0, 0, 1), 5.0, (0, 0, 3)),
            Err(GoalSeekError::NotValue)
        );
    }

    // ---- scenarios ------------------------------------------------------

    fn scenario(name: &str, cells: &[(&str, &str)]) -> Scenario {
        Scenario {
            name: name.to_string(),
            cells: cells
                .iter()
                .map(|(cell, value)| {
                    let (row, col) = crate::refs::parse_cell_name(cell).unwrap();
                    ScenarioCell {
                        row,
                        col,
                        value: value.to_string(),
                    }
                })
                .collect(),
            ..Scenario::default()
        }
    }

    fn budget() -> Workbook {
        let mut w = wb(&[("A1", "100"), ("A2", "0.1"), ("A3", "=A1*(1+A2)")]);
        w.scenario_add(0, scenario("Low", &[("A1", "50"), ("A2", "5%")]))
            .unwrap();
        w.scenario_add(0, scenario("High", &[("A1", "200")]))
            .unwrap();
        w.commit("Add Scenario");
        w
    }

    #[test]
    fn scenarios_are_added_edited_deleted_and_shown() {
        let mut w = budget();
        assert_eq!(
            w.scenario_add(0, scenario("low", &[("A1", "1")])),
            Err(ScenarioError::Duplicate)
        );
        assert_eq!(
            w.scenario_add(0, scenario(" ", &[("A1", "1")])),
            Err(ScenarioError::NoName)
        );
        assert_eq!(
            w.scenario_add(0, scenario("None", &[])),
            Err(ScenarioError::NoCells)
        );
        let many: Vec<(String, &str)> = (0..33).map(|i| (cell_name(i, 5), "1")).collect();
        let many: Vec<(&str, &str)> = many
            .iter()
            .map(|(cell, value)| (cell.as_str(), *value))
            .collect();
        assert_eq!(
            w.scenario_add(0, scenario("Many", &many)),
            Err(ScenarioError::TooManyCells)
        );
        // Show enters the values as typing would, in one step.
        let changed = w.scenario_show(0, 0).unwrap();
        w.commit("Show Scenario");
        assert_eq!(changed.len(), 2);
        assert_eq!(val(&w, "A2"), n(0.05));
        assert_eq!(val(&w, "A3"), n(52.5));
        w.undo();
        assert_eq!(val(&w, "A3"), n(110.00000000000001));
        // Editing keeps the name free for itself; deleting is undone.
        w.scenario_edit(0, 1, scenario("High", &[("A1", "300")]))
            .unwrap();
        assert_eq!(
            w.scenario_edit(0, 1, scenario("Low", &[("A1", "300")])),
            Err(ScenarioError::Duplicate)
        );
        assert_eq!(
            w.scenario_edit(0, 5, scenario("X", &[("A1", "1")])),
            Err(ScenarioError::Missing)
        );
        w.commit("Edit Scenario");
        assert!(w.scenario_delete(0, 0));
        w.commit("Delete Scenario");
        assert_eq!(w.sheets[0].scenarios.len(), 1);
        assert_eq!(w.sheets[0].scenarios[0].cells[0].value, "300");
        w.undo();
        assert_eq!(w.sheets[0].scenarios.len(), 2);
        assert!(!w.scenario_delete(0, 9));
    }

    #[test]
    fn scenario_summary_sheet() {
        let mut w = budget();
        let labels = SummaryLabels {
            title: "Scenario Summary".into(),
            current_values: "Current Values:".into(),
            changing_cells: "Changing Cells:".into(),
            result_cells: "Result Cells:".into(),
            notes: vec!["Notes".into()],
        };
        let report = w.scenario_summary(0, &[(2, 0)], &labels).unwrap();
        w.commit("Scenario Summary");
        assert_eq!(report, 0);
        assert_eq!(w.sheets[0].name, "Scenario Summary");
        let at = |cell: &str| {
            let (row, col) = crate::refs::parse_cell_name(cell).unwrap();
            w.value_at(0, row, col)
        };
        assert_eq!(at("B2"), Value::text("Scenario Summary"));
        assert_eq!(at("D3"), Value::text("Current Values:"));
        assert_eq!(at("E3"), Value::text("Low"));
        assert_eq!(at("F3"), Value::text("High"));
        assert_eq!(at("B4"), Value::text("Changing Cells:"));
        assert_eq!(at("C5"), Value::text("$A$1"));
        assert_eq!(
            [at("D5"), at("E5"), at("F5")],
            [n(100.0), n(50.0), n(200.0)]
        );
        // A cell a scenario leaves alone keeps its current value.
        assert_eq!([at("D6"), at("E6"), at("F6")], [n(0.1), n(0.05), n(0.1)]);
        assert_eq!(at("B7"), Value::text("Result Cells:"));
        assert_eq!(at("C8"), Value::text("$A$3"));
        assert_eq!([at("D8"), at("E8")], [n(110.00000000000001), n(52.5)]);
        assert!(close(&at("F8"), 220.0));
        assert_eq!(at("B9"), Value::text("Notes"));
        // The cells of each section fold under its heading.
        let outline = &w.sheets[0].outline;
        assert!(!outline.summary_below);
        assert_eq!(
            (3..9)
                .map(|row| outline.rows.level(row))
                .collect::<Vec<_>>(),
            [0, 1, 1, 0, 1, 0]
        );
        // The sheet itself is as it was, and a second report gets another name.
        assert_eq!(w.value_at(1, 0, 0), n(100.0));
        assert_eq!(w.value_at(1, 2, 0), n(110.00000000000001));
        w.scenario_summary(1, &[], &labels).unwrap();
        w.commit("Scenario Summary");
        assert_eq!(w.sheets[1].name, "Scenario Summary 2");
        w.undo();
        w.undo();
        assert_eq!(w.sheets.len(), 1);
    }

    #[test]
    fn scenarios_follow_rows_and_columns() {
        let mut w = budget();
        w.insert_rows(0, 1, 2);
        w.commit("Insert");
        let low = &w.sheets[0].scenarios[0];
        assert_eq!((low.cells[0].row, low.cells[1].row), (0, 3));
        // Deleting the only cell of a scenario deletes the scenario.
        w.delete_rows(0, 0, 1);
        w.commit("Delete");
        assert_eq!(w.sheets[0].scenarios.len(), 1);
        assert_eq!(w.sheets[0].scenarios[0].name, "Low");
        assert_eq!(w.sheets[0].scenarios[0].cells.len(), 1);
    }

    // ---- data tables ----------------------------------------------------

    /// Price in B1, quantity in B2, total in B3; a two-variable table in
    /// D1:G4 (prices across, quantities down) and a one-variable one in
    /// D7:E9 (quantities down, the formula at the head of the column).
    fn tables() -> Workbook {
        let mut w = wb(&[
            ("B1", "10"),
            ("B2", "3"),
            ("B3", "=B1*B2"),
            ("D1", "=B3"),
            ("E1", "10"),
            ("F1", "20"),
            ("G1", "30"),
            ("D2", "1"),
            ("D3", "2"),
            ("D4", "3"),
            ("E7", "=B3+1"),
            ("D8", "5"),
            ("D9", "6"),
        ]);
        let range = Rect::parse("D1:G4").unwrap();
        assert_eq!(
            w.data_table_create(0, range, Some((0, 1)), Some((1, 1))),
            Ok(Rect::parse("E2:G4").unwrap())
        );
        w.commit("Data Table");
        let range = Rect::parse("D7:E9").unwrap();
        w.data_table_create(0, range, None, Some((1, 1))).unwrap();
        w.commit("Data Table");
        w
    }

    #[test]
    fn data_tables_show_the_formula_for_each_trial() {
        let w = tables();
        assert_eq!(val(&w, "E2"), n(10.0));
        assert_eq!(val(&w, "G4"), n(90.0));
        assert_eq!(val(&w, "F3"), n(40.0));
        assert_eq!(val(&w, "E8"), n(51.0));
        assert_eq!(val(&w, "E9"), n(61.0));
        // The input cells are what they were.
        assert_eq!(val(&w, "B1"), n(10.0));
        assert_eq!(val(&w, "B3"), n(30.0));
        assert_eq!(
            w.data_table_text(0, 1, 4).as_deref(),
            Some("{=TABLE(B1,B2)}")
        );
        assert_eq!(w.data_table_text(0, 7, 4).as_deref(), Some("{=TABLE(,B2)}"));
        assert_eq!(w.data_table_text(0, 0, 4), None);
    }

    #[test]
    fn data_tables_are_recalculated_with_the_workbook() {
        let mut w = tables();
        // A trial value, then the model itself.
        w.set_cell_text(0, 0, 5, "25").unwrap();
        w.commit("edit");
        assert_eq!(val(&w, "F3"), n(50.0));
        w.set_cell_text(0, 2, 1, "=B1*B2*2").unwrap();
        w.commit("edit");
        assert_eq!(val(&w, "G4"), n(180.0));
        assert_eq!(val(&w, "E9"), n(121.0));
        // Undo calculates them back.
        w.undo();
        assert_eq!(val(&w, "G4"), n(90.0));
        // A formula reading the table follows it.
        w.set_cell_text(0, 11, 0, "=SUM(E2:G4)").unwrap();
        w.commit("edit");
        assert_eq!(
            val(&w, "A12"),
            n(10.0 + 25.0 + 30.0 + 20.0 + 50.0 + 60.0 + 30.0 + 75.0 + 90.0)
        );
        w.set_cell_text(0, 3, 3, "4").unwrap();
        w.commit("edit");
        assert_eq!(val(&w, "G4"), n(120.0));
        assert_eq!(
            val(&w, "A12"),
            n(10.0 + 25.0 + 30.0 + 20.0 + 50.0 + 60.0 + 40.0 + 100.0 + 120.0)
        );
    }

    #[test]
    fn automatic_except_tables_waits_for_calculate_now() {
        let mut w = tables();
        w.set_calc_mode(CalcMode::AutomaticExceptTables);
        w.set_cell_text(0, 0, 5, "25").unwrap();
        w.set_cell_text(0, 0, 1, "100").unwrap();
        w.commit("edit");
        // Formulas follow, the tables do not.
        assert_eq!(val(&w, "B3"), n(300.0));
        assert_eq!(val(&w, "F3"), n(40.0));
        assert_eq!(val(&w, "E8"), n(51.0));
        w.calculate_now();
        assert_eq!(val(&w, "F3"), n(50.0));
        assert_eq!(val(&w, "E8"), n(501.0));
        // Back to automatic, they follow again.
        w.set_calc_mode(CalcMode::Automatic);
        w.set_cell_text(0, 7, 3, "7").unwrap();
        w.commit("edit");
        assert_eq!(val(&w, "E8"), n(701.0));
    }

    #[test]
    fn row_input_alone_and_text_results() {
        let mut w = wb(&[("B1", "2"), ("A4", "=B1&\"!\""), ("B3", "x"), ("C3", "y")]);
        // Trial values across, the formula at the head of the row.
        w.data_table_create(0, Rect::parse("A3:C4").unwrap(), Some((0, 1)), None)
            .unwrap();
        w.commit("Data Table");
        assert_eq!(val(&w, "B4"), Value::text("x!"));
        assert_eq!(val(&w, "C4"), Value::text("y!"));
        assert_eq!(val(&w, "B1"), n(2.0));
        assert_eq!(w.data_table_text(0, 3, 1).as_deref(), Some("{=TABLE(B1,)}"));
    }

    #[test]
    fn data_table_edits_and_refusals() {
        let mut w = tables();
        let whole = Rect::parse("E2:G4").unwrap();
        assert!(w.splits_data_table(0, Rect::parse("E2").unwrap()));
        assert!(w.splits_data_table(0, Rect::parse("A1:F3").unwrap()));
        assert!(!w.splits_data_table(0, whole));
        assert!(!w.splits_data_table(0, Rect::parse("A1:Z50").unwrap()));
        assert!(!w.splits_data_table(0, Rect::parse("A1:C3").unwrap()));
        let range = Rect::parse("D1:G4").unwrap();
        assert_eq!(
            w.data_table_create(0, Rect::parse("D1:D4").unwrap(), None, Some((1, 1))),
            Err(DataTableError::BadSelection)
        );
        assert_eq!(
            w.data_table_create(0, range, None, None),
            Err(DataTableError::NoInput)
        );
        assert_eq!(
            w.data_table_create(0, range, Some((1, 4)), None),
            Err(DataTableError::InputInside)
        );
        assert_eq!(
            w.data_table_create(0, Rect::parse("E2:H6").unwrap(), Some((0, 1)), None),
            Err(DataTableError::Overlap)
        );
        // Clearing the whole table forgets it; undo brings it back.
        w.clear(0, whole, crate::workbook::ClearMode::Contents);
        assert!(w.remove_data_tables_in(0, whole));
        w.commit("Clear");
        assert_eq!(w.sheets[0].data_tables.len(), 1);
        assert_eq!(val(&w, "F3"), Value::Empty);
        w.undo();
        assert_eq!(w.sheets[0].data_tables.len(), 2);
        assert_eq!(val(&w, "F3"), n(40.0));
        // Undoing the creation takes the results away.
        w.undo();
        w.undo();
        assert!(w.sheets[0].data_tables.is_empty());
        assert_eq!(val(&w, "F3"), Value::Empty);
        w.redo();
        assert_eq!(val(&w, "F3"), n(40.0));
    }

    #[test]
    fn data_tables_follow_rows_and_columns() {
        let mut w = tables();
        w.insert_rows(0, 0, 1);
        w.insert_cols(0, 0, 2);
        w.commit("Insert");
        let table = w.sheets[0].data_tables[0];
        assert_eq!(table.range, Rect::parse("G3:I5").unwrap());
        assert_eq!(
            (table.row_input, table.col_input),
            (Some((1, 3)), Some((2, 3)))
        );
        assert_eq!(w.value_at(0, 3, 7), n(40.0));
        // Still live after the move.
        w.set_cell_text(0, 1, 7, "25").unwrap();
        w.commit("edit");
        assert_eq!(w.value_at(0, 3, 7), n(50.0));
        // Without its input cell a table is no table.
        w.delete_rows(0, 2, 1);
        w.commit("Delete");
        assert!(w.sheets[0].data_tables.is_empty());
    }
}
