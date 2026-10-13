//! Notes and comments attached to cells (`Sheet::comments`).
//!
//! Excel has two kinds and a cell holds one of them at most: a *note* is
//! the yellow box of old (an author, formatted text, a box that can be
//! left showing); a *comment* is a conversation (who said what and when,
//! replies, resolved or not). Both travel with their cell.

use crate::ast::StructuralEdit;
use crate::datetime::{date_to_serial, serial_to_ymd, time_to_serial};
use crate::drawing::{Corner, Frame, columns, moved_corner, rows};
use crate::features::{CellShift, tiled};
use crate::locale::Locale;
use crate::refs::{MAX_COLS, MAX_ROWS, Rect};
use crate::style::Font;
use crate::validation::localized;
use crate::workbook::{Sheet, Workbook};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::ops::Bound::{Excluded, Unbounded};
use std::sync::Arc;

// ---- notes ---------------------------------------------------------------

/// A stretch of the text of a note in one font (`<r>` of the file).
#[derive(Clone, Debug, PartialEq)]
pub struct NoteRun {
    pub text: String,
    pub font: Font,
}

/// Where the box of a note sits, like a chart: a cell (row, col) holding
/// its top-left corner, an offset inside that cell and a size, in pixels
/// at 100 %.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteBox {
    pub anchor: (u32, u32),
    pub offset: (f32, f32),
    pub size: (f32, f32),
}

/// The size of the box of a new note (108 × 59.25 points).
pub const NOTE_SIZE: (f32, f32) = (144.0, 79.0);

/// A note: the yellow box shown when the pointer rests on its cell.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub author: String,
    /// The text, in runs of one font each. Excel starts a note with the
    /// name of its author in bold; that line is part of the text.
    pub runs: Vec<NoteRun>,
    /// The box stays on the sheet ("Show/Hide Note") instead of showing
    /// only under the pointer.
    pub visible: bool,
    /// `None`: the box sits where Excel first puts it, next to its cell
    /// (`Sheet::note_frame`), and follows the cell wherever it goes.
    pub frame: Option<NoteBox>,
}

impl Note {
    /// The font of notes: Tahoma, 9 points.
    pub fn font() -> Font {
        Font {
            name: Arc::from("Tahoma"),
            size: 18,
            ..Font::default()
        }
    }

    /// What a new note holds before anything is typed: the name of its
    /// author on a line of its own.
    pub fn template(author: &str) -> String {
        if author.is_empty() {
            String::new()
        } else {
            format!("{author}:\n")
        }
    }

    /// A note with the given text; the author's name at its start is set
    /// in bold, as Excel does.
    pub fn new(author: &str, text: &str) -> Note {
        let mut note = Note {
            author: author.to_string(),
            runs: Vec::new(),
            visible: false,
            frame: None,
        };
        note.set_text(text);
        note
    }

    /// The whole text.
    pub fn text(&self) -> String {
        self.runs.iter().map(|run| run.text.as_str()).collect()
    }

    /// Replace the text. Fonts are not kept: the result is laid out like
    /// a new note.
    pub fn set_text(&mut self, text: &str) {
        let regular = Note::font();
        let header = format!("{}:", self.author);
        let run = |text: &str, bold: bool| NoteRun {
            text: text.to_string(),
            font: Font {
                bold,
                ..regular.clone()
            },
        };
        self.runs = match text.strip_prefix(&header) {
            Some(rest) if !self.author.is_empty() => vec![run(&header, true), run(rest, false)],
            _ => vec![run(text, false)],
        };
        self.runs.retain(|run| !run.text.is_empty());
    }

    /// The text without the line naming the author.
    fn body(&self) -> String {
        let text = self.text();
        let body = match text.strip_prefix(&format!("{}:", self.author)) {
            Some(rest) if !self.author.is_empty() => rest,
            _ => &text,
        };
        body.trim_start_matches(['\r', '\n']).to_string()
    }

    /// The comment thread this note becomes: its text without formatting
    /// or the line naming the author, who is the author of the comment,
    /// dated `time`.
    fn as_thread(&self, time: f64) -> Thread {
        Thread {
            first: Post::new(&Person::new(&self.author), time, &self.body()),
            replies: Vec::new(),
            resolved: false,
        }
    }
}

// ---- comments ------------------------------------------------------------

/// Who wrote a comment (`<person>` of the file).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Person {
    /// `displayName`.
    pub name: String,
    /// `userId`: the account, as the provider knows it.
    pub user_id: String,
    /// `providerId`: "None" for people without an account, "AD" or
    /// "Windows Live" for those signed in with Microsoft.
    pub provider_id: String,
}

impl Person {
    /// Someone known by name only.
    pub fn new(name: &str) -> Person {
        Person {
            name: name.to_string(),
            user_id: name.to_string(),
            provider_id: "None".to_string(),
        }
    }

    /// Someone named in a comment by an e-mail address, as Excel records
    /// the people it takes from the directory of an organisation: the
    /// address is the account and the provider is the people picker.
    pub fn mentioned(name: &str, email: &str) -> Person {
        Person {
            name: name.to_string(),
            user_id: email.to_string(),
            provider_id: "PeoplePicker".to_string(),
        }
    }
}

/// Someone named in a post (`@Ann Lee`): who, and where the name stands
/// in the text (`<mention>` of the file).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mention {
    pub person: Person,
    /// Where the mention starts in the text of its post, at the `@`: a
    /// byte offset.
    pub start: usize,
    /// Its length in bytes, the `@` included.
    pub len: usize,
}

impl Mention {
    /// The bytes of the text the mention covers.
    pub fn range(&self) -> std::ops::Range<usize> {
        self.start..self.start + self.len
    }
}

/// One contribution to a thread: the comment that starts it, or a reply.
#[derive(Clone, Debug, PartialEq)]
pub struct Post {
    pub author: Person,
    /// When it was written: a date-time serial in UTC (`dT` in the file,
    /// see `time_to_iso`). Show it with `utc_to_local`.
    pub time: f64,
    pub text: String,
    /// The people the text names, in the order of the text; they never
    /// overlap.
    pub mentions: Vec<Mention>,
}

impl Post {
    /// A post that names nobody.
    pub fn new(author: &Person, time: f64, text: &str) -> Post {
        Post {
            author: author.clone(),
            time,
            text: text.to_string(),
            mentions: Vec::new(),
        }
    }

    /// The same post naming these people. Mentions that do not fit the
    /// text (past its end, inside a character, over one another, empty)
    /// are left out, and the others are put in the order of the text.
    pub fn with_mentions(mut self, mut mentions: Vec<Mention>) -> Post {
        mentions.sort_by_key(|mention| mention.start);
        let mut end = 0;
        mentions.retain(|mention| {
            let range = mention.range();
            let fits = mention.len > 0
                && range.start >= end
                && range.end <= self.text.len()
                && self.text.is_char_boundary(range.start)
                && self.text.is_char_boundary(range.end);
            if fits {
                end = range.end;
            }
            fits
        });
        self.mentions = mentions;
        self
    }

    /// The text cut where mentions start and end: each piece with the
    /// person it names, if it is a mention.
    pub fn pieces(&self) -> Vec<(&str, Option<&Person>)> {
        let mut out = Vec::new();
        let mut at = 0;
        for mention in &self.mentions {
            let range = mention.range();
            if range.start < at || self.text.get(range.clone()).is_none() {
                continue;
            }
            if range.start > at {
                out.push((&self.text[at..range.start], None));
            }
            out.push((&self.text[range.clone()], Some(&mention.person)));
            at = range.end;
        }
        if at < self.text.len() {
            out.push((&self.text[at..], None));
        }
        out
    }
}

/// A comment and the replies to it.
#[derive(Clone, Debug, PartialEq)]
pub struct Thread {
    pub first: Post,
    /// In the order they were written.
    pub replies: Vec<Post>,
    /// "Resolved" (`done`): closed to replies and edits until reopened.
    pub resolved: bool,
}

impl Thread {
    /// Every post, the first comment included.
    pub fn posts(&self) -> impl Iterator<Item = &Post> {
        std::iter::once(&self.first).chain(self.replies.iter())
    }

