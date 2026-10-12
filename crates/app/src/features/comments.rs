//! Comments and notes: the indicators in cells, the boxes shown on hover,
//! editing, the Comments pane.
//!
//! Not written yet: the commands are in place and say so. See
//! `features/mod.rs` for the hooks (indicators and hover boxes are painted
//! with zones, a note is typed in an element of `overlay_elements`, the
//! Comments pane is opened with `open_pane`).

use super::{Command, ContextMenu, Cx, Feature, MenuSection, MenuTarget};
use crate::app::{ExcelApp, MenuEntry};
use crate::i18n::t;
use gpui::Window;

pub struct Comments;

commands! {
    /// What the user can ask of comments and notes.
    pub enum Cmd {
        NewComment = "comment", t("New Comment", "Nouveau commentaire");
        Delete = "trash", t("Delete", "Supprimer");
        Previous = "chevron-left", t("Previous Comment", "Commentaire précédent");
        Next = "chevron-right", t("Next Comment", "Commentaire suivant");
        ShowPane = "comment", t("Show Comments", "Afficher les commentaires");
        NotesMenu = "note", t("Notes", "Notes");
        NewNote = "note", t("New Note", "Nouvelle note");
        PreviousNote = "", t("Previous Note", "Note précédente");
        NextNote = "", t("Next Note", "Note suivante");
        ShowHideNote = "", t("Show/Hide Note", "Afficher/Masquer la note");
        ShowAllNotes = "", t("Show All Notes", "Afficher toutes les notes");
        ConvertToComments = "", t("Convert to Comments", "Convertir en commentaires");
        Clear = "", t("Clear Comments and Notes", "Effacer les commentaires et les notes");
    }
}

impl Command for Cmd {
    fn label(self) -> &'static str {
        self.face().1
    }

    fn icon(self) -> &'static str {
        self.face().0
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        match self {
            Cmd::NotesMenu => {
                let entries = vec![
                    MenuEntry::command(Cmd::NewNote, app).shortcut("⇧F2"),
                    MenuEntry::command(Cmd::PreviousNote, app),
                    MenuEntry::command(Cmd::NextNote, app),
                    MenuEntry::command(Cmd::ShowHideNote, app),
                    MenuEntry::command(Cmd::ShowAllNotes, app),
                    MenuEntry::command(Cmd::ConvertToComments, app),
                ];
                app.open_command_menu(entries, 240.0, window, cx);
            }
            _ => app.not_implemented(self.label(), cx),
        }
    }
}

impl Feature for Comments {
    fn name(&self) -> &'static str {
        "comments"
    }

    fn context_menu(&self, app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
        if matches!(target, MenuTarget::Cell(..)) {
            menu.add(MenuSection::Review, MenuEntry::command(Cmd::NewComment, app));
            menu.add(MenuSection::Review, MenuEntry::command(Cmd::NewNote, app));
        }
    }
}
