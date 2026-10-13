//! Interface language.
//!
//! Strings are written inline as (English, French) pairs — `t("Home",
//! "Accueil")` — and the other languages are looked up by their English
//! text in `i18n_tables.rs`. A string missing from a table falls back to
//! English.

use crate::i18n_tables;
use opensheet_engine::Locale;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    En,
    Fr,
    Es,
    De,
    It,
}

impl Lang {
    pub const ALL: [Lang; 5] = [Lang::En, Lang::Fr, Lang::Es, Lang::De, Lang::It];

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Fr => "fr",
            Lang::Es => "es",
            Lang::De => "de",
            Lang::It => "it",
        }
    }

    /// Language for a tag such as "fr", "de-DE" or "es_ES.UTF-8".
    pub fn from_code(code: &str) -> Option<Lang> {
        let lower = code.trim().to_lowercase();
        Lang::ALL.into_iter().find(|l| lower.starts_with(l.code()))
    }

    /// The matching calculation locale (formula names, separators, formats).
    pub fn locale(self) -> &'static Locale {
        Locale::by_id(self.code())
    }

    pub fn display_name(self) -> &'static str {
        self.locale().display_name
    }
}

#[cfg(not(test))]
static LANG: AtomicU8 = AtomicU8::new(0);

// Tests run side by side, each on its own thread: the language one of
// them chooses must not change the words the others compare.
#[cfg(test)]
thread_local! {
    static LANG: AtomicU8 = const { AtomicU8::new(0) };
}

#[cfg(not(test))]
fn current() -> &'static AtomicU8 {
    &LANG
}

pub fn set_lang(lang: Lang) {
    #[cfg(not(test))]
    current().store(lang as u8, Ordering::Relaxed);
    #[cfg(test)]
    LANG.with(|current| current.store(lang as u8, Ordering::Relaxed));
}

pub fn lang() -> Lang {
    #[cfg(not(test))]
    let index = current().load(Ordering::Relaxed);
    #[cfg(test)]
    let index = LANG.with(|current| current.load(Ordering::Relaxed));
    Lang::ALL[index as usize % Lang::ALL.len()]
}

fn table(lang: Lang) -> &'static HashMap<&'static str, &'static str> {
    static TABLES: OnceLock<[HashMap<&'static str, &'static str>; 3]> = OnceLock::new();
    let tables = TABLES.get_or_init(|| {
        [i18n_tables::ES, i18n_tables::DE, i18n_tables::IT].map(|pairs| pairs.iter().copied().collect())
    });
    match lang {
        Lang::Es => &tables[0],
        Lang::De => &tables[1],
        _ => &tables[2],
    }
}

/// The interface string in the current language.
pub fn t(en: &'static str, fr: &'static str) -> &'static str {
    match lang() {
        Lang::En => en,
        Lang::Fr => fr,
        other => table(other).get(en).copied().unwrap_or(en),
    }
}

/// Like `t`, for an English word that needs different translations in
/// different places ("Edit" the menu vs. "Edit" the cell mode). The tables
/// may hold a `"context|English"` entry; without one the plain entry is used.
pub fn t_ctx(context: &'static str, en: &'static str, fr: &'static str) -> &'static str {
    match lang() {
        Lang::En => en,
        Lang::Fr => fr,
        other => {
            let key = format!("{context}|{en}");
            let table = table(other);
            table.get(key.as_str()).or_else(|| table.get(en)).copied().unwrap_or(en)
        }
    }
}

/// "Label: value", with the spacing of the current language (French puts a
/// space before the colon).
pub fn labelled(label: &str, value: impl std::fmt::Display) -> String {
    match lang() {
        Lang::Fr => format!("{label} : {value}"),
        _ => format!("{label}: {value}"),
    }
}

fn preference_file() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Application Support/OpenSheet/language"))
}

/// Remember the language chosen in the app.
pub fn save_preference(lang: Lang) {
    if let Some(path) = preference_file() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, lang.code());
    }
}

/// Language to start in: `OPENSHEET_LANG`, else the one chosen in the app,
/// else the first supported system language, else English.
pub fn detect() -> Lang {
    if let Some(lang) = std::env::var("OPENSHEET_LANG").ok().and_then(|v| Lang::from_code(&v)) {
        return lang;
    }
    if let Some(lang) = preference_file()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| Lang::from_code(&s))
    {
        return lang;
    }
    // macOS: preferred languages, in order.
    if let Ok(out) = std::process::Command::new("defaults")
        .args(["read", "-g", "AppleLanguages"])
        .output()
    {
        let text = String::from_utf8_lossy(&out.stdout);
        for tag in text.split('"').skip(1).step_by(2) {
            if let Some(lang) = Lang::from_code(tag) {
                return lang;
            }
        }
    }
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(lang) = std::env::var(var).ok().and_then(|v| Lang::from_code(&v)) {
            return lang;
        }
    }
    Lang::En
}
