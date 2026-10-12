//! Notes and comments, the two kinds of annotation a cell can carry.
//!
//! A *note* is the yellow box of old: a red corner in its cell, the box
//! shown while the pointer rests on the cell or left showing on the
//! sheet, typed in place, moved and resized by its border and handles.
//! A *comment* is a conversation: a purple mark in its cell and a card
//! (who said what and when, replies, resolve) beside the cell, or in the
//! Comments pane when that is open.
//!
//! The marks and the boxes are painted on the grid (`notes.rs`); the box
//! in which a note is typed and the comment cards are elements placed
//! over it (`text_area.rs`, `card.rs`, `pane.rs`).

mod card;
mod notes;
mod pane;
mod text_area;

pub use self::text_area::bind_keys;

use self::card::{Beside, CommentCard};
use self::notes::{BoxDrag, Grip};
use self::pane::CommentsPane;
use self::text_area::{AreaEvent, TextArea};
use super::{
    Command, ContextMenu, Cx, Feature, Intent, MenuSection, MenuTarget, PaintLayer, Painter, StatusItem, Zone,
    ZoneEvent, ZoneResponse,
};
use crate::app::{ExcelApp, MenuEntry};
use crate::grid::{GridLayout, pt_to_px};
use crate::i18n::t;
use crate::shell::Alert;
use crate::text_input as line;
use crate::theme::{self, c};
use gpui::{
    AnyElement, CursorStyle, Entity, FocusHandle, Focusable, MouseButton, MouseDownEvent, Pixels, Point, Subscription,
    Task, Window, div, prelude::*, px,
};
use opensheet_engine::comment::{self, Comment, CommentKind, Note, Person};
use opensheet_engine::refs::cell_name;
use opensheet_engine::workbook::ClearMode;
use opensheet_engine::Rect;
use std::sync::OnceLock;
use std::time::Duration;

pub struct Comments;

/// Id of the Comments pane.
const PANE: &str = "comments";
/// How long the pointer rests on a cell before its note or comment shows.
const REST: Duration = Duration::from_millis(350);
/// How long they stay once the pointer has left: no time for a note, the
/// time to reach its card for a comment.
const LINGER_NOTE: Duration = Duration::from_millis(60);
const LINGER_CARD: Duration = Duration::from_millis(300);
/// The corner mark of a note, and that of a comment.
const NOTE_MARK: u32 = 0xFF0000;
const COMMENT_MARK: u32 = 0x8764B8;

commands! {
    /// What the user can ask of comments and notes.
    pub enum Cmd {
        NewComment = "comment", t("New Comment", "Nouveau commentaire");
        Reply = "comment", t("Reply to Comment", "Répondre au commentaire");
        Delete = "trash", t("Delete", "Supprimer");
        DeleteComment = "trash", t("Delete Comment", "Supprimer le commentaire");
        Previous = "chevron-left", t("Previous Comment", "Commentaire précédent");
        Next = "chevron-right", t("Next Comment", "Commentaire suivant");
        ShowPane = "comment", t("Show Comments", "Afficher les commentaires");
        NotesMenu = "note", t("Notes", "Notes");
        NewNote = "note", t("New Note", "Nouvelle note");
        EditNote = "note", t("Edit Note", "Modifier la note");
        DeleteNote = "trash", t("Delete Note", "Supprimer la note");
        PreviousNote = "", t("Previous Note", "Note précédente");
        NextNote = "", t("Next Note", "Note suivante");
        ShowHideNote = "", t("Show/Hide Note", "Afficher/masquer la note");
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

    fn enabled(self, app: &ExcelApp) -> bool {
        let (row, col) = app.active();
        let here = kind_at(app, app.sheet, (row, col));
        let any = |kind: CommentKind| app.wb.first_comment(Some(kind)).is_some();
        match self {
            Cmd::NewNote => here != Some(CommentKind::Thread),
            Cmd::EditNote | Cmd::DeleteNote | Cmd::ShowHideNote => here == Some(CommentKind::Note),
            Cmd::Reply | Cmd::DeleteComment => here == Some(CommentKind::Thread),
            Cmd::Delete => {
                let ranges = &app.sel().ranges;
                let inside = |(row, col): &(u32, u32)| ranges.iter().any(|range| range.contains(*row, *col));
                app.wb.sheets[app.sheet].comments.keys().any(inside)
            }
            Cmd::Previous | Cmd::Next => any(CommentKind::Thread),
            Cmd::PreviousNote | Cmd::NextNote => any(CommentKind::Note),
            Cmd::ConvertToComments => app.wb.sheets[app.sheet]
                .comments
                .values()
                .any(|comment| comment.kind() == CommentKind::Note),
            _ => true,
        }
    }

    fn checked(self, app: &ExcelApp) -> bool {
        match self {
            Cmd::ShowPane => app.pane_open(PANE),
            Cmd::ShowAllNotes => app.wb.all_notes_visible(app.sheet),
            _ => false,
        }
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        // What is being typed in a cell is stored first.
        if app.is_editing() && !app.commit_edit(None, window, cx) {
            return;
        }
        let (row, col) = app.active();
        let sheet = app.sheet;
        match self {
            Cmd::NotesMenu => {
                let entries = notes_menu(kind_at(app, sheet, (row, col)))
                    .into_iter()
                    .map(|cmd| {
                        let entry = MenuEntry::command(cmd, app);
                        if matches!(cmd, Cmd::NewNote | Cmd::EditNote) { entry.shortcut("⇧F2") } else { entry }
                    })
                    .collect();
                app.open_command_menu(entries, 240.0, window, cx);
            }
            // ⇧F2 on a cell that has a note opens it.
            Cmd::NewNote | Cmd::EditNote => edit_note(app, (row, col), None, window, cx),
            Cmd::DeleteNote | Cmd::DeleteComment => {
                finish_note_edit(app, cx);
                if may_edit(app, cx) && app.wb.delete_comment(sheet, row, col) {
                    app.commit(self.label(), cx);
                }
            }
            Cmd::Delete => {
                finish_note_edit(app, cx);
                let ranges = app.sel().ranges.clone();
                if may_edit(app, cx) && app.wb.delete_comments(sheet, &ranges) > 0 {
                    app.commit(Cmd::DeleteComment.label(), cx);
                }
            }
            // One of the Clear commands of the Home tab: like the others,
            // it leaves the rows a filter hides alone.
            Cmd::Clear => {
                finish_note_edit(app, cx);
                if may_edit(app, cx) {
                    for range in app.sel().ranges.clone() {
                        app.wb.clear(sheet, range, ClearMode::Comments);
                    }
                    app.commit(self.label(), cx);
                }
            }
            Cmd::ShowHideNote => {
                let shown = app
                    .wb
                    .comment_at(sheet, row, col)
                    .and_then(Comment::as_note)
                    .is_some_and(|note| note.visible);
                if may_edit(app, cx) && app.wb.set_note_visible(sheet, row, col, !shown) {
                    app.commit(self.label(), cx);
                }
            }
            Cmd::ShowAllNotes => {
                let all = app.wb.all_notes_visible(sheet);
                if may_edit(app, cx) && app.wb.set_all_notes_visible(sheet, !all) {
                    app.commit(self.label(), cx);
                }
            }
            Cmd::PreviousNote => go(app, CommentKind::Note, false, window, cx),
            Cmd::NextNote => go(app, CommentKind::Note, true, window, cx),
            Cmd::Previous => go(app, CommentKind::Thread, false, window, cx),
            Cmd::Next => go(app, CommentKind::Thread, true, window, cx),
            Cmd::ConvertToComments => {
                let (title, text) = comment::convert_question(app.wb.locale);
                app.alert(
                    Alert::new(title, text)
                        .button(t("Convert all notes", "Convertir toutes les notes"), move |app, _, cx| {
                            finish_note_edit(app, cx);
                            if may_edit(app, cx) && app.wb.convert_notes_to_comments(sheet, comment::now_utc()) > 0 {
                                app.commit(Cmd::ConvertToComments.label(), cx);
                            }
                        })
                        .close_button(t("Cancel", "Annuler")),
                    cx,
                );
            }
            Cmd::NewComment | Cmd::Reply => write_comment(app, window, cx),
            Cmd::ShowPane => toggle_pane(app, cx),
        }
    }
}

/// What the Notes button of the Review tab lists: New Note becomes Edit
/// Note on a cell that has one.
fn notes_menu(here: Option<CommentKind>) -> Vec<Cmd> {
    let first = if here == Some(CommentKind::Note) { Cmd::EditNote } else { Cmd::NewNote };
    vec![
        first,
        Cmd::PreviousNote,
        Cmd::NextNote,
        Cmd::ShowHideNote,
        Cmd::ShowAllNotes,
        Cmd::ConvertToComments,
    ]
}

/// What the menu of a cell offers, as Excel does: to annotate a bare
/// cell, to change its note, or to answer its comment.
fn cell_menu(here: Option<CommentKind>) -> &'static [Cmd] {
    match here {
        None => &[Cmd::NewComment, Cmd::NewNote],
        Some(CommentKind::Note) => &[Cmd::NewComment, Cmd::EditNote, Cmd::DeleteNote, Cmd::ShowHideNote],
        Some(CommentKind::Thread) => &[Cmd::Reply, Cmd::DeleteComment],
    }
}

