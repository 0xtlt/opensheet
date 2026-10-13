//! Preferences (OpenSheet ▸ Preferences…, ⌘,): the settings of the
//! application and of the workbook that have no place on the ribbon, laid
//! out as Excel for Mac's window: a page of panes, each opening on its
//! settings, "Show All" coming back.
//!
//! * `settings.rs`: what the application keeps between launches.
//! * `window.rs`: the window.
//!
//! Where a setting lives follows Excel: the calculation mode and iterative
//! calculation belong to the workbook (and its file), gridlines, headings
//! and formulas to the sheet shown, the rest to the application.
//!
//! What the settings change is asked of this file by the code that does
//! it: [`enter_direction`] by the Return key, [`prepare_new_workbook`] by
//! File ▸ New, [`before_save`] by Save, [`user_name`] by the comments,
//! [`shows_sheet_tabs`] by the window, [`checks_error_values`] by Error
//! Checking. The rest is done here through the feature hooks: the decimal
//! point put into typed numbers and the green corner of cells in error.

pub mod autocorrect;
mod pane_autocorrect;
mod pane_lists;
pub mod settings;
#[cfg(test)]
mod ui_tests;
mod window;

use super::{Command, Cx, Feature, PaintLayer, Painter, PendingEdit, Verdict};
use crate::app::ExcelApp;
use crate::i18n::t;
use crate::theme::c;
use gpui::{App, Context, KeyBinding, Stateful, Window, actions, prelude::*};
use opensheet_engine::ops::Direction;
use opensheet_engine::workbook::Content;
use opensheet_engine::{CalcMode, Style, Workbook};

pub struct Preferences;

commands! {
    /// What the user can ask of the preferences.
    pub enum Cmd {
        Open = "", t("Preferences…", "Préférences…");
    }
}

impl Command for Cmd {
    fn label(self) -> &'static str {
        self.face().1
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        if app.is_editing() && !app.commit_edit(None, window, cx) {
            return;
        }
        window::open(app, window, cx);
    }
}

actions!(preferences, [OpenPreferences]);

/// ⌘, opens the preferences, as in every Mac application.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("cmd-,", OpenPreferences, Some("Workbook"))]);
}

/// Make the window answer the shortcut and the menu item.
pub fn key_actions(root: Stateful<gpui::Div>, cx: &Context<ExcelApp>) -> Stateful<gpui::Div> {
    root.on_action(cx.listener(|app, _: &OpenPreferences, window, cx| app.run_command(Cmd::Open, window, cx)))
}

// ------------------------------------------------- what the settings change

/// Where Return takes the selection (`shift`: the other way); `None` when
/// the selection stays on the cell.
pub fn enter_direction(shift: bool) -> Option<Direction> {
    let settings = settings::get();
    if !settings.move_after_return {
        return None;
    }
    Some(match (settings.move_direction, shift) {
        (direction, false) => direction,
        (Direction::Down, true) => Direction::Up,
        (Direction::Up, true) => Direction::Down,
        (Direction::Right, true) => Direction::Left,
        (Direction::Left, true) => Direction::Right,
    })
}

/// Give a workbook File ▸ New just made the sheets and the font of the
/// General pane.
pub fn prepare_new_workbook(wb: &mut Workbook) {
    let settings = settings::get();
    let mut style: Style = wb.styles.default_style().clone();
    if !settings.default_font.is_empty() {
        style.font.set_name(settings.default_font.as_str());
    }
    style.font.set_size_pt(settings.default_font_size);
    if style != *wb.styles.default_style() {
        wb.styles.set_default(style);
    }
    for index in 1..settings.sheets_in_new_workbook.clamp(1, 255) as usize {
        wb.add_sheet(index);
    }
    // A new workbook has nothing to undo and nothing to save.
    wb.commit("");
    wb.clear_history();
    wb.modified = false;
}

