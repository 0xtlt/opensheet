//! Where the scripts live: one `.osts` file per script (the format Excel
//! keeps its Office Scripts in) in a folder of the user.
//!
//! The folder is the one named in Preferences, or by default a folder of
//! the application's own (`~/Library/Application Support/OpenSheet/
//! Scripts`): macOS lets an application use that one without asking,
//! which it would not for `~/Documents`.
//!
//! Tests never see the user's folder: each test thread has a temporary
//! folder to itself.

use crate::features::preferences::settings;
use crate::i18n::t;
use opensheet_script::Script;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The extension of a script file.
pub(crate) const EXTENSION: &str = "osts";

#[cfg(test)]
thread_local! {
    /// The folder of this test thread; removed when the thread ends.
    static TEST_FOLDER: TestFolder = TestFolder::new();
}

#[cfg(test)]
struct TestFolder(PathBuf);

#[cfg(test)]
impl TestFolder {
    fn new() -> TestFolder {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("opensheet-scripts-test-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        TestFolder(path)
    }
}

#[cfg(test)]
impl Drop for TestFolder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The folder the application keeps scripts in when none was chosen.
pub(crate) fn default_folder() -> PathBuf {
    #[cfg(test)]
    {
        TEST_FOLDER.with(|folder| folder.0.clone())
    }
    #[cfg(not(test))]
    {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        home.join("Library/Application Support/OpenSheet/Scripts")
    }
}

/// The folder the scripts are in: the one chosen in Preferences, else
/// the application's own. The scripts written before the folder was
/// changed stay where they are.
pub(crate) fn folder() -> PathBuf {
    let chosen = settings::get().scripts_folder;
    if chosen.trim().is_empty() { default_folder() } else { PathBuf::from(chosen) }
}

/// Where deleted scripts go: the Trash of the user, so that a script
/// deleted by mistake can be taken back.
fn trash() -> Option<PathBuf> {
    #[cfg(test)]
    {
        Some(default_folder().join(".Trash"))
    }
    #[cfg(not(test))]
    {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".Trash"))
    }
}

/// A script of the folder, as the lists show it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub name: String,
    pub description: String,
    pub modified: SystemTime,
}

fn path_of(name: &str) -> PathBuf {
    folder().join(format!("{name}.{EXTENSION}"))
}

/// Why a name cannot be given to a script; `None` when it can. `current`
/// is the name the script has now, which it may of course keep.
pub(crate) fn name_problem(name: &str, current: Option<&str>) -> Option<&'static str> {
    let name = name.trim();
    if name.is_empty() {
        return Some(t("Enter a name for the script.", "Entrez un nom pour le script."));
    }
    if name.chars().count() > 100 || name.starts_with('.') || name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) || name.chars().any(char::is_control) {
        return Some(t(
            "A script name can't start with a period or contain any of these characters: / \\ : * ? \" < > |",
            "Un nom de script ne peut pas commencer par un point ni contenir les caractères suivants : / \\ : * ? \" < > |",
        ));
    }
    let same = current.is_some_and(|current| current.to_lowercase() == name.to_lowercase());
    if !same && exists(name) {
        return Some(t("A script with this name already exists.", "Un script portant ce nom existe déjà."));
    }
    None
}

/// File names do not tell upper case from lower case on this system.
pub(crate) fn exists(name: &str) -> bool {
    let wanted = name.trim().to_lowercase();
    list().iter().any(|entry| entry.name.to_lowercase() == wanted)
}

/// The scripts of the folder, the one changed last first.
pub(crate) fn list() -> Vec<Entry> {
    let mut entries = Vec::new();
    let Ok(dir) = std::fs::read_dir(folder()) else {
        return entries;
    };
    for file in dir.flatten() {
        let path = file.path();
        let is_script = path.extension().is_some_and(|e| e.eq_ignore_ascii_case(EXTENSION));
        let Some(name) = path.file_stem().map(|stem| stem.to_string_lossy().to_string()).filter(|_| is_script) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        // A file that is not a script is not listed.
        let Some(script) = std::fs::read_to_string(&path).ok().and_then(|text| Script::from_osts(&name, &text).ok()) else {
            continue;
        };
        let modified = file.metadata().and_then(|meta| meta.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
        entries.push(Entry {
            name,
            description: script.description,
            modified,
        });
    }
    entries.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    entries
}

pub(crate) fn load(name: &str) -> Result<Script, String> {
    let text = std::fs::read_to_string(path_of(name)).map_err(|e| e.to_string())?;
    Script::from_osts(name, &text)
}

/// Write a script to its file (made, with its folder, when it is new).
pub(crate) fn save(script: &Script) -> Result<(), String> {
    let path = path_of(&script.name);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, script.to_osts()).map_err(|e| e.to_string())
}

