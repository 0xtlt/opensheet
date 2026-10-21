//! Comments (threads with replies and mentions) and notes: `Comment`,
//! `CommentReply` and `Note`.
//!
//! A comment is held by its cell, a reply by its place in its thread. The
//! author of what a script writes is the user of the application
//! (`Context::user_name`), and the time is now.

use serde_json::{Value as J, json};

use opensheet_engine::comment::{self, Comment, Mention, Note, Person, Post, Thread};
use opensheet_engine::drawing::Frame;
use opensheet_engine::protect::Action;
use opensheet_engine::refs::Rect;

use super::util::range_arg;
use super::{ApiClass, Call, Res, Ret, class, enum_value, object};
use crate::host::Host;

/// Pixels at 100 % for a length in points, and back.
const PX_PER_POINT: f64 = 96.0 / 72.0;

fn cell_object(h: &Host, class: &str, sheet: usize, cell: (u32, u32)) -> J {
    object(
        class,
        vec![h.sheet_id(sheet).into(), cell.0.into(), cell.1.into()],
    )
}

fn reply_object(h: &Host, sheet: usize, cell: (u32, u32), reply: usize) -> J {
    object(
        "CommentReply",
        vec![
            h.sheet_id(sheet).into(),
            cell.0.into(),
            cell.1.into(),
            reply.into(),
        ],
    )
}

/// The sheet and the cell an object of these classes stands for.
fn place(h: &Host, call: &Call) -> Res<(usize, (u32, u32))> {
    let sheet = h.sheet(call, call.this_int(0))?;
    let number = |index: usize| u32::try_from(call.this_int(index)).map_err(|_| call.invalid());
    Ok((sheet, (number(1)?, number(2)?)))
}

fn thread<'h>(h: &'h Host, call: &Call) -> Res<(usize, (u32, u32), &'h Thread)> {
    let (sheet, cell) = place(h, call)?;
    let thread =
        h.wb.comment_at(sheet, cell.0, cell.1)
            .and_then(Comment::as_thread)
            .ok_or_else(|| call.gone())?;
    Ok((sheet, cell, thread))
}

/// The post a Comment (its first) or a CommentReply stands for, with the
/// place of the reply in the thread.
fn post<'h>(h: &'h Host, call: &Call) -> Res<(usize, (u32, u32), Option<usize>, &'h Post)> {
    let (sheet, cell, thread) = thread(h, call)?;
    if call.this.len() < 4 {
        return Ok((sheet, cell, None, &thread.first));
    }
    let index = usize::try_from(call.this_int(3)).map_err(|_| call.invalid())?;
    let reply = thread.replies.get(index).ok_or_else(|| call.gone())?;
    Ok((sheet, cell, Some(index), reply))
}

fn note<'h>(h: &'h Host, call: &Call) -> Res<(usize, (u32, u32), &'h Note)> {
    let (sheet, cell) = place(h, call)?;
    let note =
        h.wb.comment_at(sheet, cell.0, cell.1)
            .and_then(Comment::as_note)
            .ok_or_else(|| call.gone())?;
    Ok((sheet, cell, note))
}

/// The one cell an address or a Range argument names.
fn cell_arg(h: &mut Host, call: &Call, index: usize, sheet: usize) -> Res<(usize, (u32, u32))> {
    let (on, rect) = range_arg(h, call, index, sheet)?;
    Ok((on, (rect.r0, rect.c0)))
}

// ---- identifiers -------------------------------------------------------------------

fn comment_id(h: &Host, sheet: usize, cell: (u32, u32)) -> String {
    format!("c{}-{}-{}", h.sheet_id(sheet), cell.0, cell.1)
}

/// The sheet, the cell and the reply an identifier names.
fn parse_id(h: &Host, id: &str) -> Option<(usize, (u32, u32), Option<usize>)> {
    let mut parts = id.strip_prefix('c')?.split('-');
    let sheet = h.sheet_index(parts.next()?.parse().ok()?)?;
    let cell = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    let reply = match parts.next() {
        Some(reply) => Some(reply.parse().ok()?),
        None => None,
    };
    parts.next().is_none().then_some((sheet, cell, reply))
}

// ---- texts and mentions --------------------------------------------------------------