    /// The text of the note that stands for the thread in applications
    /// that only know notes; Excel writes it next to every thread, under
    /// an author named `tc=` followed by the id of the thread.
    pub fn legacy_text(&self, locale: &Locale) -> String {
        let mut out = localized(
            locale,
            [
                "[Threaded comment]\n\nYour version of Excel allows you to read this threaded comment; however, any edits to it will get removed if the file is opened in a newer version of Excel. Learn more: https://go.microsoft.com/fwlink/?linkid=870924\n\n",
                "[Commentaire à thread]\n\nVotre version d’Excel vous permet de lire ce commentaire à thread. Toutefois, les modifications qui y sont apportées seront supprimées si le fichier est ouvert dans une version plus récente d’Excel. En savoir plus\u{a0}: https://go.microsoft.com/fwlink/?linkid=870924\n\n",
                "[Comentario encadenado]\n\nSu versión de Excel le permite leer este comentario encadenado; sin embargo, las ediciones que se apliquen se quitarán si el archivo se abre en una versión más reciente de Excel. Más información: https://go.microsoft.com/fwlink/?linkid=870924\n\n",
                "[Kommentarthread]\n\nIhre Version von Excel gestattet Ihnen das Lesen dieses Kommentarthreads. Jegliche Bearbeitungen daran werden jedoch entfernt, wenn die Datei in einer neueren Version von Excel geöffnet wird. Weitere Informationen: https://go.microsoft.com/fwlink/?linkid=870924.\n\n",
                "[Commento in thread]\n\nLa versione di Excel in uso consente di leggere questo commento in thread, ma tutte le modifiche a esso apportate verranno rimosse se il file viene aperto in una versione più recente di Excel. Ulteriori informazioni: https://go.microsoft.com/fwlink/?linkid=870924\n\n",
            ],
        )
        .to_string();
        out.push_str(localized(
            locale,
            [
                "Comment:\n    ",
                "Commentaire\u{a0}:\n    ",
                "Comentario:\n    ",
                "Kommentar:\n    ",
                "Commento:\n",
            ],
        ));
        out.push_str(&self.first.text);
        for reply in &self.replies {
            out.push_str(localized(
                locale,
                [
                    "\nReply:\n    ",
                    "\nRéponse\u{a0}:\n    ",
                    "\nRespuesta:\n    ",
                    "\nAntwort:\n    ",
                    "\nRispondi:\n",
                ],
            ));
            out.push_str(&reply.text);
        }
        out
    }
}

/// The two kinds of annotation Excel has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommentKind {
    Note,
    Thread,
}

/// The note or the comment thread of one cell.
#[derive(Clone, Debug, PartialEq)]
pub enum Comment {
    Note(Note),
    Thread(Thread),
}

impl Comment {
    pub fn kind(&self) -> CommentKind {
        match self {
            Comment::Note(_) => CommentKind::Note,
            Comment::Thread(_) => CommentKind::Thread,
        }
    }

    pub fn as_note(&self) -> Option<&Note> {
        match self {
            Comment::Note(note) => Some(note),
            Comment::Thread(_) => None,
        }
    }

    pub fn as_thread(&self) -> Option<&Thread> {
        match self {
            Comment::Thread(thread) => Some(thread),
            Comment::Note(_) => None,
        }
    }
}

// ---- time ----------------------------------------------------------------

fn serial_of(moment: &chrono::NaiveDateTime) -> f64 {
    use chrono::{Datelike, Timelike};
    let day = date_to_serial(moment.year() as i64, moment.month() as i64, moment.day() as i64).unwrap_or(0.0);
    let seconds = moment.second() as f64 + moment.nanosecond() as f64 / 1e9;
    day + time_to_serial(moment.hour() as f64, moment.minute() as f64, seconds)
}

/// Year, month, day, then the time of day in hundredths of a second (the
/// precision files keep).
fn parts(serial: f64) -> (i64, u32, u32, u64) {
    let mut day = serial.floor();
    let mut hundredths = ((serial - day) * 8_640_000.0).round() as u64;
    if hundredths >= 8_640_000 {
        day += 1.0;
        hundredths = 0;
    }
    let (year, month, date) = serial_to_ymd(day);
    (year, month, date, hundredths)
}

/// The present moment as a date-time serial in UTC: the `time` of a new
/// comment.
pub fn now_utc() -> f64 {
    serial_of(&chrono::Utc::now().naive_utc())
}

/// A UTC serial as the same moment on the clocks of this computer, for
/// display.
pub fn utc_to_local(serial: f64) -> f64 {
    use chrono::TimeZone;
    let (year, month, day, hundredths) = parts(serial);
    let (seconds, milli) = ((hundredths / 100) as u32, (hundredths % 100) as u32 * 10);
    chrono::NaiveDate::from_ymd_opt(year as i32, month, day)
        .and_then(|date| date.and_hms_milli_opt(seconds / 3600, seconds / 60 % 60, seconds % 60, milli))
        .map_or(serial, |utc| {
            serial_of(&chrono::Local.from_utc_datetime(&utc).naive_local())
        })
}

/// The `dT` attribute of a threaded comment: `2024-03-01T09:30:15.25`.
pub fn time_to_iso(serial: f64) -> String {
    let (year, month, day, hundredths) = parts(serial);
    let seconds = hundredths / 100;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60,
        hundredths % 100
    )
}

/// Read a `dT` attribute; `None` when it is not a date and a time.
pub fn time_from_iso(text: &str) -> Option<f64> {
    let (date, time) = text.trim().trim_end_matches('Z').split_once('T')?;
    let mut date = date.split('-').map(|part| part.parse::<i64>().ok());
    let (year, month, day) = (date.next()??, date.next()??, date.next()??);
    let mut time = time.split(':').map(|part| part.parse::<f64>().ok());
    let (hours, minutes, seconds) = (time.next()??, time.next()??, time.next().flatten().unwrap_or(0.0));
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hours >= 24.0 || minutes >= 60.0 || seconds >= 61.0 {
        return None;
    }
    Some(date_to_serial(year, month, day)? + time_to_serial(hours, minutes, seconds))
}

/// What Excel asks when Next or Previous runs out of comments.
pub fn end_of_workbook_question(locale: &Locale) -> &'static str {
    localized(
        locale,
        [
            "Microsoft Excel reached the end of the workbook. Do you want to continue reviewing from the beginning of the workbook?",
            "Microsoft Excel a atteint la fin du classeur. Voulez-vous poursuivre la révision depuis le début du classeur\u{a0}?",
            "Microsoft Excel llegó al final del libro. ¿Desea continuar revisando desde el comienzo del libro?",
            "Microsoft Excel hat das Ende der Arbeitsmappe erreicht. Soll mit der Überprüfung am Anfang der Mappe fortgefahren werden?",
            "Raggiunta la fine della cartella di lavoro. Continuare la revisione dall'inizio?",
        ],
    )
}

/// What Excel asks before "Convert to Comments": a title and a text.
pub fn convert_question(locale: &Locale) -> (&'static str, &'static str) {
    (
        localized(
            locale,
            [
                "Convert all notes to threaded comments?",
                "Voulez-vous convertir toutes les notes en commentaires thématiques\u{a0}?",
                "¿Convertir todas las notas en comentarios encadenados?",
                "Alle Notizen in Kommentarthreads konvertieren?",
                "Convertire tutte le note in commenti in thread?",
            ],
        ),
        localized(
            locale,
            [
                "If you convert from notes to threaded comments, you can then reply to comments or @mention people. However, converting will also remove images or formatting that may be inside the notes.",
                "Si vous convertissez des notes en commentaires, vous pouvez ensuite répondre aux commentaires ou aux contacts @mention. Toutefois, la conversion supprimera également les images ou la mise en forme qui peuvent se trouver dans les notes.",
                "Si convierte desde notas a comentarios encadenados, podrá responder a comentarios o realizar @menciones. Sin embargo, al convertir también se quitarán las imágenes o el formato que pueda haber dentro de las notas.",
                "Wenn Sie aus Notizen in Kommentarthreads konvertieren, können Sie auf Kommentare antworten oder Personen @erwähnen. Durch die Konvertierung werden jedoch möglicherweise in den Notizen enthaltene Bilder oder Formatierungen entfernt.",
                "Se si converte da note a commenti del thread, è possibile rispondere a commenti o @menzionare le persone. Tuttavia, la conversione rimuoverà anche immagini o formattazione che potrebbero trovarsi all'interno delle note.",
            ],
        ),
    )
}

// ---- on the sheet --------------------------------------------------------

