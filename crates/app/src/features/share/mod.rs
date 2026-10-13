//! Share: the button at the right of the tab row, and File ▸ Share.
//!
//! Excel shares a workbook from the cloud: it invites people to the file
//! or hands out a link to it, and several people then edit it together.
//! OpenSheet has no cloud location, so those two entries answer what
//! Excel answers for a file that is not in the cloud. What works on a
//! file of this computer is Send a Copy: the workbook, or a PDF of it,
//! goes to the share sheet of macOS (Mail, Messages, AirDrop…), which
//! hangs under the Share button.
//!
//! The saved file itself is sent when it is up to date; a workbook with
//! changes that were not saved, or never saved, is written to a temporary
//! copy first, so that what is sent is what is on screen. Nothing is sent
//! by OpenSheet itself: the share sheet is where the user decides.

pub mod system;
#[cfg(test)]
mod ui_tests;

use self::system::{Item, system};
use super::{Command, Cx};
use crate::app::{ExcelApp, MenuEntry};
use crate::i18n::t;
use crate::shell::Alert;
use gpui::{Bounds, Context, Pixels, Stateful, Window, actions, prelude::*, px, size};
use std::path::{Path, PathBuf};

/// The control the menu and the share sheet hang from.
pub const BUTTON: &str = "btn-share";

commands! {
    /// What the user can ask of sharing.
    pub enum Cmd {
        Share = "share", t("Share", "Partager");
        InvitePeople = "", t("Invite People...", "Inviter des personnes...");
        CopyLink = "link", t("Copy Link", "Copier le lien");
        SendWorkbook = "", t("Send Workbook", "Envoyer un classeur");
        SendPdf = "", t("Send PDF", "Envoyer au format PDF");
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
            Cmd::Share => app.open_menu(BUTTON, menu(app), 230.0, window, cx),
            Cmd::InvitePeople | Cmd::CopyLink => needs_the_cloud(app, self, cx),
            Cmd::SendWorkbook => send(app, Format::Workbook, window, cx),
            Cmd::SendPdf => send(app, Format::Pdf, window, cx),
        }
    }
}

/// The menu of the Share button: Excel's three ways to share.
fn menu(app: &ExcelApp) -> Vec<MenuEntry> {
    let copies = vec![
        MenuEntry::new(t("Workbook", "Classeur"), |app, window, cx| Cmd::SendWorkbook.run(app, window, cx)),
        MenuEntry::new(t("PDF", "PDF"), |app, window, cx| Cmd::SendPdf.run(app, window, cx)),
    ];
    vec![
        MenuEntry::command(Cmd::InvitePeople, app),
        MenuEntry::command(Cmd::CopyLink, app),
        MenuEntry::separator(),
        MenuEntry::submenu(t("Send a Copy", "Envoyer une copie"), copies),
    ]
}

/// File ▸ Share, as Excel's File menu has it.
pub fn file_menu_entry(app: &ExcelApp) -> MenuEntry {
    let entries = vec![
        MenuEntry::command(Cmd::InvitePeople, app),
        MenuEntry::command(Cmd::CopyLink, app),
        MenuEntry::separator(),
        MenuEntry::command(Cmd::SendWorkbook, app),
        MenuEntry::command(Cmd::SendPdf, app),
    ];
    MenuEntry::submenu(Cmd::Share.label(), entries).icon(Cmd::Share.icon())
}

/// Inviting people and links need the file to be in the cloud: say what
/// Excel says of a file that is not, and what can be done here instead.
fn needs_the_cloud(app: &mut ExcelApp, cmd: Cmd, cx: &mut Cx) {
    let message = format!(
        "{}\n\n{}",
        t(
            "Save this file to the cloud to share it with other people.",
            "Enregistrez ce fichier dans le cloud pour le partager avec d’autres personnes."
        ),
        t(
            "OpenSheet has no cloud location. To share this workbook, use Send a Copy.",
            "OpenSheet n’a pas d’emplacement dans le cloud. Pour partager ce classeur, utilisez Envoyer une copie."
        ),
    );
    let title = cmd.label().trim_end_matches('.').to_string();
    app.alert(Alert::new(title, message), cx);
}

// ------------------------------------------------------------- send a copy

/// What Send a Copy sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Format {
    Workbook,
    Pdf,
}

/// A name that can be a file name: the title of the workbook without what
/// a path cannot hold.
pub(crate) fn file_stem(title: &str) -> String {
    let clean: String = title.chars().map(|ch| if matches!(ch, '/' | ':' | '\\' | '\0') { '-' } else { ch }).collect();
    let clean = clean.trim().trim_start_matches('.').to_string();
    if clean.is_empty() { "OpenSheet".to_string() } else { clean }
}

/// Whether the file of the workbook can be sent as it is: it was saved as
/// a workbook, nothing changed since, and it is still there.
pub(crate) fn saved_file(path: Option<&Path>, modified: bool) -> Option<&Path> {
    let path = path?;
    let workbook = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("xlsx") || ext.eq_ignore_ascii_case("xlsm"));
    (workbook && !modified && path.is_file()).then_some(path)
}

/// Where temporary copies are written: a folder of their own, so that the
/// copy carries the name of the workbook.
fn copies_dir() -> PathBuf {
    std::env::temp_dir().join("OpenSheet").join(t("Shared", "Partagé"))
}