/// "Recalculate workbook before saving": formulas that wait in Manual
/// mode are calculated before the file is written.
pub fn before_save(wb: &mut Workbook) {
    if settings::get().recalc_before_save && wb.calc_mode() == CalcMode::Manual && wb.needs_calculation() {
        wb.calculate_now();
    }
}

/// The user name of the General pane; `None` when it was left empty.
pub fn user_name() -> Option<String> {
    Some(settings::get().user_name.trim().to_string()).filter(|name| !name.is_empty())
}

pub fn shows_sheet_tabs() -> bool {
    settings::get().show_sheet_tabs
}

/// "Replace internet and network paths with hyperlinks": whether an
/// address typed into a cell becomes a link.
pub fn links_typed_addresses() -> bool {
    settings::get().link_addresses
}

/// Give a workbook what the application decides for every workbook: the
/// custom lists of the user and what tables do by themselves. Called
/// when a workbook comes into the window and when those settings change.
pub fn apply_to_workbook(wb: &mut Workbook) {
    let settings = settings::get();
    wb.user_lists = settings.custom_lists;
    wb.table_auto = opensheet_engine::table::TableAuto {
        expand: settings.tables_expand,
        fill_formulas: settings.tables_fill_formulas,
    };
}

/// Whether Error Checking looks for formulas that result in an error.
pub fn checks_error_values() -> bool {
    settings::get().check_error_values
}

/// What "Automatically insert a decimal point" makes of an entry: digits
/// typed without a decimal point get one `places` from their right (zeros
/// are added on the right for negative `places`). `None` when the entry
/// is left as it is: not a plain whole number.
pub(crate) fn with_fixed_decimal(text: &str, places: i32, decimal_sep: char) -> Option<String> {
    let typed = text.trim();
    let (sign, digits) = match typed.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", typed.strip_prefix('+').unwrap_or(typed)),
    };
    if places == 0 || digits.is_empty() || digits.len() > 15 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if places < 0 {
        return Some(format!("{sign}{digits}{}", "0".repeat(places.unsigned_abs() as usize)));
    }
    let places = places as usize;
    // "5" with two places is 0.05.
    let padded = format!("{digits:0>width$}", width = places + 1);
    let (whole, fraction) = padded.split_at(padded.len() - places);
    Some(format!("{sign}{whole}{decimal_sep}{fraction}"))
}

