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
use std::cell::RefCell;
use std::path::PathBuf;

/// Everything the application remembers between launches, apart from the
/// language (`i18n.rs`).
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// The gallery of pens of the Draw tab, in order. Never empty.
    pub pens: Vec<Pen>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings { pens: default_pens() }
    }
}

impl Settings {
    /// The settings as the text of their file.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for pen in &self.pens {
            out.push_str(&format!("pen = {}\n", pen.to_setting()));
        }
        out
    }

    /// Read the text of a settings file. Lines that make no sense are
    /// skipped: each setting then keeps its default.
    pub fn from_text(text: &str) -> Settings {
        let mut settings = Settings::default();
        let mut pens = Vec::new();
        for line in text.lines() {
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            if name.trim() == "pen" {
                pens.extend(Pen::from_setting(value));
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
        assert_eq!(Settings::from_text(&settings.to_text()), settings);
        // An empty or damaged file gives the defaults.
        assert_eq!(Settings::from_text(""), Settings::default());
        assert_eq!(Settings::from_text("pen = nonsense\nwhat\n= 3\n"), Settings::default());
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