/// The full name of the person at the keyboard: asked from the system
/// once, as Excel does when it first runs, and kept for the session.
pub(self) fn user_name() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        let clean = |name: String| Some(name.trim().to_string()).filter(|name| !name.is_empty());
        std::process::Command::new("id")
            .arg("-F")
            .output()
            .ok()
            .filter(|out| out.status.success())
            .and_then(|out| clean(String::from_utf8_lossy(&out.stdout).into_owned()))
            .or_else(|| std::env::var("USER").ok().and_then(clean))
            .unwrap_or_else(|| t("Author", "Auteur").to_string())
    })
}

fn kind_at(app: &ExcelApp, sheet: usize, cell: (u32, u32)) -> Option<CommentKind> {
    app.wb.comment_at(sheet, cell.0, cell.1).map(Comment::kind)
}

/// May notes and comments be changed? A protected sheet says no, and why.
pub(self) fn may_edit(app: &mut ExcelApp, cx: &mut Cx) -> bool {
    app.may(Intent::EditComment, cx)
}

// -------------------------------------------------------------------- state

/// What a zone of this module stands for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(self) enum Spot {
    /// A cell that has a note or a comment.
    Cell(u32, u32),
    /// The inside of the note box of a cell.
    Body(u32, u32),
    /// Its border, or one of its handles.
    Grip(u32, u32, Grip),
}

/// The annotated cell under the pointer.
struct Hover {
    cell: (u32, u32),
    /// The pointer has rested: the note or the comment shows.
    shown: bool,
}

/// A note being typed in its box.
struct NoteEdit {
    sheet: usize,
    cell: (u32, u32),
    area: Entity<TextArea>,
    /// Its text before, `None` for a new note.
    original: Option<String>,
    _subscription: Subscription,
}

/// The comment card open beside its cell.
struct OpenCard {
    sheet: usize,
    cell: (u32, u32),
    view: Entity<CommentCard>,
    /// Opened by a click or a command: it stays when the pointer leaves.
    pinned: bool,
    pointer_inside: bool,
}

/// What the feature remembers between events.
#[derive(Default)]
pub(self) struct Ui {
    hover: Option<Hover>,
    /// Shows the note or the comment once the pointer has rested.
    rest_timer: Option<Task<()>>,
    /// Takes it away shortly after the pointer left.
    leave_timer: Option<Task<()>>,
    /// The note Previous / Next Note went to: its box shows until the
    /// selection moves.
    visited: Option<(u32, u32)>,
    note_edit: Option<NoteEdit>,
    /// The note whose box is selected: it shows its handles.
    note_sel: Option<(u32, u32)>,
    /// What has the keyboard while a box is selected.
    box_focus: Option<FocusHandle>,
    _box_blur: Option<Subscription>,
    drag: Option<BoxDrag>,
    card: Option<OpenCard>,
    pane: Option<Entity<CommentsPane>>,
}

impl Ui {
    /// Whether the box of a hidden note shows for now: the pointer rests
    /// on its cell, or Next Note went there.
    fn peeks(&self, cell: (u32, u32)) -> bool {
        self.visited == Some(cell) || self.hover.as_ref().is_some_and(|hover| hover.shown && hover.cell == cell)
    }
}

// -------------------------------------------------------------------- notes

