//! Interface language.
//!
//! Strings are written inline as (English, French) pairs — `t("Home",
//! "Accueil")` — and the other languages are looked up by their English
//! text in `i18n_tables.rs`. A string missing from a table falls back to
//! English.

use crate::i18n_tables;
use opensheet_engine::Locale;
use std::collections::HashMap;
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

    /// Language for a tag such as "fr", "de-CH", "es_ES.UTF-8" or
    /// "zh-Hans-CN" (none): its first subtag, whatever the separators of
    /// the system (BCP 47 for macOS and Windows, POSIX for Linux).
    pub fn from_code(code: &str) -> Option<Lang> {
        let primary = code.trim().split(['-', '_', '.', '@']).next()?.to_lowercase();
        Lang::ALL.into_iter().find(|l| primary == l.code())
    }

    /// The matching calculation locale (formula names, separators, formats).
    pub fn locale(self) -> &'static Locale {
        Locale::by_id(self.code())
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

/// The first language of a list in order of preference that the interface
/// speaks.
fn first_supported<S: AsRef<str>>(tags: impl IntoIterator<Item = S>) -> Option<Lang> {
    tags.into_iter().find_map(|tag| Lang::from_code(tag.as_ref()))
}

/// Language to start in: `OPENSHEET_LANG`, else the first supported one of
/// the system's preferred languages, else English. On macOS the list is the
/// one System Settings ▸ General ▸ Language & Region gives, OpenSheet's own
/// language (Applications) first when one is set; on Windows the user's
/// interface languages; elsewhere `LANGUAGE`, `LC_ALL`, `LC_MESSAGES` and
/// `LANG`.
pub fn detect() -> Lang {
    if let Some(lang) = std::env::var("OPENSHEET_LANG").ok().and_then(|v| Lang::from_code(&v)) {
        return lang;
    }
    first_supported(sys_locale::get_locales()).unwrap_or(Lang::En)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_of_every_system_name_their_language() {
        assert_eq!(Lang::from_code("fr"), Some(Lang::Fr));
        assert_eq!(Lang::from_code("fr-FR"), Some(Lang::Fr));
        assert_eq!(Lang::from_code("fr_FR.UTF-8"), Some(Lang::Fr));
        assert_eq!(Lang::from_code("de-CH"), Some(Lang::De));
        assert_eq!(Lang::from_code("es-419"), Some(Lang::Es));
        assert_eq!(Lang::from_code("it_IT@euro"), Some(Lang::It));
        assert_eq!(Lang::from_code(" EN-gb "), Some(Lang::En));
        assert_eq!(Lang::from_code("zh-Hans-CN"), None);
        // Languages whose code begins like a supported one.
        assert_eq!(Lang::from_code("frr-DE"), None);
        assert_eq!(Lang::from_code("dsb"), None);
        assert_eq!(Lang::from_code("C"), None);
        assert_eq!(Lang::from_code(""), None);
    }

    #[test]
    fn the_first_supported_preferred_language_wins() {
        assert_eq!(first_supported(["zh-Hans-CN", "de-CH", "fr-FR"]), Some(Lang::De));
        assert_eq!(first_supported(["ja-JP", "pt-BR"]), None);
        assert_eq!(first_supported(Vec::<String>::new()), None);
        assert_eq!(first_supported(["en-US", "fr-FR"]), Some(Lang::En));
    }
}