/// `base`, or `base 1`, `base 2`… — the first that no script has.
pub(crate) fn free_name(base: &str) -> String {
    let taken: Vec<String> = list().into_iter().map(|entry| entry.name.to_lowercase()).collect();
    if !taken.contains(&base.to_lowercase()) {
        return base.to_string();
    }
    (1..).map(|n| format!("{base} {n}")).find(|name| !taken.contains(&name.to_lowercase())).unwrap_or_else(|| base.to_string())
}

/// A new script holding what Excel starts one with, written to its file.
pub(crate) fn create() -> Result<Script, String> {
    let script = Script::new(&free_name(t("Script", "Script")));
    save(&script)?;
    Ok(script)
}

/// Give a script another name: its file is renamed.
pub(crate) fn rename(old: &str, new: &str) -> Result<String, String> {
    let new = new.trim().to_string();
    if let Some(problem) = name_problem(&new, Some(old)) {
        return Err(problem.to_string());
    }
    if new != old {
        std::fs::rename(path_of(old), path_of(&new)).map_err(|e| e.to_string())?;
    }
    Ok(new)
}

/// Delete a script: its file goes to the Trash.
pub(crate) fn delete(name: &str) -> Result<(), String> {
    let path = path_of(name);
    let moved = trash().is_some_and(|trash| {
        let _ = std::fs::create_dir_all(&trash);
        // A name the Trash does not hold yet.
        let free = (0..1000)
            .map(|n| if n == 0 { format!("{name}.{EXTENSION}") } else { format!("{name} {n}.{EXTENSION}") })
            .map(|file| trash.join(file))
            .find(|target| !target.exists());
        free.is_some_and(|target| std::fs::rename(&path, target).is_ok())
    });
    if moved {
        return Ok(());
    }
    // No Trash to move it to (another volume): the file is removed.
    std::fs::remove_file(path).map_err(|e| e.to_string())
}

/// "Make a copy": the same script under a name of its own; returns it.
pub(crate) fn duplicate(name: &str) -> Result<String, String> {
    let mut script = load(name)?;
    script.name = free_name(&format!("{name} - {}", t("Copy", "Copie")));
    save(&script)?;
    Ok(script.name)
}

/// Bring in a script file made elsewhere (by Excel): it is copied into
/// the folder under its own name, or the next free one. Returns the name.
pub(crate) fn import(file: &Path) -> Result<String, String> {
    let not_a_script = || t("This file is not an Office Script.", "Ce fichier n'est pas un script Office.").to_string();
    let stem = file.file_stem().map(|stem| stem.to_string_lossy().trim().to_string()).filter(|stem| !stem.is_empty()).ok_or_else(not_a_script)?;
    let text = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
    let mut script = Script::from_osts(&stem, &text).map_err(|_| not_a_script())?;
    // A name our folder can hold.
    let cleaned: String = stem.chars().map(|ch| if "/\\:*?\"<>|".contains(ch) || ch.is_control() { '-' } else { ch }).collect();
    script.name = free_name(cleaned.trim_start_matches('.'));
    save(&script)?;
    Ok(script.name)
}

