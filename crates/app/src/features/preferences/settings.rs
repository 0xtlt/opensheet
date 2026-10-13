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
use opensheet_engine::theme::{ThemeColors, ThemeFonts};
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
    // ---- AutoCorrect
    /// "Replace text as you type".
    pub replace_text: bool,
    /// What is replaced by what, in the order of the pane. `None`: the
    /// starting list of the interface language (`autocorrect.rs`), which
    /// the user has not changed.
    pub replacements: Option<Vec<(String, String)>>,
    /// "Correct TWo INitial CApitals".
    pub correct_two_capitals: bool,
    /// "Capitalize first letter of sentences".
    pub capitalize_sentences: bool,
    /// "Capitalize names of days".
    pub capitalize_days: bool,
    /// "Replace internet and network paths with hyperlinks".
    pub link_addresses: bool,
    // ---- Custom Lists
    /// The lists of the user, after the built-in ones. None is empty.
    pub custom_lists: Vec<Vec<String>>,
    // ---- Page Layout ▸ Themes
    /// The colour sets and font pairs made in the Customize dialogs,
    /// listed under "Custom" in the Colors and Fonts menus.
    pub theme_colors: Vec<ThemeColors>,
    pub theme_fonts: Vec<ThemeFonts>,
    // ---- Tables & Filters
    /// "Automatically expand tables".
    pub tables_expand: bool,
    /// "Automatically fill formulas".
    pub tables_fill_formulas: bool,
    // ---- View
    pub show_sheet_tabs: bool,
    /// "In Ribbon, Show: Developer tab".
    pub developer_tab: bool,
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
    // ---- Automate
    /// The folder the Office Scripts are kept in; empty: the folder of
    /// the application (see `features/automate/scripts.rs`).
    pub scripts_folder: String,
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
            replace_text: true,
            replacements: None,
            correct_two_capitals: true,
            capitalize_sentences: true,
            capitalize_days: true,
            link_addresses: true,
            custom_lists: Vec::new(),
            theme_colors: Vec::new(),
            theme_fonts: Vec::new(),
            tables_expand: true,
            tables_fill_formulas: true,
            show_sheet_tabs: true,
            developer_tab: false,
            recalc_before_save: true,
            background_error_checking: true,
            check_error_values: true,
            pens: default_pens(),
            scripts_folder: String::new(),
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
        line("replace-text", flag(self.replace_text));
        if let Some(replacements) = &self.replacements {
            // The line that says the list is the user's, even when empty.
            line("replacements", "own");
            for (from, to) in replacements {
                line("replace", &format!("{from}\t{to}"));
            }
        }
        line("correct-two-capitals", flag(self.correct_two_capitals));
        line("capitalize-sentences", flag(self.capitalize_sentences));
        line("capitalize-days", flag(self.capitalize_days));
        line("link-addresses", flag(self.link_addresses));
        for list in &self.custom_lists {
            line("custom-list", &list.join("\t"));
        }
        for set in &self.theme_colors {
            // Its name, then its twelve colours in the order of a theme part.
            let colors: Vec<String> = set.part_order().iter().map(|rgb| format!("{rgb:06X}")).collect();
            line("theme-colors", &format!("{}\t{}", set.name, colors.join(" ")));
        }
        for pair in &self.theme_fonts {
            line("theme-fonts", &format!("{}\t{}\t{}", pair.name, pair.major, pair.minor));
        }
        line("tables-expand", flag(self.tables_expand));
        line("tables-fill-formulas", flag(self.tables_fill_formulas));
        line("show-sheet-tabs", flag(self.show_sheet_tabs));
        line("developer-tab", flag(self.developer_tab));
        line("recalculate-before-save", flag(self.recalc_before_save));
        line("background-error-checking", flag(self.background_error_checking));
        line("check-error-values", flag(self.check_error_values));
        for pen in &self.pens {
            line("pen", &pen.to_setting());
        }
        line("scripts-folder", &self.scripts_folder);
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
                "replace-text" => set(&mut settings.replace_text),
                "replacements" if value == "own" => {
                    settings.replacements.get_or_insert_with(Vec::new);
                }
                "replace" => {
                    if let Some((from, to)) = value.split_once('\t').filter(|(from, to)| !from.trim().is_empty() && !to.trim().is_empty()) {
                        settings.replacements.get_or_insert_with(Vec::new).push((from.trim().to_string(), to.trim().to_string()));
                    }
                }
                "correct-two-capitals" => set(&mut settings.correct_two_capitals),
                "capitalize-sentences" => set(&mut settings.capitalize_sentences),
                "capitalize-days" => set(&mut settings.capitalize_days),
                "link-addresses" => set(&mut settings.link_addresses),
                "custom-list" => {
                    let list = clean_list(value.split('\t'));
                    if !list.is_empty() {
                        settings.custom_lists.push(list);
                    }
                }
                "theme-colors" => settings.theme_colors.extend(theme_colors_of(value)),
                "theme-fonts" => {
                    let mut parts = value.split('\t').map(str::trim);
                    if let (Some(name), Some(major), Some(minor), None) = (parts.next(), parts.next(), parts.next(), parts.next()) {
                        if ![name, major, minor].iter().any(|part| part.is_empty()) {
                            settings.theme_fonts.push(ThemeFonts::new(name, major, minor));
                        }
                    }
                }
                "tables-expand" => set(&mut settings.tables_expand),
                "tables-fill-formulas" => set(&mut settings.tables_fill_formulas),
                "show-sheet-tabs" => set(&mut settings.show_sheet_tabs),
                "developer-tab" => set(&mut settings.developer_tab),
                "recalculate-before-save" => set(&mut settings.recalc_before_save),
                "background-error-checking" => set(&mut settings.background_error_checking),
                "check-error-values" => set(&mut settings.check_error_values),
                "pen" => pens.extend(Pen::from_setting(value)),
                "scripts-folder" => settings.scripts_folder = value.to_string(),
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