impl Sheet {
    /// Where the box of the note of a cell is, in pixels at 100 % from
    /// the corner of A1; `None` when the cell has no note. A note that
    /// was never moved sits 15 pixels right of its cell, a little above
    /// its top.
    pub fn note_frame(&self, row: u32, col: u32) -> Option<Frame> {
        let note = self.comments.get(&(row, col))?.as_note()?;
        let (cols, rows) = (columns(self), rows(self));
        Some(match &note.frame {
            Some(frame) => Frame {
                x: cols.position(frame.anchor.1, frame.offset.0),
                y: rows.position(frame.anchor.0, frame.offset.1),
                width: frame.size.0.max(0.0) as f64,
                height: frame.size.1.max(0.0) as f64,
            },
            None => self.default_note_frame(row, col),
        })
    }

    /// Where Excel first puts the box of the note of a cell: what
    /// `note_frame` answers for a note that was never moved, and where
    /// the box of a note about to be written opens.
    pub fn default_note_frame(&self, row: u32, col: u32) -> Frame {
        let (cols, rows) = (columns(self), rows(self));
        Frame {
            x: cols.span(0, (col + 1).min(MAX_COLS - 1)) + 15.0,
            y: (rows.span(0, row) - 10.0).max(2.0),
            width: NOTE_SIZE.0 as f64,
            height: NOTE_SIZE.1 as f64,
        }
    }

    /// The box a frame on the sheet stands for.
    pub fn note_box(&self, frame: Frame) -> NoteBox {
        let (col, dx) = columns(self).locate(frame.x);
        let (row, dy) = rows(self).locate(frame.y);
        NoteBox {
            anchor: (row, col),
            offset: (dx, dy),
            size: (frame.width.max(0.0) as f32, frame.height.max(0.0) as f32),
        }
    }

    /// The corners of the box of a note as a file writes them (the
    /// `Anchor` of its shape).
    pub fn note_corners(&self, row: u32, col: u32) -> Option<(Corner, Corner)> {
        let frame = self.note_frame(row, col)?;
        let (cols, rows) = (columns(self), rows(self));
        let (c0, dx0) = cols.locate(frame.x);
        let (r0, dy0) = rows.locate(frame.y);
        let (c1, dx1) = cols.locate(frame.x + frame.width);
        let (r1, dy1) = rows.locate(frame.y + frame.height);
        Some((((r0, c0), (dx0, dy0)), ((r1, c1), (dx1, dy1))))
    }

    /// The box between two corners read from a file.
    pub fn note_box_between(&self, from: Corner, to: Corner) -> NoteBox {
        let (cols, rows) = (columns(self), rows(self));
        let width = cols.position(to.0.1, to.1.0) - cols.position(from.0.1, from.1.0);
        let height = rows.position(to.0.0, to.1.1) - rows.position(from.0.0, from.1.1);
        NoteBox {
            anchor: from.0,
            offset: from.1,
            size: (width.max(0.0) as f32, height.max(0.0) as f32),
        }
    }

    /// The cells with a note or a comment, in sheet order (row by row);
    /// `kind` keeps one kind only.
    pub fn comment_cells(&self, kind: Option<CommentKind>) -> Vec<(u32, u32)> {
        let wanted = |comment: &Comment| kind.is_none_or(|kind| comment.kind() == kind);
        self.comments
            .iter()
            .filter(|(_, comment)| wanted(comment))
            .map(|(cell, _)| *cell)
            .collect()
    }
}

/// A box moved along with its cell by (`d_row`, `d_col`).
fn shifted_box(frame: NoteBox, d_row: i64, d_col: i64) -> NoteBox {
    let moved = |position: u32, delta: i64, max: u32| (position as i64 + delta).clamp(0, max as i64 - 1) as u32;
    NoteBox {
        anchor: (
            moved(frame.anchor.0, d_row, MAX_ROWS),
            moved(frame.anchor.1, d_col, MAX_COLS),
        ),
        ..frame
    }
}

impl Workbook {
    /// The note or the comment of a cell.
    pub fn comment_at(&self, sheet: usize, row: u32, col: u32) -> Option<&Comment> {
        self.sheets.get(sheet)?.comments.get(&(row, col))
    }

    fn change_note(&mut self, sheet: usize, row: u32, col: u32, f: impl FnOnce(&mut Note)) -> bool {
        if self.comment_at(sheet, row, col).and_then(Comment::as_note).is_none() {
            return false;
        }
        self.snapshot();
        if let Some(Comment::Note(note)) = self.sheets[sheet].comments.get_mut(&(row, col)) {
            f(note);
        }
        true
    }

    /// Change a thread that is not resolved (or any thread, `even_resolved`).
    fn change_thread(
        &mut self,
        sheet: usize,
        row: u32,
        col: u32,
        even_resolved: bool,
        f: impl FnOnce(&mut Thread),
    ) -> bool {
        let open = |thread: &Thread| even_resolved || !thread.resolved;
        if !self
            .comment_at(sheet, row, col)
            .and_then(Comment::as_thread)
            .is_some_and(open)
        {
            return false;
        }
        self.snapshot();
        if let Some(Comment::Thread(thread)) = self.sheets[sheet].comments.get_mut(&(row, col)) {
            f(thread);
        }
        true
    }

    /// Give a cell a note, or replace the text of the one it has (New
    /// Note, Edit Note). Refused when the cell has a comment thread or
    /// lies off the sheet. Recorded for undo, like every change below.
    pub fn set_note(&mut self, sheet: usize, row: u32, col: u32, author: &str, text: &str) -> bool {
        match self.comment_at(sheet, row, col).map(Comment::kind) {
            Some(CommentKind::Thread) => false,
            Some(CommentKind::Note) => self.change_note(sheet, row, col, |note| note.set_text(text)),
            None if sheet < self.sheets.len() && row < MAX_ROWS && col < MAX_COLS => {
                self.snapshot();
                self.sheets[sheet]
                    .comments
                    .insert((row, col), Comment::Note(Note::new(author, text)));
                true
            }
            None => false,
        }
    }

    /// Show a note all the time, or only under the pointer.
    pub fn set_note_visible(&mut self, sheet: usize, row: u32, col: u32, visible: bool) -> bool {
        self.change_note(sheet, row, col, |note| note.visible = visible)
    }

    /// Whether every note of the sheet is showing (the state of "Show
    /// All Notes"); false without notes.
    pub fn all_notes_visible(&self, sheet: usize) -> bool {
        let Some(sh) = self.sheets.get(sheet) else {
            return false;
        };
        let mut notes = sh.comments.values().filter_map(Comment::as_note).peekable();
        notes.peek().is_some() && notes.all(|note| note.visible)
    }

    /// Show or hide every note of a sheet ("Show All Notes"). Returns
    /// whether any changed.
    pub fn set_all_notes_visible(&mut self, sheet: usize, visible: bool) -> bool {
        let differs = |comment: &Comment| comment.as_note().is_some_and(|note| note.visible != visible);
        if !self
            .sheets
            .get(sheet)
            .is_some_and(|sh| sh.comments.values().any(differs))
        {
            return false;
        }
        self.snapshot();
        for comment in self.sheets[sheet].comments.values_mut() {
            if let Comment::Note(note) = comment {
                note.visible = visible;
            }
        }
        true
    }

    /// Move or resize the box of a note; `None` puts it back next to its
    /// cell.
    pub fn set_note_frame(&mut self, sheet: usize, row: u32, col: u32, frame: Option<Frame>) -> bool {
        let Some(sh) = self.sheets.get(sheet) else {
            return false;
        };
        let frame = frame.map(|frame| sh.note_box(frame));
        self.change_note(sheet, row, col, |note| note.frame = frame)
    }

    /// Delete the note or the whole thread of a cell.
    pub fn delete_comment(&mut self, sheet: usize, row: u32, col: u32) -> bool {
        if self.comment_at(sheet, row, col).is_none() {
            return false;
        }
        self.snapshot();
        self.sheets[sheet].comments.remove(&(row, col));
        true
    }

    /// Delete the notes and comments of the selected cells; returns how
    /// many there were.
    pub fn delete_comments(&mut self, sheet: usize, ranges: &[Rect]) -> usize {
        let Some(sh) = self.sheets.get(sheet) else {
            return 0;
        };
        // Range by range: there may be thousands (the rows a filter shows).
        let doomed: BTreeSet<(u32, u32)> = ranges
            .iter()
            .flat_map(|r| {
                let rows = sh.comments.range((r.r0, 0)..=(r.r1, u32::MAX));
                rows.map(|(cell, _)| *cell).filter(|(_, col)| r.c0 <= *col && *col <= r.c1)
            })
            .collect();
        if !doomed.is_empty() {
            self.snapshot();
            for cell in &doomed {
                self.sheets[sheet].comments.remove(cell);
            }
        }
        doomed.len()
    }