/// Open the box of the note of a cell for typing, the cursor at its end
/// or under `at` (a point of the window). A new note starts with the name
/// of its author.
fn edit_note(app: &mut ExcelApp, cell: (u32, u32), at: Option<Point<Pixels>>, window: &mut Window, cx: &mut Cx) {
    if app.is_editing() && !app.commit_edit(None, window, cx) {
        return;
    }
    finish_note_edit(app, cx);
    let sheet = app.sheet;
    let (original, author) = match app.wb.comment_at(sheet, cell.0, cell.1) {
        Some(Comment::Thread(_)) => return,
        Some(Comment::Note(note)) => (Some(note.text()), note.author.clone()),
        None => (None, user_name().to_string()),
    };
    if !may_edit(app, cx) {
        return;
    }
    let text = original.clone().unwrap_or_else(|| Note::template(&author));
    let area = cx.new(|cx| {
        let mut area = TextArea::new(cx);
        area.bold_prefix = Some(format!("{author}:"));
        // The box is filled whatever the length of the text.
        area.min_rows = 10_000;
        area.max_height = Some(comment::NOTE_SIZE.1);
        area.set_text(text, cx);
        if let Some(at) = at {
            area.caret_at_next_paint(at);
        }
        area
    });
    let subscription = cx.subscribe_in(&area, window, |app, _, event: &AreaEvent, window, cx| match event {
        // Escape keeps what was typed, as it does in Excel.
        AreaEvent::Escape => {
            finish_note_edit(app, cx);
            window.focus(&app.grid_focus);
        }
        AreaEvent::Blurred => finish_note_edit(app, cx),
        _ => {}
    });
    window.focus(&area.focus_handle(cx));
    let ui = app.ext_mut::<Ui>();
    ui.note_edit = Some(NoteEdit {
        sheet,
        cell,
        area,
        original,
        _subscription: subscription,
    });
    ui.note_sel = None;
    ui.hover = None;
    ui.rest_timer = None;
    ui.leave_timer = None;
    cx.notify();
}

/// Store the note being typed, if any: a new note, or the new text of an
/// old one, in one undo step.
fn finish_note_edit(app: &mut ExcelApp, cx: &mut Cx) {
    let Some(edit) = app.ext::<Ui>().is_some().then(|| app.ext_mut::<Ui>().note_edit.take()).flatten() else {
        return;
    };
    let text = edit.area.read(cx).text().to_string();
    if edit.original.as_deref() != Some(text.as_str())
        && app.wb.set_note(edit.sheet, edit.cell.0, edit.cell.1, user_name(), &text)
    {
        let label = match edit.original {
            Some(_) => Cmd::EditNote.label(),
            None => Cmd::NewNote.label(),
        };
        app.commit(label, cx);
    }
    cx.notify();
}

/// Select the box of a note: it shows its handles and takes the keyboard
/// (Delete removes the note, the arrows move the box).
fn select_box(app: &mut ExcelApp, cell: (u32, u32), window: &mut Window, cx: &mut Cx) {
    let ui = app.ext_mut::<Ui>();
    let handle = ui.box_focus.get_or_insert_with(|| cx.focus_handle()).clone();
    ui.note_sel = Some(cell);
    if ui._box_blur.is_none() {
        ui._box_blur = Some(cx.on_blur(&handle, window, |app, _, cx| {
            let ui = app.ext_mut::<Ui>();
            // A press on a zone hands the keyboard to the grid for the
            // time of the drag; the box gets it back at the release.
            if ui.drag.is_none() {
                ui.note_sel = None;
                cx.notify();
            }
        }));
    }
    window.focus(&handle);
    cx.notify();
}

/// A press on the border of a note box or on one of its handles.
fn start_drag(app: &mut ExcelApp, cell: (u32, u32), grip: Grip, ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) {
    // A box whose text is being typed keeps that text, then is selected.
    finish_note_edit(app, cx);
    if !may_edit(app, cx) {
        return;
    }
    let Some(note) = app.wb.comment_at(app.sheet, cell.0, cell.1).and_then(Comment::as_note) else {
        return;
    };
    let stays = note.visible;
    select_box(app, cell, window, cx);
    let Some(layout) = app.current_layout() else {
        return;
    };
    // A box that only shows for now may have been pushed back into view:
    // it starts from where it is seen.
    let origin = if stays {
        app.wb.sheets[app.sheet].note_frame(cell.0, cell.1)
    } else {
        notes::shown_boxes(app, &layout)
            .iter()
            .find(|shown| shown.cell == cell)
            .map(|shown| notes::frame_at(&layout, shown.rect))
    };
    if let Some(origin) = origin {
        app.ext_mut::<Ui>().drag = Some(BoxDrag {
            cell,
            grip,
            start: (ev.x, ev.y),
            origin,
            current: origin,
        });
    }
}

/// Move the selected box by a pixel with an arrow key.
fn nudge_box(app: &mut ExcelApp, dx: f64, dy: f64, cx: &mut Cx) {
    let Some((row, col)) = app.ext::<Ui>().and_then(|ui| ui.note_sel) else {
        return;
    };
    let sheet = app.sheet;
    let Some(frame) = app.wb.sheets[sheet].note_frame(row, col) else {
        return;
    };
    let moved = notes::dragged(frame, Grip::Move, dx, dy);
    if moved != frame && may_edit(app, cx) && app.wb.set_note_frame(sheet, row, col, Some(moved)) {
        app.commit(t("Move Object", "Déplacer l'objet"), cx);
    }
}

// ----------------------------------------------------------------- comments

/// Post a comment on a cell that has none, or a reply to its thread.
pub(self) fn post(app: &mut ExcelApp, sheet: usize, cell: (u32, u32), text: &str, cx: &mut Cx) -> bool {
    if !may_edit(app, cx) {
        return false;
    }
    let author = Person::new(user_name());
    let now = comment::now_utc();
    let (row, col) = cell;
    let (done, label) = match kind_at(app, sheet, cell) {
        Some(CommentKind::Thread) => (
            app.wb.add_reply(sheet, row, col, &author, now, text),
            t("Reply", "Répondre"),
        ),
        Some(CommentKind::Note) => return false,
        None => (
            app.wb.add_comment(sheet, row, col, &author, now, text),
            Cmd::NewComment.label(),
        ),
    };
    if done {
        app.commit(label, cx);
    }
    done
}