/// A colour set as its line of the settings file: its name, a tab, its
/// twelve colours in hexadecimal.
fn theme_colors_of(value: &str) -> Option<ThemeColors> {
    let (name, colors) = value.split_once('\t')?;
    let colors: Vec<u32> = colors.split_whitespace().filter_map(|rgb| u32::from_str_radix(rgb, 16).ok()).filter(|rgb| *rgb <= 0xFF_FFFF).collect();
    let colors: [u32; 12] = colors.try_into().ok()?;
    Some(ThemeColors::from_part_order(name.trim(), colors)).filter(|set| !set.name.is_empty())
}

/// The entries of a custom list as they are kept: trimmed, none empty,
/// none twice (whatever the capitals), as Excel refuses them.
pub fn clean_list<'a>(entries: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut list: Vec<String> = Vec::new();
    for entry in entries.map(str::trim).filter(|entry| !entry.is_empty()) {
        if !list.iter().any(|known| known.to_lowercase() == entry.to_lowercase()) {
            list.push(entry.to_string());
        }
    }
    list
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
        settings.scripts_folder = "/Users/ada/My Scripts = here".to_string();
        settings.spelling_language = "en_GB".to_string();
        settings.translate_from = "fr".to_string();
        settings.translate_to = "pt-BR".to_string();
        settings.replace_text = false;
        settings.replacements = Some(vec![("(c)".to_string(), "©".to_string()), ("teh".to_string(), "the = it".to_string())]);
        settings.correct_two_capitals = false;
        settings.capitalize_sentences = false;
        settings.capitalize_days = false;
        settings.link_addresses = false;
        settings.custom_lists = vec![vec!["North".to_string(), "East".to_string()], vec!["a b".to_string(), "c, d".to_string(), "e".to_string()]];
        settings.tables_expand = false;
        settings.tables_fill_formulas = false;
        let mut colors = opensheet_engine::theme::Theme::office().colors;
        colors.name = "Company = ours".to_string();
        colors.set(opensheet_engine::style::ThemeSlot::Accent1, 0x123ABC);
        settings.theme_colors = vec![colors];
        settings.theme_fonts = vec![ThemeFonts::new("House", "Helvetica Neue", "Georgia")];
        assert_eq!(Settings::from_text(&settings.to_text()), settings);
        // An empty or damaged file gives the defaults, setting by setting.
        assert_eq!(Settings::from_text(""), Settings::default());
        let damaged = "pen = nonsense\nwhat\n= 3\nsheets-in-new-workbook = 0\ndecimal-places = 1000\nmove-direction = sideways\nfixed-decimal = maybe\n";
        assert_eq!(Settings::from_text(damaged), Settings::default());
        // A list the user emptied stays empty: it is not the starting list.
        let emptied = Settings {
            replacements: Some(Vec::new()),
            ..Settings::default()
        };
        assert_eq!(Settings::from_text(&emptied.to_text()), emptied);
        // A colour set needs its twelve colours, a font pair its two fonts.
        let broken = Settings::from_text("theme-colors = A\t000000 FFFFFF\ntheme-fonts = B\tArial\ntheme-colors = \t\n");
        assert!(broken.theme_colors.is_empty() && broken.theme_fonts.is_empty());
        // Entries of a list are trimmed; empty ones and repeats are dropped.
        assert_eq!(Settings::from_text("custom-list = a\t\t A \tb \n").custom_lists, vec![vec!["a".to_string(), "b".to_string()]]);
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