/// The text and the mentions of a content given by a script: plain text,
/// or `{ richContent, mentions }` where `<at id="0">Ann</at>` names the
/// mention of that id.
fn content(call: &Call, index: usize) -> Res<(String, Vec<Mention>)> {
    let given = call.arg(index);
    if let Some(text) = given.as_str() {
        return Ok((text.to_string(), Vec::new()));
    }
    let rich = given
        .get("richContent")
        .and_then(J::as_str)
        .ok_or_else(|| call.invalid())?;
    let people = given
        .get("mentions")
        .and_then(J::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let person = |id: u64| {
        let found = people
            .iter()
            .find(|p| p.get("id").and_then(J::as_u64) == Some(id))?;
        let field = |key: &str| found.get(key).and_then(J::as_str).unwrap_or_default();
        Some(Person::mentioned(field("name"), field("email")))
    };
    let (mut text, mut mentions, mut rest) = (String::new(), Vec::new(), rich);
    while let Some(at) = rest.find("<at id=\"") {
        let tag = &rest[at + 8..];
        let parsed = (|| {
            let quote = tag.find('"')?;
            let id: u64 = tag[..quote].parse().ok()?;
            let inner = tag[quote..].strip_prefix("\">")?;
            let end = inner.find("</at>")?;
            Some((person(id)?, &inner[..end], &inner[end + 5..]))
        })();
        let Some((person, shown, after)) = parsed else {
            return Err(call.invalid());
        };
        text.push_str(&rest[..at]);
        let name = format!("@{}", shown.trim_start_matches('@'));
        mentions.push(Mention {
            person,
            start: text.len(),
            len: name.len(),
        });
        text.push_str(&name);
        rest = after;
    }
    text.push_str(rest);
    Ok((text, mentions))
}

/// The text of a post with its mentions marked, as `getRichContent`
/// gives it.
fn rich(post: &Post) -> String {
    let mut out = String::new();
    let mut id = 0;
    for (piece, person) in post.pieces() {
        match person {
            Some(_) => {
                out.push_str(&format!(
                    "<at id=\"{id}\">{}</at>",
                    piece.trim_start_matches('@')
                ));
                id += 1;
            }
            None => out.push_str(piece),
        }
    }
    out
}

fn mentions(post: &Post) -> J {
    let list: Vec<J> = post
        .mentions
        .iter()
        .enumerate()
        .map(|(id, mention)| json!({ "email": email(&mention.person), "id": id, "name": mention.person.name }))
        .collect();
    J::from(list)
}

/// The address a person is known by, when they are known by one.
fn email(person: &Person) -> &str {
    if person.user_id.contains('@') {
        &person.user_id
    } else {
        ""
    }
}

/// A moment of the workbook (a serial in UTC) as milliseconds since 1970.
fn date(serial: f64) -> Ret {
    Ret::Date((serial - 25569.0) * 86_400_000.0)
}

fn refused(call: &Call) -> super::Fail {
    call.fail("The comment thread is resolved: reopen it to change it.")
}

// ---- adding and finding ----------------------------------------------------------------

/// `addComment`, for the workbook (`sheet` is the active one) or a sheet.
pub(super) fn add(h: &mut Host, call: &Call, sheet: usize) -> Res<Ret> {
    let (on, cell) = cell_arg(h, call, 0, sheet)?;
    let (text, mentions) = content(call, 1)?;
    enum_value(call, 2, "ContentType")?;
    h.permit(call, &Action::EditComment { sheet: on })?;
    let author = Person::new(&h.user_name);
    let post = Post::new(&author, comment::now_utc(), &text).with_mentions(mentions);
    if !h.wb.add_comment_post(on, cell.0, cell.1, post) {
        return Err(call.fail("The cell already has a comment or a note."));
    }
    Ok(Ret::Object(cell_object(h, "Comment", on, cell)))
}

/// `addNote`.
pub(super) fn add_note(h: &mut Host, call: &Call, sheet: usize) -> Res<Ret> {
    let (on, cell) = cell_arg(h, call, 0, sheet)?;
    let text = call.str(1)?;
    h.permit(call, &Action::EditComment { sheet: on })?;
    if h.wb.comment_at(on, cell.0, cell.1).is_some() {
        return Err(call.fail("The cell already has a comment or a note."));
    }
    let author = h.user_name.clone();
    h.wb.set_note(on, cell.0, cell.1, &author, text);
    Ok(Ret::Object(cell_object(h, "Note", on, cell)))
}

/// The comments (`threads`) or the notes of a sheet or of every sheet, in
/// the order of the cells.
pub(super) fn all(h: &Host, only: Option<usize>, threads: bool) -> Ret {
    let mut out = Vec::new();
    for (sheet, sh) in
        h.wb.sheets
            .iter()
            .enumerate()
            .filter(|(index, _)| only.is_none_or(|s| s == *index))
    {
        for (cell, held) in &sh.comments {
            if held.as_thread().is_some() == threads {
                out.push(cell_object(
                    h,
                    if threads { "Comment" } else { "Note" },
                    sheet,
                    *cell,
                ));
            }
        }
    }
    Ret::Object(J::from(out))
}

/// `getComment(id)`.
pub(super) fn by_id(h: &Host, call: &Call, only: Option<usize>) -> Res<Ret> {
    let found = parse_id(h, call.str(0)?).filter(|(sheet, cell, reply)| {
        reply.is_none()
            && only.is_none_or(|s| s == *sheet)
            && h.wb
                .comment_at(*sheet, cell.0, cell.1)
                .is_some_and(|c| c.as_thread().is_some())
    });
    Ok(Ret::maybe(found.map(|(sheet, cell, _)| {
        cell_object(h, "Comment", sheet, cell)
    })))
}

/// `getCommentByCell`.
pub(super) fn by_cell(h: &mut Host, call: &Call, sheet: usize) -> Res<Ret> {
    let (on, cell) = cell_arg(h, call, 0, sheet)?;
    match h
        .wb
        .comment_at(on, cell.0, cell.1)
        .and_then(Comment::as_thread)
    {
        Some(_) => Ok(Ret::Object(cell_object(h, "Comment", on, cell))),
        None => Err(call.gone()),
    }
}

/// `getCommentByReplyId`.
pub(super) fn by_reply(h: &Host, call: &Call, only: Option<usize>) -> Res<Ret> {
    let (sheet, cell, reply) = parse_id(h, call.str(0)?).ok_or_else(|| call.gone())?;
    let thread =
        h.wb.comment_at(sheet, cell.0, cell.1)
            .and_then(Comment::as_thread)
            .ok_or_else(|| call.gone())?;
    if only.is_some_and(|s| s != sheet) || reply.is_none_or(|index| index >= thread.replies.len()) {
        return Err(call.gone());
    }
    Ok(Ret::Object(cell_object(h, "Comment", sheet, cell)))
}

/// `getNote(key)`: by cell address, or by place among the notes.
pub(super) fn note_by_key(h: &mut Host, call: &Call, sheet: usize) -> Res<Ret> {
    if let Some(index) = call.arg(0).as_u64() {
        let cell = h.wb.sheets[sheet]
            .comments
            .iter()
            .filter(|(_, held)| held.as_note().is_some())
            .nth(index as usize)
            .map(|(cell, _)| *cell);
        return Ok(Ret::maybe(
            cell.map(|cell| cell_object(h, "Note", sheet, cell)),
        ));
    }
    let (on, cell) = cell_arg(h, call, 0, sheet)?;
    let found =
        h.wb.comment_at(on, cell.0, cell.1)
            .and_then(Comment::as_note)
            .is_some();
    Ok(Ret::maybe(found.then(|| cell_object(h, "Note", on, cell))))
}

// ---- what comments and replies share -----------------------------------------------------

fn set_content(h: &mut Host, call: &Call, text: &str, mentions: Vec<Mention>) -> Res<Ret> {
    let (sheet, cell, reply, _) = post(h, call)?;
    h.permit(call, &Action::EditComment { sheet })?;
    if !h
        .wb
        .edit_comment_mentions(sheet, cell.0, cell.1, reply, text, mentions)
    {
        return Err(refused(call));
    }
    Ok(Ret::Nothing)
}

macro_rules! post_members {
    ($name:literal, $doc:literal, { $($own:tt)* }) => {
        class!($name, $doc, {
            getId "(): string" "An identifier for this run of the script." => |h, c| {
                let (sheet, cell, reply, _) = post(h, c)?;
                let id = comment_id(h, sheet, cell);
                Ok(reply.map_or(id.clone(), |index| format!("{id}-{index}")).into())
            };
            getContent "(): string" "The text; people it names appear as @Name." => |h, c| Ok(post(h, c)?.3.text.clone().into());
            setContent "(content: string): void" "Replaces the text; it then names nobody. Refused while the thread is resolved." => |h, c| set_content(h, c, c.str(0)?, Vec::new());
            getRichContent "(): string" "The text with the people it names marked: <at id=\"0\">Name</at>." => |h, c| Ok(rich(post(h, c)?.3).into());
            getContentType "(): ExcelScript.ContentType" "Mention when the text names people, else Plain." => |h, c| {
                Ok(if post(h, c)?.3.mentions.is_empty() { "Plain" } else { "Mention" }.into())
            };
            getMentions "(): ExcelScript.CommentMention[]" "The people the text names: { email, id, name }, the id being the one of getRichContent." => |h, c| Ok(Ret::Value(mentions(post(h, c)?.3)));
            updateMentions "(contentWithMentions: ExcelScript.CommentRichContent): void" "Replaces the text by one that names people: { richContent, mentions }." => |h, c| {
                if !c.arg(0).is_object() {
                    return Err(c.invalid());
                }
                let (text, mentions) = content(c, 0)?;
                set_content(h, c, &text, mentions)
            };
            getAuthorName "(): string" "The name of who wrote it." => |h, c| Ok(post(h, c)?.3.author.name.clone().into());
            getAuthorEmail "(): string" "The e-mail address of who wrote it; empty text when it is not known." => |h, c| Ok(email(&post(h, c)?.3.author).to_string().into());
            getCreationDate "(): Date" "When it was written." => |h, c| Ok(date(post(h, c)?.3.time));
            getLocation "(): Range" "The cell the thread is on." => |h, c| {
                let (sheet, cell, _, _) = post(h, c)?;
                Ok(Ret::Object(h.range(sheet, Rect::cell(cell.0, cell.1))))
            };
            getResolved "(): boolean" "Whether the thread is resolved." => |h, c| Ok(thread(h, c)?.2.resolved.into());
            $($own)*
        })
    };
}

pub(super) const COMMENT: ApiClass = post_members!("Comment", "A comment on a cell: the first text of a thread, with its replies.", {
    setResolved "(resolved: boolean): void" "Resolves the thread, or reopens it." => |h, c| {
        let (sheet, cell, _) = thread(h, c)?;
        let resolved = c.bool(0)?;
        h.permit(c, &Action::EditComment { sheet })?;
        h.wb.set_comment_resolved(sheet, cell.0, cell.1, resolved);
        Ok(Ret::Nothing)
    };
    addCommentReply "(content: ExcelScript.CommentRichContent | string, contentType?: ExcelScript.ContentType): CommentReply" "Replies to the thread, as the user of the application. Refused while the thread is resolved." => |h, c| {
        let (sheet, cell, held) = thread(h, c)?;
        let index = held.replies.len();
        let (text, mentions) = content(c, 0)?;
        enum_value(c, 1, "ContentType")?;
        h.permit(c, &Action::EditComment { sheet })?;
        let reply = Post::new(&Person::new(&h.user_name), comment::now_utc(), &text).with_mentions(mentions);
        if !h.wb.add_reply_post(sheet, cell.0, cell.1, reply) {
            return Err(refused(c));
        }
        Ok(Ret::Object(reply_object(h, sheet, cell, index)))
    };
    getReplies "(): CommentReply[]" "The replies, the oldest first." => |h, c| {
        let (sheet, cell, held) = thread(h, c)?;
        Ok(Ret::Object(J::from((0..held.replies.len()).map(|index| reply_object(h, sheet, cell, index)).collect::<Vec<J>>())))
    };
    getCommentReply "(commentReplyId: string): CommentReply | undefined" "The reply with this identifier; undefined when the thread has none." => |h, c| {
        let (sheet, cell, held) = thread(h, c)?;
        let found = parse_id(h, c.str(0)?).filter(|(of, at, _)| (*of, *at) == (sheet, cell)).and_then(|(_, _, reply)| reply).filter(|index| *index < held.replies.len());
        Ok(Ret::maybe(found.map(|index| reply_object(h, sheet, cell, index))))
    };
    delete "(): void" "Deletes the comment with its replies." => |h, c| {
        let (sheet, cell, _) = thread(h, c)?;
        h.permit(c, &Action::EditComment { sheet })?;
        h.wb.delete_comment(sheet, cell.0, cell.1);
        Ok(Ret::Nothing)
    };
});

pub(super) const COMMENT_REPLY: ApiClass = post_members!("CommentReply", "A reply in the thread of a comment.", {
    getParentComment "(): Comment" "The comment the reply answers." => |h, c| {
        let (sheet, cell, _, _) = post(h, c)?;
        Ok(Ret::Object(cell_object(h, "Comment", sheet, cell)))
    };
    delete "(): void" "Deletes the reply. Refused while the thread is resolved." => |h, c| {
        let (sheet, cell, reply, _) = post(h, c)?;
        h.permit(c, &Action::EditComment { sheet })?;
        if !h.wb.delete_reply(sheet, cell.0, cell.1, reply.ok_or_else(|| c.gone())?) {
            return Err(refused(c));
        }
        Ok(Ret::Nothing)
    };
});

/// Resize the box of a note, keeping its corner.
fn resize_note(h: &mut Host, call: &Call, width: Option<f64>, height: Option<f64>) -> Res<Ret> {
    let (sheet, cell, _) = note(h, call)?;
    let size = |given: Option<f64>| {
        given
            .map(|points| points * PX_PER_POINT)
            .filter(|px| !(*px > 0.0 && *px < 10_000.0))
    };
    if size(width).is_some() || size(height).is_some() {
        return Err(call.invalid());
    }
    h.permit(call, &Action::EditComment { sheet })?;
    let old = h.wb.sheets[sheet]
        .note_frame(cell.0, cell.1)
        .ok_or_else(|| call.gone())?;
    let frame = Frame {
        width: width.map_or(old.width, |points| points * PX_PER_POINT),
        height: height.map_or(old.height, |points| points * PX_PER_POINT),
        ..old
    };
    h.wb.set_note_frame(sheet, cell.0, cell.1, Some(frame));
    Ok(Ret::Nothing)
}

pub(super) const NOTE: ApiClass = class!("Note", "A note on a cell: the yellow box of earlier versions of Excel.", {
    getContent "(): string" "The text of the note." => |h, c| Ok(note(h, c)?.2.text().into());
    setContent "(content: string): void" "Replaces the text of the note." => |h, c| {
        let (sheet, cell, held) = note(h, c)?;
        let author = held.author.clone();
        let text = c.str(0)?;
        h.permit(c, &Action::EditComment { sheet })?;
        h.wb.set_note(sheet, cell.0, cell.1, &author, text);
        Ok(Ret::Nothing)
    };
    getAuthorName "(): string" "The name of who wrote the note." => |h, c| Ok(note(h, c)?.2.author.clone().into());
    getLocation "(): Range" "The cell the note is on." => |h, c| {
        let (sheet, cell, _) = note(h, c)?;
        Ok(Ret::Object(h.range(sheet, Rect::cell(cell.0, cell.1))))
    };
    getVisible "(): boolean" "Whether the note shows all the time rather than under the pointer only." => |h, c| Ok(note(h, c)?.2.visible.into());
    setVisible "(visible: boolean): void" "Shows the note all the time, or under the pointer only (Show/Hide Note)." => |h, c| {
        let (sheet, cell, _) = note(h, c)?;
        let visible = c.bool(0)?;
        h.permit(c, &Action::EditComment { sheet })?;
        h.wb.set_note_visible(sheet, cell.0, cell.1, visible);
        Ok(Ret::Nothing)
    };
    getWidth "(): number" "The width of the box of the note, in points." => |h, c| {
        let (sheet, cell, _) = note(h, c)?;
        Ok((h.wb.sheets[sheet].note_frame(cell.0, cell.1).map_or(0.0, |frame| frame.width) / PX_PER_POINT).into())
    };
    setWidth "(width: number): void" "Sets the width of the box of the note, in points." => |h, c| resize_note(h, c, Some(c.num(0)?), None);
    getHeight "(): number" "The height of the box of the note, in points." => |h, c| {
        let (sheet, cell, _) = note(h, c)?;
        Ok((h.wb.sheets[sheet].note_frame(cell.0, cell.1).map_or(0.0, |frame| frame.height) / PX_PER_POINT).into())
    };
    setHeight "(height: number): void" "Sets the height of the box of the note, in points." => |h, c| resize_note(h, c, None, Some(c.num(0)?));
    delete "(): void" "Deletes the note." => |h, c| {
        let (sheet, cell, _) = note(h, c)?;
        h.permit(c, &Action::EditComment { sheet })?;
        h.wb.delete_comment(sheet, cell.0, cell.1);
        Ok(Ret::Nothing)
    };
});