/// Change the text of a post (`reply` `None`: the first of its thread).
pub(self) fn edit_post(
    app: &mut ExcelApp,
    sheet: usize,
    cell: (u32, u32),
    reply: Option<usize>,
    text: &str,
    cx: &mut Cx,
) -> bool {
    let done = may_edit(app, cx) && app.wb.edit_comment(sheet, cell.0, cell.1, reply, text);
    if done {
        app.commit(t("Edit Comment", "Modifier le commentaire"), cx);
    }
    done
}

pub(self) fn delete_reply(app: &mut ExcelApp, sheet: usize, cell: (u32, u32), reply: usize, cx: &mut Cx) {
    if may_edit(app, cx) && app.wb.delete_reply(sheet, cell.0, cell.1, reply) {
        app.commit(Cmd::DeleteComment.label(), cx);
    }
}

pub(self) fn delete_thread(app: &mut ExcelApp, sheet: usize, cell: (u32, u32), cx: &mut Cx) {
    if may_edit(app, cx) && app.wb.delete_comment(sheet, cell.0, cell.1) {
        app.commit(Cmd::DeleteComment.label(), cx);
    }
}

/// Resolve a thread, or reopen it.
pub(self) fn resolve(app: &mut ExcelApp, sheet: usize, cell: (u32, u32), resolved: bool, cx: &mut Cx) {
    if may_edit(app, cx) && app.wb.set_comment_resolved(sheet, cell.0, cell.1, resolved) {
        let label = if resolved {
            t("Resolve thread", "Résoudre la conversation")
        } else {
            t("Reopen thread", "Rouvrir la conversation")
        };
        app.commit(label, cx);
    }
}

/// Put away the card floating beside a cell.
pub(self) fn close_card(app: &mut ExcelApp, sheet: usize, cell: (u32, u32), cx: &mut Cx) {
    let ui = app.ext_mut::<Ui>();
    if ui.card.as_ref().is_some_and(|card| (card.sheet, card.cell) == (sheet, cell)) {
        ui.card = None;
        ui.hover = None;
        cx.notify();
    }
}

/// A press on the card that only showed under the pointer: it stays.
pub(self) fn pin_card(app: &mut ExcelApp, sheet: usize, cell: (u32, u32), cx: &mut Cx) {
    let ui = app.ext_mut::<Ui>();
    if let Some(card) = ui.card.as_mut().filter(|card| (card.sheet, card.cell) == (sheet, cell)) {
        card.pinned = true;
        ui.leave_timer = None;
        cx.notify();
    }
}

/// The pointer went onto the floating card, or left it.
pub(self) fn pointer_on_card(app: &mut ExcelApp, inside: bool, cx: &mut Cx) {
    let ui = app.ext_mut::<Ui>();
    let Some(card) = ui.card.as_mut() else {
        return;
    };
    card.pointer_inside = inside;
    if inside {
        ui.leave_timer = None;
    } else if !card.pinned {
        linger(app, LINGER_CARD, cx);
    }
}

/// Show the comment card of a cell of the current sheet, and keep it
/// showing; `write` puts the cursor in its box. With the Comments pane
/// open, the card is the one of the pane.
fn open_card(app: &mut ExcelApp, cell: (u32, u32), write: bool, window: &mut Window, cx: &mut Cx) {
    let sheet = app.sheet;
    let composing = kind_at(app, sheet, cell).is_none();
    if app.pane_open(PANE) {
        if let Some(pane) = app.ext::<Ui>().and_then(|ui| ui.pane.clone()) {
            pane.update(cx, |pane, cx| {
                if write {
                    pane.write(sheet, cell, composing, window, cx);
                } else {
                    pane.reveal(cell, cx);
                }
            });
        }
        return;
    }
    // Words typed in the card of another cell are not thrown away.
    let elsewhere = |card: &&OpenCard| (card.sheet, card.cell) != (sheet, cell);
    if app
        .ext::<Ui>()
        .and_then(|ui| ui.card.as_ref())
        .filter(elsewhere)
        .is_some_and(|card| card.view.read(cx).has_draft(cx))
    {
        return;
    }
    let weak = cx.weak_entity();
    let ui = app.ext_mut::<Ui>();
    ui.rest_timer = None;
    ui.leave_timer = None;
    let view = match ui.card.as_ref().filter(|card| (card.sheet, card.cell) == (sheet, cell)) {
        Some(card) => card.view.clone(),
        None => cx.new(|cx| CommentCard::new(weak, sheet, cell, false, window, cx)),
    };
    ui.card = Some(OpenCard {
        sheet,
        cell,
        view: view.clone(),
        pinned: true,
        pointer_inside: false,
    });
    view.update(cx, |card, cx| {
        card.composing = composing;
        if write {
            card.focus_input(window, cx);
        }
    });
    cx.notify();
}

