//! Where the application keeps what lasts between launches (settings,
//! scripts): the folder each system gives an application for
//! its own data.

use std::path::PathBuf;

/// `~/Library/Application Support/OpenSheet` on macOS, `%APPDATA%\OpenSheet`
/// on Windows, `$XDG_CONFIG_HOME/opensheet` (or `~/.config/opensheet`)
/// elsewhere. `None` when the system does not say where the user's home is.
pub fn app_data() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty()).map(PathBuf::from);
    if cfg!(target_os = "macos") {
        var("HOME").map(|home| home.join("Library/Application Support/OpenSheet"))
    } else if cfg!(target_os = "windows") {
        var("APPDATA").map(|dir| dir.join("OpenSheet"))
    } else {
        var("XDG_CONFIG_HOME")
            .or_else(|| var("HOME").map(|home| home.join(".config")))
            .map(|dir| dir.join("opensheet"))
    }
}
