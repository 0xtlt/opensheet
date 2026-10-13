//! The keyboard shortcuts of the Help pane, read from the key bindings
//! the application actually installed (`bind_keys`), so the list cannot
//! say anything the keyboard does not do.
//!
//! Only the bindings of the sheet and of the window are listed: the keys
//! of text boxes and dialogs (arrows, Tab, Return) are those of every
//! Mac application.

use super::docs::Line;
use crate::i18n::t;
use gpui::{App, KeyBindingContextPredicate, Keystroke};

/// Where a binding must apply to be listed.
const CONTEXTS: [&str; 2] = ["Grid", "Workbook"];

/// What an action does, by the name it was declared under: the label of
/// its command, or the words of Excel's own list of shortcuts where it has
/// them. An action bound without a line here is listed under its own
/// name, spaced out.
fn described(action: &str) -> Option<&'static str> {
    Some(match action {
        "MoveUp" => t("Select Cell Up", "Sélectionner une cellule en haut"),
        "MoveDown" => t("Select Cell Down", "Sélectionner une cellule vers le bas"),
        "MoveLeft" => t("Select Cell Left", "Sélectionner la cellule de gauche"),
        "MoveRight" => t("Select Cell Right", "Sélectionner la cellule de droite"),
        "ExtendUp" | "ExtendDown" | "ExtendLeft" | "ExtendRight" => {
            t("Extend the selection by one cell", "Étendre la sélection d’une cellule")
        }
        "JumpUp" | "JumpDown" | "JumpLeft" | "JumpRight" => {
            t("Move to the edge of the current data region", "Atteindre le bord de la zone de données active")
        }
        "ExtendJumpUp" | "ExtendJumpDown" | "ExtendJumpLeft" | "ExtendJumpRight" => t(
            "Extend the selection to the edge of the current data region",
            "Étendre la sélection jusqu’au bord de la zone de données active",
        ),
        "NextCell" => t("Move Right One Cell", "Déplacer vers la droite d’une cellule"),
        "PrevCell" => t("Move Left One Cell", "Déplacer vers la gauche d’une cellule"),
        "EnterDown" => t("Move Down One Cell", "Descendre d’une cellule"),
        "EnterUp" => t("Move Up One Cell", "Monter d’une cellule"),
        "EditCell" => t("Enter Edit Mode", "Activer le mode d’édition"),
        "ClearContents" => t("Clear Contents", "Effacer le contenu"),
        "Copy" => t("Copy", "Copier"),
        "Cut" => t("Cut", "Couper"),
        "Paste" => t("Paste", "Coller"),
        "PasteValues" => t("Paste Values", "Coller les valeurs"),
        "PasteSpecialDialog" => t("Paste Special…", "Collage spécial…"),
        "SelectAll" => t("Select All", "Sélectionner tout"),
        "FillDown" => t("Fill Down", "Recopier vers le bas"),
        "FillRight" => t("Fill Right", "Recopier à droite"),
        "PageUp" => t("Move one screen up", "Remonter d’un écran"),
        "PageDown" => t("Move one screen down", "Descendre d’un écran"),
        "GoRowStart" => t("Move to the beginning of the row", "Atteindre le début de la ligne"),
        "GoStart" => t("Go to cell A1", "Aller à la cellule A1"),
        "GoEnd" => t("Go to last cell of the used range", "Atteindre la dernière cellule de la plage utilisée"),
        "SelectColumn" => t("Select an entire column", "Sélectionner une colonne entière"),
        "SelectRow" => t("Select an entire row", "Sélectionner une ligne entière"),
        "InsertDate" => t("Enter the date", "Insérer la date"),
        "AutoSum" => t("AutoSum", "Somme automatique"),
        "InsertRows" => t("Insert cells, rows or columns", "Insérer des cellules, des lignes ou des colonnes"),
        "DeleteRows" => t("Delete cells, rows or columns", "Supprimer des cellules, des lignes ou des colonnes"),
        "Cancel" => t("Cancel an entry", "Annuler une saisie"),
        "OpenDropDown" => t("Open the drop-down list of the cell", "Ouvrir la liste déroulante de la cellule"),
        "Undo" => t("Undo", "Annuler"),
        "Redo" => t("Redo", "Rétablir"),
        "Bold" => t("Bold", "Gras"),
        "Italic" => t("Italic", "Italique"),
        "UnderlineText" => t("Underline", "Souligné"),
        "Save" => t("Save", "Enregistrer"),
        "SaveAs" => t("Save As…", "Enregistrer sous…"),
        "Open" => t("Open…", "Ouvrir…"),
        "NewWorkbook" => t("New", "Nouveau"),
        "Find" | "FindDialog" => t("Find…", "Rechercher…"),
        "Replace" => t("Replace…", "Remplacer…"),
        "ZoomIn" => t("Zoom In", "Zoom avant"),
        "ZoomOut" => t("Zoom Out", "Zoom arrière"),
        "ZoomReset" => t("Actual Size", "Taille réelle"),
        "NextSheet" => t("Next Sheet", "Feuille suivante"),
        "PrevSheet" => t("Previous Sheet", "Feuille précédente"),
        "FormatCells" => t("Format Cells", "Format de cellule"),
        "Print" => t("Print…", "Imprimer…"),
        "ToggleFilter" => t("Filter", "Filtrer"),
        "InsertTable" => t("Create Table", "Créer un tableau"),
        "NewNote" => t("New Note", "Nouvelle note"),
        "NewComment" => t("New Comment", "Nouveau commentaire"),
        "NameManager" => t("Name Manager", "Gestionnaire de noms"),
        "InsertLink" => t("Insert Hyperlink", "Insérer un lien hypertexte"),
        "Quit" => t("Quit OpenSheet", "Quitter OpenSheet"),
        "FlashFill" => t("Flash Fill", "Remplissage instantané"),
        "GoTo" => t("Go To…", "Atteindre…"),
        "InsertFunction" => t("Insert Function", "Insérer une fonction"),
        "ShowFormulas" => t("Show Formulas", "Afficher les formules"),
        "CalculateNow" => t("Calculate Now", "Calculer maintenant"),
        "CalculateSheet" => t("Calculate Sheet", "Calculer la feuille"),
        "PasteName" => t("Paste Names…", "Coller des noms…"),
        "CreateNames" => t("Create from Selection", "Depuis sélection"),
        "GroupSelection" => t("Group…", "Grouper…"),
        "UngroupSelection" => t("Ungroup…", "Dissocier…"),
        "ToggleOutlineSymbols" => t("Outline symbols", "Symboles du plan"),
        "OpenPreferences" => t("Preferences…", "Préférences…"),
        "Spelling" => t("Spelling", "Orthographe"),
        "Thesaurus" => t("Thesaurus", "Dictionnaire des synonymes"),
        _ => return None,
    })
}