/// New Comment, Reply to Comment: the card of the active cell opens with
/// the cursor in its box.
fn write_comment(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let (sheet, cell) = (app.sheet, app.active());
    // Words typed on another cell and not posted: go back to them, or
    // drop them?
    let draft = app
        .ext::<Ui>()
        .and_then(|ui| ui.card.as_ref())
        .filter(|card| (card.sheet, card.cell) != (sheet, cell) && card.view.read(cx).has_draft(cx))
        .map(|card| (card.sheet, card.cell));
    if let Some((draft_sheet, draft_cell)) = draft {
        let question = t(
            "There's a comment that hasn't been posted.\nDo you want to return to it so you can post it, or do you want to discard it?",
            "Un commentaire n’a pas été publié.\nVoulez-vous retourner à ce dernier afin de le publier, ou voulez-vous l’abandonner ?",
        );
        app.alert(
            Alert::new("OpenSheet", question)
                .warning()
                .button(t("Return to comment", "Revenir au commentaire"), move |app, window, cx| {
                    app.switch_sheet(draft_sheet, window, cx);
                    app.select_cell(draft_cell.0, draft_cell.1, cx);
                    if let Some(view) = app.ext::<Ui>().and_then(|ui| ui.card.as_ref()).map(|card| card.view.clone()) {
                        view.update(cx, |card, cx| card.focus_input(window, cx));
                    }
                })
                .button(t("Discard comment", "Abandonner le commentaire"), |app, window, cx| {
                    app.ext_mut::<Ui>().card = None;
                    write_comment(app, window, cx);
                }),
            cx,
        );
        return;
    }
    match kind_at(app, sheet, cell) {
        // A comment on a cell that has a note: the note becomes the
        // first post of the thread, if the user agrees.
        Some(CommentKind::Note) => {
            let text = t(
                "To do that, we need to convert this note to a threaded comment so that other people can collaborate with you. However, converting will remove any images or formatting that may be inside in this note.",
                "Pour ce faire, nous devons convertir cette note en un commentaire thématique afin que d’autres personnes puissent collaborer avec vous. Toutefois, la conversion supprimera toutes les images et toutes les mises en forme de cette note.",
            );
            app.alert(
                Alert::new(t("Convert?", "Convertir ?"), text)
                    .button(t("Convert", "Convertir"), move |app, window, cx| {
                        finish_note_edit(app, cx);
                        if may_edit(app, cx) && app.wb.convert_note_to_comment(sheet, cell.0, cell.1, comment::now_utc()) {
                            app.commit(Cmd::ConvertToComments.label(), cx);
                            if (app.sheet, app.active()) == (sheet, cell) {
                                open_card(app, cell, true, window, cx);
                            }
                        }
                    })
                    .close_button(t("Cancel", "Annuler")),
                cx,
            );
        }
        here => {
            let sh = &app.wb.sheets[sheet];
            if here.is_none() && (sh.is_row_hidden(cell.0) || sh.hidden_cols.contains(&cell.1)) {
                let message = t(
                    "Comments can't be added to hidden cells",
                    "Vous ne pouvez pas ajouter de commentaires aux cellules masquées",
                );
                app.alert(Alert::new("OpenSheet", message).warning(), cx);
                return;
            }
            if may_edit(app, cx) {
                open_card(app, cell, true, window, cx);
            }
        }
    }
}

/// Open the Comments pane, or close it.
fn toggle_pane(app: &mut ExcelApp, cx: &mut Cx) {
    if app.pane_open(PANE) {
        app.close_pane(PANE, cx);
        return;
    }
    let weak = cx.weak_entity();
    let pane = cx.new(|_| CommentsPane::new(weak));
    let active = app.active();
    pane.update(cx, |pane, cx| pane.reveal(active, cx));
    // The pane shows the threads from now on: no card over the grid.
    let ui = app.ext_mut::<Ui>();
    ui.card = None;
    ui.pane = Some(pane.clone());
    app.open_pane(PANE, t("Comments", "Commentaires"), pane, cx);
}

/// Previous / Next: go to the nearest note (or comment) before or after
/// the active cell, through the whole workbook. At its end, Excel asks
/// whether to go on from the other end.
fn go(app: &mut ExcelApp, kind: CommentKind, forward: bool, window: &mut Window, cx: &mut Cx) {
    let (row, col) = app.active();
    let from = (app.sheet, row, col);
    let found = if forward {
        app.wb.next_comment(from, Some(kind))
    } else {
        app.wb.previous_comment(from, Some(kind))
    };
    if let Some(target) = found {
        visit(app, target, kind, window, cx);
        return;
    }
    let question = comment::end_of_workbook_question(app.wb.locale);
    app.alert(
        Alert::new("OpenSheet", question)
            .button(t("OK", "OK"), move |app, window, cx| {
                let target = if forward {
                    app.wb.first_comment(Some(kind))
                } else {
                    app.wb.last_comment(Some(kind))
                };
                if let Some(target) = target {
                    visit(app, target, kind, window, cx);
                }
            })
            .close_button(t("Cancel", "Annuler")),
        cx,
    );
}

/// Select an annotated cell and show what it carries.
fn visit(app: &mut ExcelApp, target: (usize, u32, u32), kind: CommentKind, window: &mut Window, cx: &mut Cx) {
    let (sheet, row, col) = target;
    app.switch_sheet(sheet, window, cx);
    if app.sheet != sheet {
        return;
    }
    app.select_cell(row, col, cx);
    if app.active() != (row, col) {
        return;
    }
    match kind {
        CommentKind::Note => {
            app.ext_mut::<Ui>().visited = Some((row, col));
            cx.notify();
        }
        CommentKind::Thread => open_card(app, (row, col), false, window, cx),
    }
}

// -------------------------------------------------------------------- hover

/// The pointer is on an annotated cell: once it has rested there, the
/// note or the comment shows.
fn pointer_on(app: &mut ExcelApp, cell: (u32, u32), cx: &mut Cx) {
    let ui = app.ext_mut::<Ui>();
    ui.leave_timer = None;
    if ui.hover.as_ref().is_some_and(|hover| hover.cell == cell) {
        return;
    }
    ui.hover = Some(Hover { cell, shown: false });
    if ui.card.as_ref().is_some_and(|card| !card.pinned) {
        ui.card = None;
    }
    ui.rest_timer = Some(cx.spawn(async move |this, cx| {
        cx.background_executor().timer(REST).await;
        this.update(cx, |app, cx| {
            let ui = app.ext_mut::<Ui>();
            ui.rest_timer = None;
            if let Some(hover) = ui.hover.as_mut().filter(|hover| hover.cell == cell) {
                hover.shown = true;
                cx.notify();
            }
        })
        .ok();
    }));
    cx.notify();
}

/// The pointer left the annotated cell it was on.
fn pointer_off(app: &mut ExcelApp, cx: &mut Cx) {
    let sheet = app.sheet;
    let Some(ui) = app.ext::<Ui>() else {
        return;
    };
    let Some(hover) = &ui.hover else {
        return;
    };
    if !hover.shown {
        let ui = app.ext_mut::<Ui>();
        ui.hover = None;
        ui.rest_timer = None;
        return;
    }
    let delay = match kind_at(app, sheet, hover.cell) {
        Some(CommentKind::Thread) => LINGER_CARD,
        _ => LINGER_NOTE,
    };
    linger(app, delay, cx);
}

/// Take away what the pointer made appear, after a moment: the pointer
/// may come back, or reach the card.
fn linger(app: &mut ExcelApp, delay: Duration, cx: &mut Cx) {
    app.ext_mut::<Ui>().leave_timer = Some(cx.spawn(async move |this, cx| {
        cx.background_executor().timer(delay).await;
        this.update(cx, |app, cx| {
            let ui = app.ext_mut::<Ui>();
            ui.leave_timer = None;
            if ui.card.as_ref().is_some_and(|card| card.pointer_inside && !card.pinned) {
                return;
            }
            ui.hover = None;
            if ui.card.as_ref().is_some_and(|card| !card.pinned) {
                ui.card = None;
            }
            cx.notify();
        })
        .ok();
    }));
}

