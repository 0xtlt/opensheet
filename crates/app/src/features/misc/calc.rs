//! The Calculation group: Calculation Options, Calculate Now and
//! Calculate Sheet.
//!
//! ==========================================================================
//! ENGINE ADAPTER — the only place that knows how the engine calculates.
//!
//! The calculation mode is being added to the engine on another branch.
//! Until it is merged, the workbook recalculates at every change
//! (`Workbook::commit`), so the mode chosen in Calculation Options is only
//! remembered here and shown ticked in the menu: Manual does not hold
//! calculation back yet. Point the four functions below at the engine API
//! when it lands (the mode then belongs to the workbook, and `CalcState`
//! goes away); nothing else in the application looks at the mode.
//! ==========================================================================

use crate::app::ExcelApp;

/// The choices of Calculation Options.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CalcMode {
    #[default]
    Automatic,
    AutomaticExceptTables,
    Manual,
}

/// Where the mode lives until the engine has it.
#[derive(Default)]
struct CalcState {
    mode: CalcMode,
}

/// ADAPTER: the calculation mode of the workbook.
pub fn calc_mode(app: &ExcelApp) -> CalcMode {
    app.ext::<CalcState>().map(|state| state.mode).unwrap_or_default()
}

/// ADAPTER: choose the calculation mode.
pub fn set_calc_mode(app: &mut ExcelApp, mode: CalcMode) {
    app.ext_mut::<CalcState>().mode = mode;
}

/// ADAPTER: Calculate Now (F9), the whole workbook.
pub fn calculate_now(app: &mut ExcelApp) {
    app.wb.recalc();
}

/// ADAPTER: Calculate Sheet (Shift+F9). The engine only knows how to
/// calculate the whole workbook for now, which gives the sheet its values
/// all the same.
pub fn calculate_sheet(app: &mut ExcelApp, _sheet: usize) {
    app.wb.recalc();
}