    /// Start a comment thread on a cell (New Comment). `time` is
    /// `now_utc()`. Refused when the cell already has a note or a thread.
    pub fn add_comment(&mut self, sheet: usize, row: u32, col: u32, author: &Person, time: f64, text: &str) -> bool {
        self.add_comment_post(sheet, row, col, Post::new(author, time, text))
    }

    /// `add_comment` for a post that may name people.
    pub fn add_comment_post(&mut self, sheet: usize, row: u32, col: u32, post: Post) -> bool {
        if sheet >= self.sheets.len()
            || row >= MAX_ROWS
            || col >= MAX_COLS
            || self.comment_at(sheet, row, col).is_some()
        {
            return false;
        }
        self.snapshot();
        let thread = Thread {
            first: post,
            replies: Vec::new(),
            resolved: false,
        };
        self.sheets[sheet].comments.insert((row, col), Comment::Thread(thread));
        true
    }

    /// Reply to the thread of a cell. Refused while it is resolved.
    pub fn add_reply(&mut self, sheet: usize, row: u32, col: u32, author: &Person, time: f64, text: &str) -> bool {
        self.add_reply_post(sheet, row, col, Post::new(author, time, text))
    }

    /// `add_reply` for a post that may name people.
    pub fn add_reply_post(&mut self, sheet: usize, row: u32, col: u32, reply: Post) -> bool {
        self.change_thread(sheet, row, col, false, |thread| thread.replies.push(reply))
    }

    /// Change the text of the first comment of a thread (`reply` `None`)
    /// or of one of its replies; it then names nobody. Refused while the
    /// thread is resolved.
    pub fn edit_comment(&mut self, sheet: usize, row: u32, col: u32, reply: Option<usize>, text: &str) -> bool {
        self.edit_comment_mentions(sheet, row, col, reply, text, Vec::new())
    }

    /// `edit_comment` for a text that names people.
    pub fn edit_comment_mentions(
        &mut self,
        sheet: usize,
        row: u32,
        col: u32,
        reply: Option<usize>,
        text: &str,
        mentions: Vec<Mention>,
    ) -> bool {
        let exists = |thread: &Thread| reply.is_none_or(|index| index < thread.replies.len());
        if !self
            .comment_at(sheet, row, col)
            .and_then(Comment::as_thread)
            .is_some_and(exists)
        {
            return false;
        }
        self.change_thread(sheet, row, col, false, |thread| {
            let post = match reply {
                Some(index) => &mut thread.replies[index],
                None => &mut thread.first,
            };
            *post = Post::new(&post.author, post.time, text).with_mentions(mentions);
        })
    }

    /// Everyone the comments of the workbook know: those who wrote one
    /// and those one names, each once, in the order they first appear.
    pub fn people(&self) -> Vec<Person> {
        let mut out: Vec<Person> = Vec::new();
        let threads = self
            .sheets
            .iter()
            .flat_map(|sheet| sheet.comments.values())
            .filter_map(Comment::as_thread);
        for post in threads.flat_map(Thread::posts) {
            let named = post.mentions.iter().map(|mention| &mention.person);
            for person in std::iter::once(&post.author).chain(named) {
                if !out.contains(person) {
                    out.push(person.clone());
                }
            }
        }
        out
    }

    /// Delete one reply of a thread (the thread itself: `delete_comment`).
    pub fn delete_reply(&mut self, sheet: usize, row: u32, col: u32, reply: usize) -> bool {
        let exists = |thread: &Thread| reply < thread.replies.len();
        if !self
            .comment_at(sheet, row, col)
            .and_then(Comment::as_thread)
            .is_some_and(exists)
        {
            return false;
        }
        self.change_thread(sheet, row, col, false, |thread| {
            thread.replies.remove(reply);
        })
    }

    /// Resolve a thread, or reopen it.
    pub fn set_comment_resolved(&mut self, sheet: usize, row: u32, col: u32, resolved: bool) -> bool {
        let differs = |thread: &Thread| thread.resolved != resolved;
        if !self
            .comment_at(sheet, row, col)
            .and_then(Comment::as_thread)
            .is_some_and(differs)
        {
            return false;
        }
        self.change_thread(sheet, row, col, true, |thread| thread.resolved = resolved)
    }

    /// Turn every note of a sheet into a comment thread ("Convert to
    /// Comments"): the text loses its formatting and the line naming the
    /// author, who becomes the author of the comment, dated `time`.
    /// Returns how many notes were converted.
    pub fn convert_notes_to_comments(&mut self, sheet: usize, time: f64) -> usize {
        let Some(sh) = self.sheets.get(sheet) else {
            return 0;
        };
        let converted: Vec<((u32, u32), Thread)> = sh
            .comments
            .iter()
            .filter_map(|(cell, comment)| Some((*cell, comment.as_note()?.as_thread(time))))
            .collect();
        if !converted.is_empty() {
            self.snapshot();
        }
        let count = converted.len();
        for (cell, thread) in converted {
            self.sheets[sheet].comments.insert(cell, Comment::Thread(thread));
        }
        count
    }

    /// Turn the note of one cell into a comment thread, as "Convert to
    /// Comments" does for all of them: what Excel offers when a comment
    /// is asked for on a cell that has a note.
    pub fn convert_note_to_comment(&mut self, sheet: usize, row: u32, col: u32, time: f64) -> bool {
        let Some(thread) = self
            .comment_at(sheet, row, col)
            .and_then(Comment::as_note)
            .map(|note| note.as_thread(time))
        else {
            return false;
        };
        self.snapshot();
        self.sheets[sheet].comments.insert((row, col), Comment::Thread(thread));
        true
    }

    // ---- going through them ---------------------------------------------

    /// The first note or comment after a cell (sheet, row, col), in sheet
    /// order and then on the following sheets; `kind` looks for one kind
    /// only. `None` at the end of the workbook: Excel then offers to start
    /// again (`end_of_workbook_question`, `first_comment`).
    pub fn next_comment(&self, from: (usize, u32, u32), kind: Option<CommentKind>) -> Option<(usize, u32, u32)> {
        let wanted = |comment: &Comment| kind.is_none_or(|kind| comment.kind() == kind);
        let (sheet, row, col) = from;
        let rest = self
            .sheets
            .get(sheet)?
            .comments
            .range((Excluded((row, col)), Unbounded));
        if let Some((cell, _)) = rest.into_iter().find(|(_, comment)| wanted(comment)) {
            return Some((sheet, cell.0, cell.1));
        }
        self.sheets.iter().enumerate().skip(sheet + 1).find_map(|(index, sh)| {
            let (cell, _) = sh.comments.iter().find(|(_, comment)| wanted(comment))?;
            Some((index, cell.0, cell.1))
        })
    }

    /// The last note or comment before a cell, back to the beginning of
    /// the workbook.
    pub fn previous_comment(&self, from: (usize, u32, u32), kind: Option<CommentKind>) -> Option<(usize, u32, u32)> {
        let wanted = |comment: &Comment| kind.is_none_or(|kind| comment.kind() == kind);
        let (sheet, row, col) = from;
        let before = self.sheets.get(sheet)?.comments.range(..(row, col));
        if let Some((cell, _)) = before.rev().find(|(_, comment)| wanted(comment)) {
            return Some((sheet, cell.0, cell.1));
        }
        self.sheets
            .iter()
            .enumerate()
            .take(sheet)
            .rev()
            .find_map(|(index, sh)| {
                let (cell, _) = sh.comments.iter().rev().find(|(_, comment)| wanted(comment))?;
                Some((index, cell.0, cell.1))
            })
    }

    /// The first note or comment of the workbook.
    pub fn first_comment(&self, kind: Option<CommentKind>) -> Option<(usize, u32, u32)> {
        let wanted = |comment: &Comment| kind.is_none_or(|kind| comment.kind() == kind);
        self.sheets.iter().enumerate().find_map(|(index, sh)| {
            let (cell, _) = sh.comments.iter().find(|(_, comment)| wanted(comment))?;
            Some((index, cell.0, cell.1))
        })
    }

