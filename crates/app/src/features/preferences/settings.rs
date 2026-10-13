//! The settings of the application: what Preferences changes and what is
//! not kept in a workbook (the gallery of pens of the Draw tab). They are
//! read once, when first asked for, and written at every change, to a text
//! file of `name = value` lines next to the language choice
//! (`~/Library/Application Support/OpenSheet/settings`; the environment
//! variable `OPENSHEET_SETTINGS` names another file).
//!
//! Tests never touch the file: each test thread has the default settings
//! to itself.

use crate::features::ink::{Pen, default_pens};
use opensheet_engine::ops::Direction;
use std::cell::RefCell;
use std::path::PathBuf;

/// Everything the application remembers between launches, apart from the
/// language (`i18n.rs`).
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    // ---- General
    /// The name put on comments; empty: the name the system gives.
    pub user_name: String,
    /// Font of the cells of a new workbook; empty: the engine's own.
    pub default_font: String,
    /// Its size in points.
    pub default_font_size: f32,
    pub sheets_in_new_workbook: u32,
    // ---- Edit
    /// "After pressing Return, move selection".
    pub move_after_return: bool,
    pub move_direction: Direction,
    /// "Automatically insert a decimal point".
    pub fixed_decimal: bool,
    /// How many places from the right the point goes (negative: zeros are
    /// added instead).
    pub decimal_places: i32,
    // ---- View
    pub show_sheet_tabs: bool,
    // ---- Calculation
    pub recalc_before_save: bool,
    // ---- Error Checking
    /// "Enable background error checking": the green corner on cells.
    pub background_error_checking: bool,
    /// The rule "Cells containing formulas that result in an error".
    pub check_error_values: bool,
    // ---- Draw
    /// The gallery of pens of the Draw tab, in order. Never empty.
    pub pens: Vec<Pen>,
    // ---- Review
    /// "Dictionary language" of the Spelling dialog, as the system
    /// checker names it; empty: the language of the interface.
    pub spelling_language: String,
    /// The languages last chosen in the Translator pane ("" for the
    /// source: detect it).
    pub translate_from: String,
    pub translate_to: String,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            user_name: String::new(),
            default_font: String::new(),
            default_font_size: 11.0,
            sheets_in_new_workbook: 1,
            move_after_return: true,
            move_direction: Direction::Down,
            fixed_decimal: false,
            decimal_places: 2,
            show_sheet_tabs: true,
            recalc_before_save: true,
            background_error_checking: true,
            check_error_values: true,
            pens: default_pens(),
            spelling_language: String::new(),
            translate_from: String::new(),
            translate_to: String::new(),
        }
    }
}

fn direction_name(direction: Direction) -> &'static str {
    match direction {
        Direction::Down => "down",
        Direction::Right => "right",
        Direction::Up => "up",
        Direction::Left => "left",
    }
}

fn direction_named(name: &str) -> Option<Direction> {
    [Direction::Down, Direction::Right, Direction::Up, Direction::Left]
        .into_iter()
        .find(|direction| direction_name(*direction) == name)
}

impl Settings {
    /// The settings as the text of their file.
    pub fn to_text(&self) -> String {
        let flag = |on: bool| if on { "yes" } else { "no" };
        let mut out = String::new();
        let mut line = |name: &str, value: &str| out.push_str(&format!("{name} = {value}\n"));
        line("user-name", &self.user_name);
        line("default-font", &self.default_font);
        line("default-font-size", &self.default_font_size.to_string());
        line("sheets-in-new-workbook", &self.sheets_in_new_workbook.to_string());
        line("move-after-return", flag(self.move_after_return));
        line("move-direction", direction_name(self.move_direction));
        line("fixed-decimal", flag(self.fixed_decimal));
        line("decimal-places", &self.decimal_places.to_string());
        line("show-sheet-tabs", flag(self.show_sheet_tabs));
        line("recalculate-before-save", flag(self.recalc_before_save));
        line("background-error-checking", flag(self.background_error_checking));
        line("check-error-values", flag(self.check_error_values));
        for pen in &self.pens {
            line("pen", &pen.to_setting());
        }
        line("spelling-language", &self.spelling_language);
        line("translate-from", &self.translate_from);
        line("translate-to", &self.translate_to);
        out
    }