/// The file to send: the saved workbook when it is up to date, else a
/// temporary copy of what is on screen; for a PDF, the whole workbook as
/// Print would save it. The error is the message to show.
fn prepare(app: &mut ExcelApp, format: Format, cx: &mut Cx) -> Result<PathBuf, String> {
    let failed = |detail: String| {
        format!(
            "{}\n\n{detail}",
            t(
                "Failed to prepare a copy of your file for sharing.",
                "La préparation d’une copie de votre fichier à des fins de partage a échoué."
            )
        )
    };
    let dir = copies_dir();
    let name = file_stem(&app.title);
    match format {
        Format::Workbook => {
            if let Some(saved) = saved_file(app.path.as_deref(), app.wb.modified) {
                return Ok(saved.to_path_buf());
            }
            // The copy is what Save would write.
            super::preferences::before_save(&mut app.wb);
            let path = dir.join(format!("{name}.xlsx"));
            std::fs::create_dir_all(&dir)
                .map_err(|err| err.to_string())
                .and_then(|_| opensheet_io::save_xlsx(&app.wb, &path).map_err(|err| err.to_string()))
                .map_err(|err| failed(format!("{}\n{err}", path.display())))?;
            Ok(path)
        }
        Format::Pdf => {
            let Some(pdf) = super::page::workbook_pdf(app, cx) else {
                return Err(t(
                    "We didn't find anything to print.",
                    "Désolé... Nous n'avons trouvé aucun contenu à imprimer.",
                )
                .to_string());
            };
            let path = dir.join(format!("{name}.pdf"));
            std::fs::create_dir_all(&dir)
                .and_then(|_| std::fs::write(&path, pdf))
                .map_err(|err| failed(format!("{}\n{err}", path.display())))?;
            Ok(path)
        }
    }
}

/// The box the share sheet hangs from: the Share button, or the pointer
/// when the button was never drawn.
pub(crate) fn sheet_anchor(app: &ExcelApp, window: &Window) -> Bounds<Pixels> {
    app.anchors
        .borrow()
        .get(BUTTON)
        .copied()
        .unwrap_or_else(|| Bounds::new(window.mouse_position(), size(px(1.), px(1.))))
}

/// Send a Copy: hand the workbook, or its PDF, to the share sheet.
fn send(app: &mut ExcelApp, format: Format, window: &mut Window, cx: &mut Cx) {
    // What is being typed is part of what is sent.
    if app.is_editing() && !app.commit_edit(None, window, cx) {
        return;
    }
    let shared = prepare(app, format, cx).and_then(|path| {
        let under = sheet_anchor(app, window);
        system().share(&[Item::File(path)], under, window, cx).map_err(|err| {
            format!(
                "{}\n\n{err}",
                t(
                    "Something went wrong and this file can't be shared.",
                    "Désolé... Nous ne pouvons pas partager ce fichier, car nous avons rencontré une erreur."
                )
            )
        })
    });
    if let Err(message) = shared {
        app.alert(Alert::new(t("Sharing Error", "Erreur de partage"), message).warning(), cx);
    }
}

// ---------------------------------------------------------------- menu bar

actions!(share, [InvitePeople, CopyLink, SendWorkbook, SendPdf]);

/// Make the window answer the entries of File ▸ Share in the menu bar.
pub fn key_actions(root: Stateful<gpui::Div>, cx: &Context<ExcelApp>) -> Stateful<gpui::Div> {
    root.on_action(cx.listener(|app, _: &InvitePeople, window, cx| app.run_command(Cmd::InvitePeople, window, cx)))
        .on_action(cx.listener(|app, _: &CopyLink, window, cx| app.run_command(Cmd::CopyLink, window, cx)))
        .on_action(cx.listener(|app, _: &SendWorkbook, window, cx| app.run_command(Cmd::SendWorkbook, window, cx)))
        .on_action(cx.listener(|app, _: &SendPdf, window, cx| app.run_command(Cmd::SendPdf, window, cx)))
}

/// File ▸ Share of the menu bar.
pub fn menu_bar() -> gpui::Menu {
    use gpui::MenuItem;
    gpui::Menu {
        name: Cmd::Share.label().into(),
        items: vec![
            MenuItem::action(Cmd::InvitePeople.label(), InvitePeople),
            MenuItem::action(Cmd::CopyLink.label(), CopyLink),
            MenuItem::separator(),
            MenuItem::action(Cmd::SendWorkbook.label(), SendWorkbook),
            MenuItem::action(Cmd::SendPdf.label(), SendPdf),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_become_file_names() {
        assert_eq!(file_stem("Budget 2026"), "Budget 2026");
        assert_eq!(file_stem("a/b:c\\d"), "a-b-c-d");
        assert_eq!(file_stem("  "), "OpenSheet");
        assert_eq!(file_stem(".hidden"), "hidden");
    }

    #[test]
    fn only_an_up_to_date_workbook_file_is_sent_as_it_is() {
        let dir = std::env::temp_dir().join(format!("opensheet-share-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (book, csv) = (dir.join("Book.XLSX"), dir.join("Book.csv"));
        std::fs::write(&book, b"x").unwrap();
        std::fs::write(&csv, b"x").unwrap();
        assert_eq!(saved_file(Some(&book), false), Some(book.as_path()));
        // Changes not saved, another format, a file that is gone, no file.
        assert_eq!(saved_file(Some(&book), true), None);
        assert_eq!(saved_file(Some(&csv), false), None);
        assert_eq!(saved_file(Some(&dir.join("gone.xlsx")), false), None);
        assert_eq!(saved_file(None, false), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