    /// The last note or comment of the workbook.
    pub fn last_comment(&self, kind: Option<CommentKind>) -> Option<(usize, u32, u32)> {
        let wanted = |comment: &Comment| kind.is_none_or(|kind| comment.kind() == kind);
        self.sheets.iter().enumerate().rev().find_map(|(index, sh)| {
            let (cell, _) = sh.comments.iter().rev().find(|(_, comment)| wanted(comment))?;
            Some((index, cell.0, cell.1))
        })
    }

    // ---- with their cells ------------------------------------------------

    /// Take the notes and comments of the block `src` to each of `dests`,
    /// replacing those found there. A destination is a block of the same
    /// size or, for a copy, one holding several copies of `src` (one cell
    /// pasted over a selection, a row filled down), which each get them.
    /// With `take` they leave the source (cut and paste, dragging cells),
    /// else they are copied (copy and paste, filling).
    fn carry_comments(&mut self, src_sheet: usize, src: Rect, dest_sheet: usize, dests: &[Rect], take: bool) {
        if src_sheet >= self.sheets.len() || dest_sheet >= self.sheets.len() {
            return;
        }
        if src_sheet == dest_sheet && dests == [src] {
            return;
        }
        let within = |sheet: &Sheet, rect: &Rect| -> Vec<(u32, u32)> {
            let cells = sheet.comments.range((rect.r0, 0)..=(rect.r1, u32::MAX));
            cells
                .map(|(cell, _)| *cell)
                .filter(|(_, c)| *c >= rect.c0 && *c <= rect.c1)
                .collect()
        };
        let carried = within(&self.sheets[src_sheet], &src);
        let replaced: Vec<(u32, u32)> = dests
            .iter()
            .flat_map(|dest| within(&self.sheets[dest_sheet], dest))
            .collect();
        if carried.is_empty() && replaced.is_empty() {
            return;
        }
        self.snapshot();
        let mut comments = Vec::new();
        for cell in carried {
            let source = &mut self.sheets[src_sheet].comments;
            let comment = if take {
                source.remove(&cell)
            } else {
                source.get(&cell).cloned()
            };
            comments.extend(comment.map(|comment| (cell, comment)));
        }
        for cell in replaced {
            self.sheets[dest_sheet].comments.remove(&cell);
        }
        for ((row, col), comment) in comments {
            let copies = dests.iter().flat_map(|dest| tiled(Rect::cell(row, col), &src, dest));
            let cells = copies.flat_map(|copy| (copy.r0..=copy.r1).flat_map(move |r| (copy.c0..=copy.c1).map(move |c| (r, c))));
            for target in cells {
                let (d_row, d_col) = (target.0 as i64 - row as i64, target.1 as i64 - col as i64);
                let mut comment = comment.clone();
                // A box that was placed by hand keeps its place relative
                // to the cell.
                if let Comment::Note(note) = &mut comment {
                    note.frame = note.frame.map(|frame| shifted_box(frame, d_row, d_col));
                }
                self.sheets[dest_sheet].comments.insert(target, comment);
            }
        }
    }

    /// The block of the size of `src` at (`dest_row`, `dest_col`), cut at
    /// the edges of the sheet; `None` when it starts off the sheet.
    fn block_at(src: Rect, dest_row: u32, dest_col: u32) -> Option<Rect> {
        (dest_row < MAX_ROWS && dest_col < MAX_COLS).then(|| {
            Rect::new(
                dest_row,
                dest_col,
                (dest_row as u64 + src.rows() as u64 - 1).min(MAX_ROWS as u64 - 1) as u32,
                (dest_col as u64 + src.cols() as u64 - 1).min(MAX_COLS as u64 - 1) as u32,
            )
        })
    }

    /// Notes and comments follow cells that are moved (cut and paste,
    /// dragging a selection): `move_range` calls this, with its own
    /// arguments. Those of the destination are replaced.
    pub fn move_comments(&mut self, src_sheet: usize, src: Rect, dest_sheet: usize, dest_row: u32, dest_col: u32) {
        if let Some(dest) = Self::block_at(src, dest_row, dest_col) {
            self.carry_comments(src_sheet, src, dest_sheet, &[dest], true);
        }
    }

    /// Notes and comments are copied with their cells (`paste` does it
    /// for Paste and for Paste Special ▸ Comments and Notes): the block at
    /// (`dest_row`, `dest_col`) gets those of `src`, and loses its own.
    pub fn copy_comments(&mut self, src_sheet: usize, src: Rect, dest_sheet: usize, dest_row: u32, dest_col: u32) {
        if let Some(dest) = Self::block_at(src, dest_row, dest_col) {
            self.carry_comments(src_sheet, src, dest_sheet, &[dest], false);
        }
    }

    /// As `copy_comments`, laying `src` over each of `dests`: a
    /// destination holds a whole number of copies of `src` (one cell
    /// pasted over a selection, a row filled down) or one copy cut short,
    /// and every copy gets the notes and comments of `src`. Several
    /// destinations are the blocks of rows a filter leaves shown.
    pub(crate) fn tile_comments(&mut self, src_sheet: usize, src: Rect, dest_sheet: usize, dests: &[Rect]) {
        self.carry_comments(src_sheet, src, dest_sheet, dests, false);
    }

    /// Notes and comments follow the lines of `rect` that a sort or
    /// Remove Duplicates moves: `moves` lists (from, to) rows, or columns
    /// when `by_rows` is false. Those found on the lines written over
    /// are replaced.
    pub(crate) fn reorder_comments(&mut self, sheet: usize, rect: Rect, by_rows: bool, moves: &[(u32, u32)]) {
        let Some(sh) = self.sheets.get(sheet).filter(|sh| !sh.comments.is_empty()) else {
            return;
        };
        let line = |cell: &(u32, u32)| if by_rows { cell.0 } else { cell.1 };
        let landing: HashMap<u32, u32> = moves.iter().copied().collect();
        let written: HashSet<u32> = moves.iter().map(|(_, to)| *to).collect();
        let touched: Vec<(u32, u32)> = sh
            .comments
            .range((rect.r0, 0)..=(rect.r1, u32::MAX))
            .map(|(cell, _)| *cell)
            .filter(|cell| rect.contains(cell.0, cell.1))
            .filter(|cell| landing.contains_key(&line(cell)) || written.contains(&line(cell)))
            .collect();
        if touched.is_empty() {
            return;
        }
        self.snapshot();
        let comments = &mut self.sheets[sheet].comments;
        let mut lifted = Vec::new();
        for cell in touched {
            let comment = comments.remove(&cell);
            let to = landing.get(&line(&cell)).copied();
            lifted.extend(comment.zip(to).map(|(comment, to)| (cell, to, comment)));
        }
        for ((row, col), to, mut comment) in lifted {
            let target = if by_rows { (to, col) } else { (row, to) };
            if let Comment::Note(note) = &mut comment {
                let (d_row, d_col) = (target.0 as i64 - row as i64, target.1 as i64 - col as i64);
                note.frame = note.frame.map(|frame| shifted_box(frame, d_row, d_col));
            }
            comments.insert(target, comment);
        }
    }
}

/// See `features::on_structural_edit`: notes and comments follow their
/// cells and disappear with them.
pub(crate) fn on_structural_edit(sheet: &mut Sheet, edit: StructuralEdit, edited_sheet: &str) {
    if sheet.comments.is_empty() || !sheet.name.eq_ignore_ascii_case(edited_sheet) {
        return;
    }
    let (row_axis, insert, at, count) = match edit {
        StructuralEdit::InsertRows { at, count } => (true, true, at, count),
        StructuralEdit::DeleteRows { at, count } => (true, false, at, count),
        StructuralEdit::InsertCols { at, count } => (false, true, at, count),
        StructuralEdit::DeleteCols { at, count } => (false, false, at, count),
    };
    let max = if row_axis { MAX_ROWS } else { MAX_COLS };
    for ((row, col), mut comment) in std::mem::take(&mut sheet.comments) {
        let position = if row_axis { row } else { col };
        let moved = if insert {
            if position >= at { position + count } else { position }
        } else if position < at {
            position
        } else if position >= at + count {
            position - count
        } else {
            continue;
        };
        if moved >= max {
            continue;
        }
        if let Comment::Note(Note { frame: Some(frame), .. }) = &mut comment {
            if row_axis {
                (frame.anchor.0, frame.offset.1) =
                    moved_corner(frame.anchor.0, frame.offset.1, false, insert, at, count, max);
            } else {
                (frame.anchor.1, frame.offset.0) =
                    moved_corner(frame.anchor.1, frame.offset.0, false, insert, at, count, max);
            }
        }
        sheet
            .comments
            .insert(if row_axis { (moved, col) } else { (row, moved) }, comment);
    }
}