/// Write a copy of a script where the user asked, for Excel or for
/// someone else.
pub(crate) fn export(script: &Script, target: &Path) -> Result<(), String> {
    std::fs::write(target, script.to_osts()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_are_files_of_the_folder() {
        assert!(folder().starts_with(std::env::temp_dir()), "tests must not touch the user's scripts");
        assert!(list().is_empty());
        let first = create().unwrap();
        let second = create().unwrap();
        assert_eq!((first.name.as_str(), second.name.as_str()), ("Script", "Script 1"));
        assert!(folder().join("Script.osts").is_file() && folder().join("Script 1.osts").is_file());
        assert_eq!(load("Script").unwrap(), first);
        let mut names: Vec<String> = list().into_iter().map(|entry| entry.name).collect();
        names.sort();
        assert_eq!(names, ["Script", "Script 1"]);

        // Saving changes the file; what is not a script is not listed.
        let mut changed = first.clone();
        changed.body.push_str("// é\n");
        changed.description = "Does things".to_string();
        save(&changed).unwrap();
        assert_eq!(load("Script").unwrap(), changed);
        assert_eq!(list().iter().find(|entry| entry.name == "Script").map(|entry| entry.description.as_str()), Some("Does things"));
        std::fs::write(folder().join("notes.txt"), "x").unwrap();
        std::fs::write(folder().join("broken.osts"), "not json").unwrap();
        assert_eq!(list().len(), 2);
    }

    #[test]
    fn rename_copy_and_delete_act_on_the_files() {
        let script = create().unwrap();
        assert_eq!(rename(&script.name, "  Tidy up ").unwrap(), "Tidy up");
        assert!(!exists("Script") && exists("tidy UP"));
        assert_eq!(load("Tidy up").unwrap().body, script.body);
        // Names a file cannot have, and names that are taken.
        let other = create().unwrap();
        assert!(rename(&other.name, "Tidy up").is_err());
        assert!(rename(&other.name, "a/b").is_err());
        assert!(rename(&other.name, " ").is_err());
        assert!(rename(&other.name, ".hidden").is_err());
        assert!(exists(&other.name));
        // A script may change the case of its own name.
        assert_eq!(rename("Tidy up", "Tidy Up").unwrap(), "Tidy Up");

        let copy = duplicate("Tidy Up").unwrap();
        assert_eq!(copy, "Tidy Up - Copy");
        assert_eq!(load(&copy).unwrap().body, script.body);
        assert_eq!(duplicate("Tidy Up").unwrap(), "Tidy Up - Copy 1");

        delete("Tidy Up").unwrap();
        assert!(!exists("Tidy Up") && exists("Tidy Up - Copy"));
        // It went to the Trash, where it can be taken back.
        assert!(trash().unwrap().join("Tidy Up.osts").is_file());
        assert!(delete("Tidy Up").is_err());
    }

    #[test]
    fn scripts_come_in_and_go_out_as_osts_files() {
        let outside = default_folder().join("elsewhere");
        std::fs::create_dir_all(&outside).unwrap();
        // A file as Excel writes one.
        let excel = "{\"version\":\"0.3.0\",\"body\":\"function main(workbook: ExcelScript.Workbook) {\\n\\tworkbook.getActiveWorksheet().getRange(\\\"A1\\\").setValue(1);\\n}\",\"description\":\"From Excel\",\"noCodeMetadata\":\"\",\"parameterInfo\":\"{}\",\"apiInfo\":\"{}\"}";
        let file = outside.join("Fill A1.osts");
        std::fs::write(&file, excel).unwrap();
        assert_eq!(import(&file).unwrap(), "Fill A1");
        let script = load("Fill A1").unwrap();
        assert_eq!(script.description, "From Excel");
        assert!(script.body.contains("setValue(1)"));
        // Again: a second script, not the first one overwritten.
        assert_eq!(import(&file).unwrap(), "Fill A1 1");
        // What is not a script is refused, and nothing is written.
        let bad = outside.join("bad.osts");
        std::fs::write(&bad, "{\"nothing\":1}").unwrap();
        assert!(import(&bad).is_err());
        assert!(!exists("bad"));

        let target = outside.join("out.osts");
        export(&script, &target).unwrap();
        let back = Script::from_osts("out", &std::fs::read_to_string(&target).unwrap()).unwrap();
        assert_eq!(back.body, script.body);
    }

    #[test]
    fn the_folder_can_be_changed() {
        let usual = folder();
        create().unwrap();
        // What Preferences does when another folder is chosen.
        let other = default_folder().join("mine");
        settings::update(|settings| settings.scripts_folder = other.to_string_lossy().to_string());
        assert_eq!(folder(), other);
        assert!(list().is_empty());
        let script = create().unwrap();
        assert!(other.join(format!("{}.osts", script.name)).is_file());
        settings::update(|settings| settings.scripts_folder.clear());
        assert_eq!(folder(), usual);
        assert_eq!(list().len(), 1);
    }
}
