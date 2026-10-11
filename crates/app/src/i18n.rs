//! Interface language. Strings are written inline as (English, French)
//! pairs: `t("Home", "Accueil")`.

use std::sync::atomic::{AtomicBool, Ordering};

static FRENCH: AtomicBool = AtomicBool::new(false);

pub fn set_french(on: bool) {
    FRENCH.store(on, Ordering::Relaxed);
}

pub fn is_french() -> bool {
    FRENCH.load(Ordering::Relaxed)
}

pub fn t(en: &'static str, fr: &'static str) -> &'static str {
    if is_french() { fr } else { en }
}

/// True when the system (or `OPENSHEET_LANG`) asks for French.
pub fn system_prefers_french() -> bool {
    if let Ok(v) = std::env::var("OPENSHEET_LANG") {
        return v.to_lowercase().starts_with("fr");
    }
    // macOS: the first preferred language.
    if let Ok(out) = std::process::Command::new("defaults")
        .args(["read", "-g", "AppleLanguages"])
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout);
        if let Some(first) = s.split('"').nth(1) {
            return first.to_lowercase().starts_with("fr");
        }
    }
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() {
                return v.to_lowercase().starts_with("fr");
            }
        }
    }
    false
}