/// See `features::on_sheet_renamed`: notes and comments name no sheet.
pub(crate) fn on_sheet_renamed(_sheet: &mut Sheet, _old: &str, _new: &str) {}

/// See `features::on_sheet_duplicated`: the copy keeps the notes and
/// comments of the original, which cloning the sheet already gave it.
pub(crate) fn on_sheet_duplicated(_wb: &mut Workbook, _copy: usize, _original: &str) {}

/// See `features::on_sheet_deleted`: the notes and comments of a sheet go
/// with it.
pub(crate) fn on_sheet_deleted(_wb: &mut Workbook, _deleted: &Sheet, _order: &[&str]) {}

/// See `features::on_sheet_moved`: notes and comments are kept on their
/// sheet, whose place only matters to `next_comment`, which reads it.
pub(crate) fn on_sheet_moved(_wb: &mut Workbook, _moved: &str, _before: &[&str], _after: &[&str]) {}

/// See `features::on_cells_shifted`: notes and comments follow the cells
/// that moved and disappear with those that went.
pub(crate) fn on_cells_shifted(wb: &mut Workbook, sheet: usize, shift: CellShift) {
    wb.delete_comments(sheet, &[shift.removed()]);
    if let Some((from, to)) = shift.carried() {
        wb.move_comments(sheet, from, sheet, to.r0, to.c0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::wb;

    fn rect(name: &str) -> Rect {
        Rect::parse(name).unwrap()
    }

    fn cell(name: &str) -> (u32, u32) {
        crate::refs::parse_cell_name(name).unwrap()
    }

    fn note_text(w: &Workbook, at: &str) -> Option<String> {
        let (r, c) = cell(at);
        w.comment_at(0, r, c)?.as_note().map(Note::text)
    }

    fn thread<'a>(w: &'a Workbook, at: &str) -> &'a Thread {
        let (r, c) = cell(at);
        w.comment_at(0, r, c).and_then(Comment::as_thread).unwrap()
    }

    const NOON: f64 = 45_000.5;

    #[test]
    fn notes() {
        let mut w = wb(&[("B2", "5")]);
        assert_eq!(Note::template("Ada"), "Ada:\n");
        assert!(w.set_note(0, 1, 1, "Ada", "Ada:\nCheck this figure."));
        w.commit("note");
        let note = w.comment_at(0, 1, 1).unwrap().as_note().unwrap();
        assert_eq!(note.author, "Ada");
        assert_eq!(note.text(), "Ada:\nCheck this figure.");
        // The author's name is in bold, the rest is not; all of it in Tahoma 9.
        assert_eq!(note.runs.len(), 2);
        assert_eq!((note.runs[0].text.as_str(), note.runs[0].font.bold), ("Ada:", true));
        assert_eq!(
            (note.runs[1].text.as_str(), note.runs[1].font.bold),
            ("\nCheck this figure.", false)
        );
        assert_eq!(
            (note.runs[1].font.name.as_ref(), note.runs[1].font.size_pt()),
            ("Tahoma", 9.0)
        );
        assert!(!note.visible && note.frame.is_none());
        // Editing replaces the text and keeps the author.
        assert!(w.set_note(0, 1, 1, "Someone else", "Plain words"));
        w.commit("edit");
        let note = w.comment_at(0, 1, 1).unwrap().as_note().unwrap();
        assert_eq!(
            (note.author.as_str(), note.runs.len(), note.text().as_str()),
            ("Ada", 1, "Plain words")
        );
        w.undo();
        assert_eq!(note_text(&w, "B2").as_deref(), Some("Ada:\nCheck this figure."));
        w.undo();
        assert!(w.comment_at(0, 1, 1).is_none());
        w.redo();
        assert!(w.delete_comment(0, 1, 1));
        assert!(!w.delete_comment(0, 1, 1));
        assert!(!w.set_note(7, 0, 0, "Ada", "x"));
        assert!(!w.set_note(0, MAX_ROWS, 0, "Ada", "x"));
        // An empty note is a note all the same.
        assert!(w.set_note(0, 0, 0, "", ""));
        assert_eq!(note_text(&w, "A1").as_deref(), Some(""));
    }

    #[test]
    fn showing_and_hiding() {
        let mut w = wb(&[]);
        w.set_note(0, 0, 0, "Ada", "one");
        w.set_note(0, 4, 2, "Ada", "two");
        w.add_comment(0, 9, 9, &Person::new("Ada"), NOON, "a thread");
        w.commit("notes");
        assert!(!w.all_notes_visible(0));
        assert!(w.set_note_visible(0, 0, 0, true));
        assert!(!w.set_note_visible(0, 1, 1, true));
        assert!(!w.set_note_visible(0, 9, 9, true));
        assert!(!w.all_notes_visible(0));
        assert!(w.set_all_notes_visible(0, true));
        assert!(w.all_notes_visible(0));
        assert!(!w.set_all_notes_visible(0, true));
        w.commit("show");
        assert!(w.set_all_notes_visible(0, false));
        assert!(!w.comment_at(0, 4, 2).unwrap().as_note().unwrap().visible);
        w.commit("hide");
        w.undo();
        assert!(w.all_notes_visible(0));
        let other = w.add_sheet(1);
        assert!(!w.all_notes_visible(other) && !w.set_all_notes_visible(other, true));
    }

    #[test]
    fn boxes() {
        let mut w = wb(&[]);
        w.set_note(0, 0, 0, "Ada", "top");
        w.set_note(0, 4, 2, "Ada", "below");
        w.commit("notes");
        // Where Excel first puts them: 15 pixels right of the cell, a little above its top.
        assert_eq!(w.sheets[0].note_frame(0, 0), Some(Frame::new(79.0, 2.0, 144.0, 79.0)));
        assert_eq!(w.sheets[0].note_frame(4, 2), Some(Frame::new(207.0, 70.0, 144.0, 79.0)));
        assert_eq!(
            w.sheets[0].note_corners(0, 0),
            Some((((0, 1), (15.0, 2.0)), ((4, 3), (31.0, 1.0))))
        );
        assert_eq!(w.sheets[0].note_frame(1, 1), None);
        // The box of a note about to be written opens at the same place.
        assert_eq!(w.sheets[0].default_note_frame(4, 2), Frame::new(207.0, 70.0, 144.0, 79.0));
        // A box left alone follows its cell and the widths of the columns.
        w.set_col_width(0, 2, Some(100.0));
        w.insert_rows(0, 0, 3);
        w.commit("edit");
        assert_eq!(
            w.sheets[0].note_frame(7, 2),
            Some(Frame::new(243.0, 130.0, 144.0, 79.0))
        );
        // A box placed by hand stays where it was put, and moves with the rows above it.
        assert!(w.set_note_frame(0, 7, 2, Some(Frame::new(300.0, 210.0, 200.0, 50.0))));
        w.commit("move");
        let placed = w.comment_at(0, 7, 2).unwrap().as_note().unwrap().frame.unwrap();
        assert_eq!(
            placed,
            NoteBox {
                anchor: (10, 4),
                offset: (8.0, 10.0),
                size: (200.0, 50.0)
            }
        );
        w.insert_rows(0, 5, 2);
        w.insert_cols(0, 0, 1);
        w.commit("insert");
        assert_eq!(
            w.sheets[0].note_frame(9, 3),
            Some(Frame::new(364.0, 250.0, 200.0, 50.0))
        );
        assert!(w.set_note_frame(0, 9, 3, None));
        assert_eq!(w.sheets[0].note_frame(9, 3).unwrap().width, 144.0);
        // What a file holds, and back.
        let sheet = &w.sheets[0];
        let from_file = sheet.note_box_between(((1, 1), (15.0, 2.0)), ((5, 3), (31.0, 1.0)));
        assert_eq!(
            from_file,
            NoteBox {
                anchor: (1, 1),
                offset: (15.0, 2.0),
                size: (144.0, 79.0)
            }
        );
        assert!(!w.set_note_frame(0, 50, 50, None));
    }

    #[test]
    fn threads() {
        let mut w = wb(&[]);
        let (ada, bob) = (Person::new("Ada"), Person::new("Bob"));
        assert_eq!((ada.user_id.as_str(), ada.provider_id.as_str()), ("Ada", "None"));
        assert!(w.add_comment(0, 2, 1, &ada, NOON, "Is this right?"));
        assert!(!w.add_comment(0, 2, 1, &bob, NOON, "second thread"));
        assert!(!w.set_note(0, 2, 1, "Bob", "a note on top"));
        assert!(w.add_reply(0, 2, 1, &bob, NOON + 0.25, "Yes."));
        assert!(w.add_reply(0, 2, 1, &ada, NOON + 0.5, "Thnaks"));
        assert!(!w.add_reply(0, 0, 0, &ada, NOON, "nowhere"));
        w.commit("thread");
        assert!(w.edit_comment(0, 2, 1, Some(1), "Thanks"));
        assert!(w.edit_comment(0, 2, 1, None, "Is this right? (B3)"));
        assert!(!w.edit_comment(0, 2, 1, Some(2), "no such reply"));
        w.commit("edit");
        let t = thread(&w, "B3");
        assert_eq!(
            t.first,
            Post::new(&ada, NOON, "Is this right? (B3)")
        );
        assert_eq!(
            t.posts().map(|p| p.author.name.as_str()).collect::<Vec<_>>(),
            ["Ada", "Bob", "Ada"]
        );
        assert_eq!(t.replies[1].text, "Thanks");
        assert_eq!(t.replies[0].time, NOON + 0.25);
        // A resolved thread takes neither replies nor edits until it is reopened.
        assert!(w.set_comment_resolved(0, 2, 1, true));
        assert!(!w.set_comment_resolved(0, 2, 1, true));
        assert!(thread(&w, "B3").resolved);
        assert!(!w.add_reply(0, 2, 1, &bob, NOON + 1.0, "too late"));
        assert!(!w.edit_comment(0, 2, 1, None, "changed"));
        assert!(!w.delete_reply(0, 2, 1, 0));
        assert!(w.set_comment_resolved(0, 2, 1, false));
        assert!(w.delete_reply(0, 2, 1, 0));
        assert!(!w.delete_reply(0, 2, 1, 5));
        w.commit("resolve");
        assert_eq!(thread(&w, "B3").replies.len(), 1);
        w.undo();
        assert_eq!(thread(&w, "B3").replies.len(), 2);
        assert!(!thread(&w, "B3").resolved);
        // Deleting the comment deletes the thread.
        assert!(w.delete_comment(0, 2, 1));
        assert!(w.comment_at(0, 2, 1).is_none());
    }

    #[test]
    fn one_note_becomes_a_comment() {
        let mut w = wb(&[]);
        assert!(w.set_note(0, 8, 0, "Ada", "Ada:\nFirst"));
        assert!(w.set_note(0, 9, 0, "Bob", "Bob:\nSecond"));
        w.commit("notes");
        assert!(w.convert_note_to_comment(0, 9, 0, NOON));
        w.commit("convert one");
        let first = &thread(&w, "A10").first;
        assert_eq!(
            (first.author.name.as_str(), first.text.as_str(), first.time),
            ("Bob", "Second", NOON)
        );
        // The others stay notes; a thread or an empty cell is not converted.
        assert_eq!(w.comment_at(0, 8, 0).map(Comment::kind), Some(CommentKind::Note));
        assert!(!w.convert_note_to_comment(0, 9, 0, NOON));
        assert!(!w.convert_note_to_comment(0, 20, 0, NOON));
        w.undo();
        assert_eq!(note_text(&w, "A10").as_deref(), Some("Bob:\nSecond"));
    }

    #[test]
    fn notes_become_comments() {
        let mut w = wb(&[]);
        w.set_note(0, 0, 0, "Ada", "Ada:\nFirst");
        w.set_note(0, 1, 0, "Bob", "No header");
        w.add_comment(0, 2, 0, &Person::new("Cy"), NOON, "already one");
        w.commit("setup");
        assert_eq!(w.convert_notes_to_comments(0, NOON + 1.0), 2);
        w.commit("convert");
        let first = thread(&w, "A1");
        assert_eq!(
            first.first,
            Post::new(&Person::new("Ada"), NOON + 1.0, "First")
        );
        assert!(first.replies.is_empty() && !first.resolved);
        assert_eq!(thread(&w, "A2").first.text, "No header");
        assert_eq!(thread(&w, "A3").first.time, NOON);
        assert_eq!(w.convert_notes_to_comments(0, NOON), 0);
        assert!(w.sheets[0].comment_cells(Some(CommentKind::Note)).is_empty());
        w.undo();
        assert_eq!(w.sheets[0].comment_cells(Some(CommentKind::Note)), [(0, 0), (1, 0)]);
        assert!(
            convert_question(Locale::fr())
                .0
                .starts_with("Voulez-vous convertir toutes les notes")
        );
    }

    #[test]
    fn next_and_previous() {
        let mut w = wb(&[]);
        let ada = Person::new("Ada");
        w.set_note(0, 5, 0, "Ada", "n1");
        w.add_comment(0, 0, 3, &ada, NOON, "t1");
        w.set_note(0, 0, 1, "Ada", "n0");
        let second = w.add_sheet(1);
        let third = w.add_sheet(2);
        w.add_comment(third, 2, 2, &ada, NOON, "t2");
        w.set_note(third, 2, 3, "Ada", "n2");
        w.commit("setup");
        // Sheet order is row by row, then the following sheets.
        assert_eq!(w.sheets[0].comment_cells(None), [(0, 1), (0, 3), (5, 0)]);
        assert_eq!(w.sheets[0].comment_cells(Some(CommentKind::Thread)), [(0, 3)]);
        assert_eq!(w.first_comment(None), Some((0, 0, 1)));
        assert_eq!(w.next_comment((0, 0, 0), None), Some((0, 0, 1)));
        assert_eq!(w.next_comment((0, 0, 1), None), Some((0, 0, 3)));
        assert_eq!(w.next_comment((0, 0, 3), None), Some((0, 5, 0)));
        assert_eq!(w.next_comment((0, 5, 0), None), Some((third, 2, 2)));
        assert_eq!(w.next_comment((second, 0, 0), None), Some((third, 2, 2)));
        assert_eq!(w.next_comment((third, 2, 3), None), None);
        assert_eq!(w.last_comment(None), Some((third, 2, 3)));
        assert_eq!(w.previous_comment((third, 2, 3), None), Some((third, 2, 2)));
        assert_eq!(w.previous_comment((third, 2, 2), None), Some((0, 5, 0)));
        assert_eq!(w.previous_comment((0, 0, 1), None), None);
        // Notes and comments have their own Next and Previous.
        let (notes, threads) = (Some(CommentKind::Note), Some(CommentKind::Thread));
        assert_eq!(w.next_comment((0, 0, 1), notes), Some((0, 5, 0)));
        assert_eq!(w.next_comment((0, 5, 0), notes), Some((third, 2, 3)));
        assert_eq!(w.next_comment((0, 0, 3), threads), Some((third, 2, 2)));
        assert_eq!(w.previous_comment((third, 2, 3), notes), Some((0, 5, 0)));
        assert_eq!(w.first_comment(threads), Some((0, 0, 3)));
        assert_eq!(w.last_comment(threads), Some((third, 2, 2)));
        assert_eq!(wb(&[]).first_comment(None), None);
        assert!(end_of_workbook_question(Locale::en()).starts_with("Microsoft Excel reached the end of the workbook."));
    }

    #[test]
    fn structural_edits() {
        let mut w = wb(&[]);
        w.set_note(0, 1, 1, "Ada", "B2");
        w.set_note(0, 4, 3, "Ada", "D5");
        w.add_comment(0, 7, 0, &Person::new("Ada"), NOON, "A8");
        w.commit("setup");
        w.insert_rows(0, 2, 2);
        w.insert_cols(0, 0, 1);
        w.commit("insert");
        assert_eq!(w.sheets[0].comment_cells(None), [cell("C2"), cell("E7"), cell("B10")]);
        assert_eq!(note_text(&w, "E7").as_deref(), Some("D5"));
        // Comments of deleted cells go with them.
        w.delete_rows(0, 5, 3);
        w.delete_cols(0, 2, 1);
        w.commit("delete");
        assert_eq!(w.sheets[0].comment_cells(None), [cell("B7")]);
        assert_eq!(thread(&w, "B7").first.text, "A8");
        w.undo();
        w.undo();
        assert_eq!(w.sheets[0].comment_cells(None), [cell("B2"), cell("D5"), cell("A8")]);
        // Edits on another sheet and copies of the sheet.
        let other = w.add_sheet(1);
        w.insert_rows(other, 0, 4);
        let copy = w.duplicate_sheet(0).unwrap();
        w.commit("sheets");
        assert_eq!(w.sheets[0].comment_cells(None).len(), 3);
        assert_eq!(w.sheets[copy].comments, w.sheets[0].comments);
    }

    #[test]
    fn comments_travel_with_cut_and_copied_cells() {
        let mut w = wb(&[("A1", "1"), ("A2", "2"), ("C1", "x")]);
        w.set_note(0, 0, 0, "Ada", "on A1");
        w.add_comment(0, 1, 0, &Person::new("Ada"), NOON, "on A2");
        w.set_note(0, 5, 2, "Ada", "on C6, in the way");
        w.set_note(0, 9, 9, "Ada", "elsewhere");
        w.set_note_frame(0, 0, 0, Some(Frame::new(100.0, 0.0, 50.0, 50.0)));
        w.commit("setup");
        // Cut A1:A3 and paste on C5: the cells and what is attached to them move.
        w.move_range(0, rect("A1:A3"), 0, 4, 2);
        w.commit("move");
        assert_eq!(w.sheets[0].comment_cells(None), [cell("C5"), cell("C6"), cell("J10")]);
        assert_eq!(note_text(&w, "C5").as_deref(), Some("on A1"));
        assert_eq!(thread(&w, "C6").first.text, "on A2");
        // The box placed by hand moved by as many cells as its note.
        let moved = w.comment_at(0, 4, 2).unwrap().as_note().unwrap().frame.unwrap();
        assert_eq!((moved.anchor, moved.offset), ((4, 3), (36.0, 0.0)));
        w.undo();
        assert_eq!(
            w.sheets[0].comment_cells(None),
            [cell("A1"), cell("A2"), cell("C6"), cell("J10")]
        );
        // Copy A1:A2 to another sheet: both sheets have them.
        let other = w.add_sheet(1);
        w.copy_comments(0, rect("A1:A2"), other, 0, 4);
        w.commit("copy");
        assert_eq!(w.sheets[other].comment_cells(None), [(0, 4), (1, 4)]);
        assert_eq!(w.sheets[0].comment_cells(None).len(), 4);
        // Pasting cells without comments over cells with comments removes them.
        w.copy_comments(0, rect("E1:E2"), other, 0, 4);
        assert!(w.sheets[other].comments.is_empty());
        // Nothing to do, nothing recorded.
        w.commit("paste");
        w.copy_comments(0, rect("E1:E2"), other, 0, 4);
        w.move_comments(0, rect("A1:A2"), 0, 0, 0);
        assert!(!w.commit("nothing"));
        assert_eq!(w.delete_comments(0, &[rect("A1:C6")]), 3);
        assert_eq!(w.delete_comments(0, &[rect("A1:C6")]), 0);
        assert_eq!(w.sheets[0].comment_cells(None), [cell("J10")]);
    }

    #[test]
    fn times() {
        // 1 March 2024, 09:30:15.25.
        let serial = date_to_serial(2024, 3, 1).unwrap() + time_to_serial(9.0, 30.0, 15.25);
        assert_eq!(time_to_iso(serial), "2024-03-01T09:30:15.25");
        assert!((time_from_iso("2024-03-01T09:30:15.25").unwrap() - serial).abs() < 1e-9);
        assert_eq!(
            time_from_iso("2024-03-01T09:30:15.25Z"),
            time_from_iso("2024-03-01T09:30:15.25")
        );
        assert_eq!(
            time_to_iso(time_from_iso("2019-12-31T23:59:59.99").unwrap()),
            "2019-12-31T23:59:59.99"
        );
        assert_eq!(
            time_to_iso(time_from_iso("2020-01-01T00:00:00").unwrap()),
            "2020-01-01T00:00:00.00"
        );
        // A moment that rounds up to midnight is the next day.
        assert_eq!(time_to_iso(45_000.0 - 1e-9), time_to_iso(45_000.0));
        for bad in [
            "",
            "2024-03-01",
            "yesterday",
            "2024-13-01T00:00:00",
            "2024-03-01T25:00:00",
            "2024-03-01Tnoon",
        ] {
            assert_eq!(time_from_iso(bad), None, "{bad}");
        }
        // Now is in this century, and local time is within a day of UTC.
        let now = now_utc();
        assert!(now > date_to_serial(2025, 1, 1).unwrap() && now < date_to_serial(2100, 1, 1).unwrap());
        assert!((utc_to_local(now) - now).abs() <= 1.0);
        let offset = utc_to_local(serial) - serial;
        assert!(
            ((offset * 96.0).round() - offset * 96.0).abs() < 1e-6,
            "time zones are whole quarters of an hour"
        );
    }

    #[test]
    fn mentions() {
        let mut w = wb(&[]);
        let (ada, bob) = (Person::new("Ada"), Person::new("Bob"));
        let cy = Person::mentioned("cy@example.com", "cy@example.com");
        assert_eq!(cy.provider_id, "PeoplePicker");
        let at = |person: &Person, start: usize, len: usize| Mention {
            person: person.clone(),
            start,
            len,
        };
        // "é" takes two bytes: offsets are bytes, and must fall between
        // characters.
        let text = "@Bob vérifie, puis @cy@example.com";
        let post = Post::new(&ada, NOON, text).with_mentions(vec![
            at(&cy, 20, 15),
            at(&bob, 0, 4),
            at(&bob, 2, 4),
            at(&bob, 7, 1),
            at(&bob, 30, 40),
            at(&bob, 12, 0),
        ]);
        assert_eq!(post.mentions, [at(&bob, 0, 4), at(&cy, 20, 15)]);
        assert_eq!(
            post.pieces(),
            [
                ("@Bob", Some(&bob)),
                (" vérifie, puis ", None),
                ("@cy@example.com", Some(&cy))
            ]
        );
        assert!(w.add_comment_post(0, 0, 0, post.clone()));
        assert!(w.add_reply_post(0, 0, 0, Post::new(&bob, NOON, "@Ada ok").with_mentions(vec![at(&ada, 0, 4)])));
        w.commit("mentions");
        assert_eq!(thread(&w, "A1").first, post);
        // Authors and people named, each once, as they come.
        assert_eq!(w.people(), [ada.clone(), bob.clone(), cy.clone()]);
        // A plain edit names nobody any more; an edit can name people.
        assert!(w.edit_comment(0, 0, 0, Some(0), "ok"));
        assert!(thread(&w, "A1").replies[0].mentions.is_empty());
        assert!(w.edit_comment_mentions(0, 0, 0, None, "voir @Bob", vec![at(&bob, 5, 4)]));
        w.commit("edit");
        let first = &thread(&w, "A1").first;
        assert_eq!((first.author.clone(), first.time), (ada.clone(), NOON));
        assert_eq!(first.pieces(), [("voir ", None), ("@Bob", Some(&bob))]);
        assert_eq!(w.people(), [ada, bob]);
        w.undo();
        assert_eq!(thread(&w, "A1").first, post);
        // The note standing in for the thread says the names as text.
        assert!(thread(&w, "A1").legacy_text(Locale::en()).contains("@Bob vérifie"));
    }

    #[test]
    fn legacy_text_of_a_thread() {
        let mut t = Thread {
            first: Post::new(&Person::new("Ada"), NOON, "Why?"),
            replies: Vec::new(),
            resolved: false,
        };
        let text = t.legacy_text(Locale::en());
        assert!(
            text.starts_with("[Threaded comment]\n\nYour version of Excel allows you to read this threaded comment;")
        );
        assert!(text.ends_with("linkid=870924\n\nComment:\n    Why?"));
        t.replies.push(Post::new(&Person::new("Bob"), NOON, "Because."));
        assert!(
            t.legacy_text(Locale::en())
                .ends_with("Comment:\n    Why?\nReply:\n    Because.")
        );
        assert!(
            t.legacy_text(Locale::fr())
                .contains("Commentaire\u{a0}:\n    Why?\nRéponse\u{a0}:\n    Because.")
        );
    }
}