// ----------------------------------------------------------------- painting

/// The corner marks of the cells in view, and the zones that make those
/// cells show their note or their comment under the pointer.
fn paint_marks(p: &mut Painter) {
    let sheet = &p.app.wb.sheets[p.app.sheet];
    if sheet.comments.is_empty() {
        return;
    }
    let z = p.zoom();
    let visible = p.visible;
    // The comment of merged cells sits on the first of them, which may be
    // above the rows in view.
    let first_row = sheet
        .merges
        .iter()
        .filter(|merge| merge.intersects(&visible))
        .map(|merge| merge.r0)
        .min()
        .map_or(visible.r0, |row| row.min(visible.r0));
    for ((row, col), comment) in sheet.comments.range((first_row, 0)..=(visible.r1, u32::MAX)) {
        let area = sheet.merge_at(*row, *col).unwrap_or(Rect::cell(*row, *col));
        if !area.intersects(&visible) {
            continue;
        }
        let (x0, y0, x1, y1) = p.cell_box(area);
        if x1 - x0 < 3.0 || y1 - y0 < 3.0 {
            continue;
        }
        // Inside the gridline at the right of the cell.
        let right = x1 - 1.0;
        match comment {
            Comment::Note(_) => {
                let size = (6.0 * z).round().clamp(4.0, 14.0).min(y1 - y0);
                p.polygon(&[(right - size, y0), (right, y0), (right, y0 + size)], c(NOTE_MARK));
            }
            Comment::Thread(_) => {
                let size = (8.0 * z).round().clamp(5.0, 18.0).min(y1 - y0);
                let cut = (size * 0.45).round();
                p.polygon(
                    &[
                        (right - size, y0),
                        (right, y0),
                        (right, y0 + size),
                        (right - size + cut, y0 + size),
                        (right - size, y0 + size - cut),
                    ],
                    c(COMMENT_MARK),
                );
            }
        }
        p.zone((x0, y0, x1, y1), Spot::Cell(*row, *col));
    }
}

/// The note boxes that show, over everything else on the grid.
fn paint_boxes(p: &mut Painter) {
    let app = p.app;
    let sheet = &app.wb.sheets[app.sheet];
    if sheet.comments.is_empty() && app.ext::<Ui>().is_none_or(|ui| ui.note_edit.is_none()) {
        return;
    }
    for shown in notes::shown_boxes(app, p.layout) {
        let (x0, y0, x1, y1) = shown.rect;
        let clip = shown.clip;
        if x1 + 8.0 < clip.0 || y1 + 8.0 < clip.1 || x0 - 8.0 > clip.2 || y0 - 8.0 > clip.3 {
            continue;
        }
        let text = sheet
            .comments
            .get(&shown.cell)
            .and_then(Comment::as_note)
            .map(|note| notes::note_text(note, &app.fonts));
        notes::paint_box(p, &shown, notes::cell_px(p.layout, sheet, shown.cell), text);
        if shown.solid {
            notes::zones(p, &shown);
        }
    }
}

/// What the status bar says of the annotated cell under the pointer.
fn status_text(cell: &str, comment: &Comment) -> String {
    let (pattern, author, replies) = match comment {
        Comment::Note(note) => (
            t("Cell |0 note by |1", "Cellule |0 annotée par |1"),
            note.author.as_str(),
            0,
        ),
        Comment::Thread(thread) => (
            match thread.replies.len() {
                0 => t("Cell |0 commented by |1", "Cellule |0 commentée par |1"),
                1 => t("Cell |0 commented by |1 (1 reply)", "Cellule |0 commentée par |1 (1 réponse)"),
                _ => t(
                    "Cell |0 commented by |1 (|2 replies)",
                    "Cellule |0 commentée par |1 (|2 réponses)",
                ),
            },
            thread.first.author.name.as_str(),
            thread.replies.len(),
        ),
    };
    pattern
        .replace("|0", cell)
        .replace("|1", author)
        .replace("|2", &replies.to_string())
}

// ----------------------------------------------------------------- elements

/// Forget what no longer exists (deleted, undone, another sheet).
fn tidy(app: &mut ExcelApp, cx: &mut Cx) {
    let Some(ui) = app.ext::<Ui>() else {
        return;
    };
    let sheet = app.sheet;
    let is = |cell: (u32, u32), kind: CommentKind| kind_at(app, sheet, cell) == Some(kind);
    let gone_card = ui.card.as_ref().is_some_and(|card| {
        card.sheet == sheet && !is(card.cell, CommentKind::Thread) && !card.view.read(cx).composing
    });
    let gone_sel = ui.note_sel.is_some_and(|cell| !is(cell, CommentKind::Note));
    let gone_visit = ui.visited.is_some_and(|cell| !is(cell, CommentKind::Note));
    let gone_hover = ui.hover.as_ref().is_some_and(|hover| kind_at(app, sheet, hover.cell).is_none());
    if !(gone_card || gone_sel || gone_visit || gone_hover) {
        return;
    }
    let ui = app.ext_mut::<Ui>();
    if gone_card {
        ui.card = None;
    }
    if gone_sel {
        ui.note_sel = None;
        ui.drag = None;
    }
    if gone_visit {
        ui.visited = None;
    }
    if gone_hover {
        ui.hover = None;
        ui.rest_timer = None;
    }
}