/// "ToggleOutlineSymbols" as words.
pub(crate) fn spaced(name: &str) -> String {
    let mut out = String::new();
    for ch in name.chars() {
        if ch.is_uppercase() && !out.is_empty() {
            out.push(' ');
        }
        out.push(ch);
    }
    out
}

/// What the action `name` ("opensheet::MoveUp") does, in words.
pub(crate) fn description(name: &str) -> String {
    let short = name.rsplit("::").next().unwrap_or(name);
    described(short).map_or_else(|| spaced(short), |text| text.trim_end_matches('…').to_string())
}

/// A key as the Mac writes it.
fn key_name(key: &str) -> String {
    match key {
        "up" => "↑".into(),
        "down" => "↓".into(),
        "left" => "←".into(),
        "right" => "→".into(),
        "enter" => "↩".into(),
        "tab" => "⇥".into(),
        "escape" => "⎋".into(),
        "backspace" => "⌫".into(),
        "delete" => "⌦".into(),
        "space" => t("Space", "Espace").into(),
        "pageup" => "⇞".into(),
        "pagedown" => "⇟".into(),
        "home" => "↖".into(),
        "end" => "↘".into(),
        other => other.to_uppercase(),
    }
}

/// A keystroke as the menus of the Mac write it: ⌃⌥⇧⌘ then the key.
pub(crate) fn keys(stroke: &Keystroke) -> String {
    let mut out = String::new();
    for (held, sign) in [
        (stroke.modifiers.control, '⌃'),
        (stroke.modifiers.alt, '⌥'),
        (stroke.modifiers.shift, '⇧'),
        (stroke.modifiers.platform, '⌘'),
    ] {
        if held {
            out.push(sign);
        }
    }
    out + &key_name(&stroke.key)
}

/// The shortcuts of the sheet and of the window, one line per thing they
/// do with every key that does it, in the order the keys were bound.
pub(crate) fn lines(cx: &App) -> Vec<Line> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    let mut out: Vec<Line> = Vec::new();
    for binding in keymap.bindings() {
        let listed = match binding.predicate().as_deref() {
            None => true,
            Some(KeyBindingContextPredicate::Identifier(context)) => CONTEXTS.contains(&context.as_ref()),
            Some(_) => false,
        };
        if !listed {
            continue;
        }
        let stroke: Vec<String> = binding.keystrokes().iter().map(|stroke| keys(stroke.inner())).collect();
        let stroke = stroke.join(" ");
        let text = description(binding.action().name());
        match out.iter_mut().find(|line| line.text == text) {
            Some(line) => {
                let all = line.keys.get_or_insert_with(String::new);
                if !all.split(", ").any(|known| known == stroke) {
                    all.push_str(", ");
                    all.push_str(&stroke);
                }
            }
            None => out.push(Line {
                text,
                bullet: false,
                keys: Some(stroke),
            }),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_written_as_the_mac_writes_them() {
        let written = |source: &str| keys(&Keystroke::parse(source).unwrap());
        assert_eq!(written("cmd-c"), "⌘C");
        assert_eq!(written("cmd-shift-m"), "⇧⌘M");
        assert_eq!(written("ctrl-alt-shift-cmd-f3"), "⌃⌥⇧⌘F3");
        assert_eq!(written("alt-down"), "⌥↓");
        assert_eq!(written("shift-space"), "⇧Space");
        assert_eq!(written("ctrl-;"), "⌃;");
        assert_eq!(written("cmd-,"), "⌘,");
    }

    #[test]
    fn actions_are_said_in_words() {
        assert_eq!(description("opensheet::Copy"), "Copy");
        // Menu labels lose their dots.
        assert_eq!(description("formulas::GoTo"), "Go To");
        // An action nobody described still has a line.
        assert_eq!(description("review::CheckSpellingNow"), "Check Spelling Now");
        assert_eq!(spaced("Undo"), "Undo");
    }
}