    /// Read the text of a settings file. Lines that make no sense are
    /// skipped: each setting then keeps its default.
    pub fn from_text(text: &str) -> Settings {
        let mut settings = Settings::default();
        let mut pens = Vec::new();
        let flag = |value: &str| match value {
            "yes" => Some(true),
            "no" => Some(false),
            _ => None,
        };
        for line in text.lines() {
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            let set = |slot: &mut bool| {
                if let Some(on) = flag(value) {
                    *slot = on;
                }
            };
            match name.trim() {
                "user-name" => settings.user_name = value.to_string(),
                "default-font" => settings.default_font = value.to_string(),
                "default-font-size" => {
                    if let Some(size) = value.parse::<f32>().ok().filter(|size| (1.0..=409.0).contains(size)) {
                        settings.default_font_size = size;
                    }
                }
                "sheets-in-new-workbook" => {
                    if let Some(count) = value.parse::<u32>().ok().filter(|count| (1..=255).contains(count)) {
                        settings.sheets_in_new_workbook = count;
                    }
                }
                "move-after-return" => set(&mut settings.move_after_return),
                "move-direction" => settings.move_direction = direction_named(value).unwrap_or(settings.move_direction),
                "fixed-decimal" => set(&mut settings.fixed_decimal),
                "decimal-places" => {
                    if let Some(places) = value.parse::<i32>().ok().filter(|places| (-300..=300).contains(places)) {
                        settings.decimal_places = places;
                    }
                }
                "show-sheet-tabs" => set(&mut settings.show_sheet_tabs),
                "recalculate-before-save" => set(&mut settings.recalc_before_save),
                "background-error-checking" => set(&mut settings.background_error_checking),
                "check-error-values" => set(&mut settings.check_error_values),
                "pen" => pens.extend(Pen::from_setting(value)),
                "spelling-language" => settings.spelling_language = value.to_string(),
                "translate-from" => settings.translate_from = value.to_string(),
                "translate-to" => settings.translate_to = value.to_string(),
                _ => {}
            }
        }
        if !pens.is_empty() {
            settings.pens = pens;
        }
        settings
    }
}

fn file() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("OPENSHEET_SETTINGS") {
        return Some(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/OpenSheet/settings"))
}

fn load() -> Settings {
    if cfg!(test) {
        return Settings::default();
    }
    file()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map_or_else(Settings::default, |text| Settings::from_text(&text))
}

fn save(settings: &Settings) {
    if cfg!(test) {
        return;
    }
    if let Some(path) = file() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, settings.to_text());
    }
}

thread_local! {
    static SETTINGS: RefCell<Option<Settings>> = const { RefCell::new(None) };
}

/// The settings as they are now.
pub fn get() -> Settings {
    SETTINGS.with(|settings| settings.borrow_mut().get_or_insert_with(load).clone())
}

/// Change the settings and keep them for the next launch.
pub fn update(change: impl FnOnce(&mut Settings)) {
    SETTINGS.with(|settings| {
        let mut settings = settings.borrow_mut();
        let settings = settings.get_or_insert_with(load);
        let before = settings.clone();
        change(settings);
        if settings.pens.is_empty() {
            settings.pens = default_pens();
        }
        if *settings != before {
            save(settings);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::ink::PenKind;

    #[test]
    fn settings_survive_their_file() {
        let mut settings = Settings::default();
        settings.pens.push(Pen {
            kind: PenKind::Highlighter,
            color: 0x00F900,
            mm: 8.0,
        });
        settings.user_name = "Ada = Lovelace".to_string();
        settings.default_font = "Helvetica Neue".to_string();
        settings.default_font_size = 12.5;
        settings.sheets_in_new_workbook = 3;
        settings.move_after_return = false;
        settings.move_direction = Direction::Right;
        settings.fixed_decimal = true;
        settings.decimal_places = -2;
        settings.show_sheet_tabs = false;
        settings.recalc_before_save = false;
        settings.background_error_checking = false;
        settings.check_error_values = false;
        settings.spelling_language = "en_GB".to_string();
        settings.translate_from = "fr".to_string();
        settings.translate_to = "pt-BR".to_string();
        assert_eq!(Settings::from_text(&settings.to_text()), settings);
        // An empty or damaged file gives the defaults, setting by setting.
        assert_eq!(Settings::from_text(""), Settings::default());
        let damaged = "pen = nonsense\nwhat\n= 3\nsheets-in-new-workbook = 0\ndecimal-places = 1000\nmove-direction = sideways\nfixed-decimal = maybe\n";
        assert_eq!(Settings::from_text(damaged), Settings::default());
    }

    #[test]
    fn changes_are_seen_at_once() {
        update(|settings| settings.pens.truncate(1));
        assert_eq!(get().pens.len(), 1);
        // A gallery cannot be emptied.
        update(|settings| settings.pens.clear());
        assert_eq!(get().pens, default_pens());
    }
}