/// The text box laid over the note box whose text is being typed.
fn note_editor(app: &mut ExcelApp, layout: &GridLayout, cx: &mut Cx) -> Option<AnyElement> {
    let edit = app.ext::<Ui>()?.note_edit.as_ref().filter(|edit| edit.sheet == app.sheet)?;
    let (cell, area) = (edit.cell, edit.area.clone());
    let shown = notes::shown_boxes(app, layout).into_iter().find(|shown| shown.cell == cell)?;
    let z = layout.zoom;
    let (x0, y0, x1, y1) = shown.rect;
    let inner = (
        x0 + (notes::INSET.0 * z).round(),
        y0 + (notes::INSET.1 * z).round(),
        x1 - (notes::INSET.0 * z).round(),
        y1 - 1.0,
    );
    // Only the part of the box its pane shows.
    let view = notes::intersect(inner, shown.clip)?;
    let font = Note::font();
    let size = pt_to_px(font.size_pt());
    let family = app.fonts.resolve(&font.name);
    area.update(cx, |area, _| area.max_height = Some(inner.3 - inner.1));
    let clicked = area.clone();
    Some(
        div()
            .absolute()
            .left(px(view.0))
            .top(px(view.1))
            .w(px(view.2 - view.0))
            .h(px(view.3 - view.1))
            .overflow_hidden()
            .occlude()
            .cursor(CursorStyle::IBeam)
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
                clicked.update(cx, |area, cx| area.caret_to_point(event.position, cx));
            })
            .child(
                div()
                    .absolute()
                    .left(px(inner.0 - view.0))
                    .top(px(inner.1 - view.1))
                    .w(px(inner.2 - inner.0))
                    .h(px(inner.3 - inner.1))
                    .font_family(family)
                    .text_size(px(size * z))
                    .line_height(px((notes::line_height(size) * z).round()))
                    .text_color(c(0x000000))
                    .child(area),
            )
            .into_any_element(),
    )
}

/// What takes the keys while a note box is selected: Delete removes the
/// note, the arrows move the box, Return opens it, Escape lets go of it.
fn box_keys(app: &mut ExcelApp, cx: &mut Cx) -> Option<AnyElement> {
    let ui = app.ext::<Ui>()?;
    let cell = ui.note_sel?;
    let handle = ui.box_focus.clone()?;
    let delete = move |app: &mut ExcelApp, cx: &mut Cx| {
        let sheet = app.sheet;
        if may_edit(app, cx) && app.wb.delete_comment(sheet, cell.0, cell.1) {
            app.commit(Cmd::DeleteNote.label(), cx);
        }
    };
    Some(
        div()
            .absolute()
            .size_0()
            .track_focus(&handle)
            .key_context(line::CONTEXT)
            .on_action(cx.listener(move |app, _: &line::Backspace, window, cx| {
                delete(app, cx);
                window.focus(&app.grid_focus);
            }))
            .on_action(cx.listener(move |app, _: &line::Delete, window, cx| {
                delete(app, cx);
                window.focus(&app.grid_focus);
            }))
            .on_action(cx.listener(|app, _: &line::Escape, window, _| window.focus(&app.grid_focus)))
            .on_action(cx.listener(move |app, _: &line::Enter, window, cx| edit_note(app, cell, None, window, cx)))
            .on_action(cx.listener(|app, _: &line::Left, _, cx| nudge_box(app, -1.0, 0.0, cx)))
            .on_action(cx.listener(|app, _: &line::Right, _, cx| nudge_box(app, 1.0, 0.0, cx)))
            .on_action(cx.listener(|app, _: &line::Up, _, cx| nudge_box(app, 0.0, -1.0, cx)))
            .on_action(cx.listener(|app, _: &line::Down, _, cx| nudge_box(app, 0.0, 1.0, cx)))
            .into_any_element(),
    )
}

/// The comment card beside its cell: the one a click or a command
/// opened, or the one of the comment the pointer rests on.
fn floating_card(app: &mut ExcelApp, layout: &GridLayout, window: &mut Window, cx: &mut Cx) -> Option<AnyElement> {
    if app.pane_open(PANE) || app.is_editing() {
        return None;
    }
    let sheet = app.sheet;
    let ui = app.ext::<Ui>()?;
    if ui.card.is_none() {
        let cell = ui.hover.as_ref().filter(|hover| hover.shown).map(|hover| hover.cell)?;
        if kind_at(app, sheet, cell) != Some(CommentKind::Thread) {
            return None;
        }
        let weak = cx.weak_entity();
        let view = cx.new(|cx| CommentCard::new(weak, sheet, cell, false, window, cx));
        app.ext_mut::<Ui>().card = Some(OpenCard {
            sheet,
            cell,
            view,
            pinned: false,
            pointer_inside: false,
        });
    }
    let card = app.ext::<Ui>()?.card.as_ref().filter(|card| card.sheet == sheet)?;
    let (x0, y0, x1, y1) = notes::cell_px(layout, &app.wb.sheets[sheet], card.cell);
    let area = (
        layout.header_w,
        layout.header_h,
        layout.width() - theme::SCROLLBAR,
        layout.height() - theme::SCROLLBAR,
    );
    // Not while its cell is scrolled out of view.
    if x1 <= area.0 || x0 >= area.2 || y1 <= area.1 || y0 >= area.3 {
        return None;
    }
    let limits = layout.bounds_px(area.0 + 4.0, area.1 + 4.0, area.2 - 4.0, area.3 - 4.0);
    let view = card.view.clone();
    view.update(cx, |card, _| card.set_room(f32::from(limits.size.height)));
    Some(Beside::new(view, layout.bounds_px(x0, y0, x1, y1), limits).into_any_element())
}

// ------------------------------------------------------------------ feature