impl Feature for Preferences {
    fn name(&self) -> &'static str {
        "preferences"
    }

    /// "Automatically insert a decimal point": the entry is rewritten
    /// before it is stored.
    fn may_commit(&self, app: &mut ExcelApp, edit: &PendingEdit, _window: &mut Window, cx: &mut Cx) -> Verdict {
        let settings = settings::get();
        if settings.fixed_decimal {
            if let Some(text) = with_fixed_decimal(edit.text, settings.decimal_places, app.wb.locale.decimal_sep) {
                app.editor().clone().update(cx, |input, cx| input.set_text(text, cx));
            }
        }
        Verdict::Accept
    }

    /// AutoCorrect looks at the last word of a confirmed entry.
    fn cells_changed(&self, app: &mut ExcelApp, edit: &super::CellEdit, cx: &mut Cx) {
        autocorrect::cells_changed(app, edit, cx);
    }

    /// Background error checking: a green corner on the cells whose
    /// formula results in an error.
    fn paint(&self, p: &mut Painter) {
        if p.layer != PaintLayer::Over {
            return;
        }
        let settings = settings::get();
        if !settings.background_error_checking || !settings.check_error_values {
            return;
        }
        let sheet = &p.app.wb.sheets[p.app.sheet];
        let visible = p.visible;
        let size = (6.0 * p.zoom()).clamp(4.0, 9.0);
        let mut corners = Vec::new();
        for ((row, col), cell) in sheet.cells.range((visible.r0, 0)..=(visible.r1, u32::MAX)) {
            if *col < visible.c0 || *col > visible.c1 {
                continue;
            }
            if matches!(cell.content, Content::Formula(_)) && cell.value.is_error() {
                let (x0, y0, x1, y1) = p.cell_box(opensheet_engine::Rect::cell(*row, *col));
                // A hidden cell has no corner to mark.
                if x1 - x0 > size && y1 - y0 > size {
                    corners.push((x0, y0));
                }
            }
        }
        for (x0, y0) in corners {
            p.polygon(&[(x0, y0), (x0 + size, y0), (x0, y0 + size)], c(0x107C41));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::Locale;

    #[test]
    fn a_decimal_point_is_put_into_whole_numbers() {
        assert_eq!(with_fixed_decimal("1234", 2, '.').as_deref(), Some("12.34"));
        assert_eq!(with_fixed_decimal("5", 2, ',').as_deref(), Some("0,05"));
        assert_eq!(with_fixed_decimal(" -250 ", 2, '.').as_deref(), Some("-2.50"));
        assert_eq!(with_fixed_decimal("12", -3, '.').as_deref(), Some("12000"));
        // A point that was typed overrides; text, formulas and dates are
        // left alone.
        for typed in ["12.5", "12,5", "abc", "=12", "1e3", "12/3", "", "-", "12 "] {
            let expected = if typed == "12 " { Some("0.12".to_string()) } else { None };
            assert_eq!(with_fixed_decimal(typed, 2, '.'), expected, "{typed:?}");
        }
        assert_eq!(with_fixed_decimal("12", 0, '.'), None);
    }

    #[test]
    fn return_follows_the_edit_pane() {
        assert_eq!((enter_direction(false), enter_direction(true)), (Some(Direction::Down), Some(Direction::Up)));
        settings::update(|settings| settings.move_direction = Direction::Right);
        assert_eq!((enter_direction(false), enter_direction(true)), (Some(Direction::Right), Some(Direction::Left)));
        settings::update(|settings| settings.move_after_return = false);
        assert_eq!((enter_direction(false), enter_direction(true)), (None, None));
    }

    #[test]
    fn a_new_workbook_takes_the_general_pane() {
        let mut plain = Workbook::new(Locale::en());
        prepare_new_workbook(&mut plain);
        assert_eq!(plain.sheets.len(), 1);
        assert!(!plain.modified && plain.undo_label().is_none());
        settings::update(|settings| {
            settings.sheets_in_new_workbook = 3;
            settings.default_font = "Helvetica".to_string();
            settings.default_font_size = 12.0;
        });
        let mut wb = Workbook::new(Locale::en());
        prepare_new_workbook(&mut wb);
        assert_eq!(wb.sheets.len(), 3);
        let font = &wb.styles.default_style().font;
        assert_eq!((&*font.name, font.size_pt()), ("Helvetica", 12.0));
        // Nothing to undo, nothing to save.
        assert!(!wb.modified && wb.undo_label().is_none());
        assert!(!wb.undo());
    }

    #[test]
    fn saving_calculates_what_waits_in_manual_mode() {
        let mut wb = Workbook::new(Locale::en());
        wb.set_cell_text(0, 0, 0, "1").unwrap();
        wb.set_cell_text(0, 0, 1, "=A1+1").unwrap();
        wb.commit("setup");
        wb.set_calc_mode(CalcMode::Manual);
        wb.set_cell_text(0, 0, 0, "10").unwrap();
        wb.commit("edit");
        assert_eq!(wb.value_at(0, 0, 1), opensheet_engine::Value::Number(2.0));
        settings::update(|settings| settings.recalc_before_save = false);
        before_save(&mut wb);
        assert_eq!(wb.value_at(0, 0, 1), opensheet_engine::Value::Number(2.0));
        settings::update(|settings| settings.recalc_before_save = true);
        before_save(&mut wb);
        assert_eq!(wb.value_at(0, 0, 1), opensheet_engine::Value::Number(11.0));
    }
}