impl Feature for Comments {
    fn name(&self) -> &'static str {
        "comments"
    }

    fn paint(&self, p: &mut Painter) {
        match p.layer {
            PaintLayer::Under => {}
            PaintLayer::Over => paint_marks(p),
            PaintLayer::Top => paint_boxes(p),
        }
    }

    fn overlay_elements(&self, app: &mut ExcelApp, layout: &GridLayout, window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
        tidy(app, cx);
        [
            note_editor(app, layout, cx),
            box_keys(app, cx),
            floating_card(app, layout, window, cx),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    fn zone_mouse_down(&self, app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) -> ZoneResponse {
        let Some(spot) = zone.payload::<Spot>() else {
            return ZoneResponse::Ignored;
        };
        let left = ev.button == MouseButton::Left;
        match *spot {
            // The press goes on to the cell; a plain click on a comment
            // also opens its card at once, and keeps it open.
            Spot::Cell(row, col) => {
                let plain = left && !ev.modifiers.shift && !ev.modifiers.platform && !ev.modifiers.control;
                let thread = kind_at(app, app.sheet, (row, col)) == Some(CommentKind::Thread);
                if plain && thread && !app.is_editing() && !app.pane_open(PANE) {
                    if ev.click_count == 1 {
                        open_card(app, (row, col), false, window, cx);
                    } else {
                        // A double click is for the cell: it is typed in.
                        close_card(app, app.sheet, (row, col), cx);
                    }
                }
                ZoneResponse::Ignored
            }
            Spot::Body(row, col) => {
                if left {
                    edit_note(app, (row, col), Some(ev.position), window, cx);
                }
                ZoneResponse::Handled
            }
            Spot::Grip(row, col, grip) => {
                if left {
                    start_drag(app, (row, col), grip, ev, window, cx);
                }
                match app.ext::<Ui>().is_some_and(|ui| ui.drag.is_some()) {
                    true => ZoneResponse::Capture,
                    false => ZoneResponse::Handled,
                }
            }
        }
    }

    fn zone_drag(&self, app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, _window: &mut Window, _cx: &mut Cx) {
        if zone.payload::<Spot>().is_none() {
            return;
        }
        let z = app.zoom as f64;
        if let Some(drag) = &mut app.ext_mut::<Ui>().drag {
            let (dx, dy) = ((ev.x - drag.start.0) as f64 / z, (ev.y - drag.start.1) as f64 / z);
            drag.current = notes::dragged(drag.origin, drag.grip, dx, dy);
        }
    }

    fn zone_mouse_up(&self, app: &mut ExcelApp, zone: &Zone, _ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) {
        if zone.payload::<Spot>().is_none() {
            return;
        }
        let ui = app.ext_mut::<Ui>();
        let Some(drag) = ui.drag.take() else {
            return;
        };
        // The box that was pressed keeps the keyboard.
        if let Some(handle) = &ui.box_focus {
            window.focus(handle);
        }
        let (row, col) = drag.cell;
        if drag.current != drag.origin && app.wb.set_note_frame(app.sheet, row, col, Some(drag.current)) {
            let label = match drag.grip {
                Grip::Move => t("Move Object", "Déplacer l'objet"),
                Grip::Handle(_) => t("Size Object", "Dimensionner l'objet"),
            };
            app.commit(label, cx);
        }
    }

    fn zone_hover(&self, app: &mut ExcelApp, zone: Option<&Zone>, cx: &mut Cx) {
        match zone.and_then(|zone| zone.payload::<Spot>()) {
            Some(Spot::Cell(row, col)) => pointer_on(app, (*row, *col), cx),
            _ => pointer_off(app, cx),
        }
    }

    fn selection_changed(&self, app: &mut ExcelApp, cx: &mut Cx) {
        if app.ext::<Ui>().is_none() {
            return;
        }
        // Whatever was being typed in a note is kept.
        finish_note_edit(app, cx);
        let here = (app.sheet, app.active());
        let pane = app.pane_open(PANE);
        let ui = app.ext_mut::<Ui>();
        ui.visited = None;
        ui.note_sel = None;
        ui.drag = None;
        // A card goes with the selection, unless it holds words that
        // were not posted yet.
        if ui
            .card
            .as_ref()
            .is_some_and(|card| (card.sheet, card.cell) != here && !card.view.read(cx).has_draft(cx))
        {
            ui.card = None;
        }
        if let Some(pane) = ui.pane.clone().filter(|_| pane) {
            pane.update(cx, |pane, cx| pane.reveal(here.1, cx));
        }
    }

    fn context_menu(&self, app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
        if !matches!(target, MenuTarget::Cell(..)) {
            return;
        }
        // Like the ribbon, the menu speaks of the active cell.
        for cmd in cell_menu(kind_at(app, app.sheet, app.active())) {
            let entry = MenuEntry::command(*cmd, app);
            menu.add(
                MenuSection::Review,
                match cmd {
                    Cmd::NewNote | Cmd::EditNote => entry.shortcut("⇧F2"),
                    Cmd::NewComment => entry.shortcut("⇧⌘M"),
                    _ => entry,
                },
            );
        }
    }

    fn status_items(&self, app: &ExcelApp) -> Vec<StatusItem> {
        let hover = app.ext::<Ui>().and_then(|ui| ui.hover.as_ref()).filter(|hover| hover.shown);
        let Some((row, col)) = hover.map(|hover| hover.cell) else {
            return Vec::new();
        };
        match app.wb.comment_at(app.sheet, row, col) {
            Some(comment) => vec![StatusItem::new(status_text(&cell_name(row, col), comment))],
            None => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::comment::{Post, Thread};

    #[test]
    fn menus_follow_what_the_cell_has() {
        // A bare cell can get either kind.
        assert_eq!(cell_menu(None), [Cmd::NewComment, Cmd::NewNote]);
        // A note is edited, deleted, shown; a comment on it converts it.
        assert_eq!(
            cell_menu(Some(CommentKind::Note)),
            [Cmd::NewComment, Cmd::EditNote, Cmd::DeleteNote, Cmd::ShowHideNote]
        );
        // A comment is answered or deleted, and takes no note.
        assert_eq!(cell_menu(Some(CommentKind::Thread)), [Cmd::Reply, Cmd::DeleteComment]);

        assert_eq!(notes_menu(None)[0], Cmd::NewNote);
        assert_eq!(notes_menu(Some(CommentKind::Thread))[0], Cmd::NewNote);
        assert_eq!(
            notes_menu(Some(CommentKind::Note)),
            [
                Cmd::EditNote,
                Cmd::PreviousNote,
                Cmd::NextNote,
                Cmd::ShowHideNote,
                Cmd::ShowAllNotes,
                Cmd::ConvertToComments
            ]
        );
    }

    #[test]
    fn status_bar_names_the_author() {
        let post = |name: &str| Post {
            author: Person::new(name),
            time: 45_000.5,
            text: "x".into(),
        };
        let note = Comment::Note(Note::new("Ada", "Ada:\nCheck"));
        assert_eq!(status_text("B2", &note), "Cell B2 note by Ada");
        let mut thread = Thread {
            first: post("Bob"),
            replies: Vec::new(),
            resolved: false,
        };
        assert_eq!(status_text("C3", &Comment::Thread(thread.clone())), "Cell C3 commented by Bob");
        thread.replies.push(post("Ada"));
        assert_eq!(
            status_text("C3", &Comment::Thread(thread.clone())),
            "Cell C3 commented by Bob (1 reply)"
        );
        thread.replies.push(post("Cy"));
        assert_eq!(
            status_text("C3", &Comment::Thread(thread)),
            "Cell C3 commented by Bob (2 replies)"
        );
    }
}
