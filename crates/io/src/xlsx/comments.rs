//! Notes and comment threads (`Sheet::comments`).
//!
//! A note is kept in two parts: its author and text in `xl/commentsN.xml`,
//! its box in a VML drawing (`xl/drawings/vmlDrawingN.vml`, the drawing
//! format of Office 2003, which lives on for this) that the worksheet names
//! with `<legacyDrawing>`. Excel shows no note without its box; other
//! applications write notes without one, and those are read.
//!
//! A comment thread is kept in `xl/threadedComments/threadedCommentN.xml`
//! and its authors, for the whole workbook, in `xl/persons/person.xml`.
//! Beside every thread Excel writes a note saying what the thread says, for
//! the applications that only know notes; its author is `tc=` followed by
//! the id of the thread.
//!
//! Kept: the author of a note, its text with the font of each run, whether
//! it stays shown and where its box is; the posts of a thread with their
//! authors and times, the people they name (`<mentions>`, whose places
//! count UTF-16 units as Excel does), and whether it is resolved. Not
//! kept: the look of the box (fill, border, margins, alignment, a picture
//! as background), how it follows its cells, people nobody quotes.

use super::chart::Track;
use super::color::{Palette, RawColor};
use super::parts::{Extras, NS_REL_2017, SheetLinks};
use super::points_to_px;
use super::read::{Package, Rel, relationships};
use super::write::{MAX_COL_WIDTH_PX, MAX_ROW_HEIGHT_PT, NS_DOC_REL, NS_MAIN, push_number, push_t};
use super::xml::{Event, Reader, Tag, XML_DECL, decode_cell_text, push_attr, push_cell_text};
use opensheet_engine::comment::{
    Comment, Mention, NOTE_SIZE, Note, NoteBox, NoteRun, Person, Post, Thread, time_from_iso, time_to_iso,
};
use opensheet_engine::drawing::Frame;
use opensheet_engine::refs::{MAX_COLS, MAX_ROWS, cell_name, parse_cell_name};
use opensheet_engine::style::{Font, Underline};
use opensheet_engine::workbook::{DEFAULT_COL_WIDTH, DEFAULT_ROW_HEIGHT, Sheet};
use opensheet_engine::{Color, Workbook};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{Read, Seek};
use std::sync::Arc;

const NS_THREADS: &str = "http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments";
const NS_REVISION: &str = "http://schemas.microsoft.com/office/spreadsheetml/2014/revision";
const NS_COMPATIBILITY: &str = "http://schemas.openxmlformats.org/markup-compatibility/2006";

const CT_COMMENTS: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml";
const CT_VML: &str = "application/vnd.openxmlformats-officedocument.vmlDrawing";
const CT_THREADS: &str = "application/vnd.ms-excel.threadedcomments+xml";
const CT_PERSONS: &str = "application/vnd.ms-excel.person+xml";

/// What starts the author of the note standing in for a thread.
const THREAD_AUTHOR: &str = "tc=";

/// The colour of the text of notes in the legacy palette: the system's
/// "tooltip text". It is what Excel writes for automatic.
const INDEXED_NOTE_TEXT: u32 = 81;

/// Shape ids are handed out in blocks of this many, one block per
/// `<o:idmap>` number.
const SHAPE_ID_BLOCK: usize = 1024;

// ---- writing -------------------------------------------------------------

/// 64 bits of FNV-1a over `data`, continuing from `hash`.
fn fnv(mut hash: u64, data: &[u8]) -> u64 {
    for byte in data {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01B3);
    }
    hash
}

/// An identifier in the form Excel gives to threads and people: a GUID in
/// braces and capitals. The model keeps none, so they are derived from what
/// they name (`kind` and `fields`): the same workbook always gives the same
/// file, and a person keeps one id across the sheets and from save to save.
fn guid(kind: &str, fields: &[&[u8]]) -> String {
    // Spread what FNV leaves in the low bits over the whole word.
    let mix = |mut z: u64| {
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    let hash = |seed: u64| {
        let mut hash = fnv(seed, kind.as_bytes());
        for field in fields {
            // Lengths keep ("ab", "c") apart from ("a", "bc").
            hash = fnv(hash, &(field.len() as u64).to_le_bytes());
            hash = fnv(hash, field);
        }
        mix(hash)
    };
    let (a, b) = (hash(0xCBF2_9CE4_8422_2325), hash(0x6C62_272E_07BB_0142));
    // The fixed digits are those of a random GUID (version 4), which is
    // what Excel makes.
    format!(
        "{{{:08X}-{:04X}-4{:03X}-{:04X}-{:012X}}}",
        a >> 32,
        (a >> 16) & 0xFFFF,
        a & 0xFFF,
        0x8000 | ((b >> 48) & 0x3FFF),
        b & 0xFFFF_FFFF_FFFF
    )
}

fn person_id(person: &Person) -> String {
    guid(
        "person",
        &[
            person.name.as_bytes(),
            person.user_id.as_bytes(),
            person.provider_id.as_bytes(),
        ],
    )
}

/// The id of a post of the thread of a cell: 0 is the comment that starts
/// it, 1 its first reply.
fn post_id(sheet: usize, (row, col): (u32, u32), post: usize) -> String {
    guid(
        "post",
        &[
            &(sheet as u64).to_le_bytes(),
            &row.to_le_bytes(),
            &col.to_le_bytes(),
            &(post as u64).to_le_bytes(),
        ],
    )
}

/// The `dT` of a post; none when its time is no date a file can hold
/// (a post read from a file that gave none has time 0).
fn post_time(serial: f64) -> Option<String> {
    // From March 1900, where serials and the calendar agree, to the last
    // day Excel knows.
    (serial.is_finite() && (61.0..2_958_466.0).contains(&serial)).then(|| time_to_iso(serial))
}

/// The box of a note or of the stand-in of a thread, as a VML shape.
struct Shape {
    cell: (u32, u32),
    /// Left, top, width and height in whole pixels from the corner of A1.
    frame: [f64; 4],
    /// The `Anchor`: column, offset, row, offset of the top-left corner,
    /// then of the bottom-right one, offsets in pixels.
    anchor: [u32; 8],
    visible: bool,
}

/// Where the box of the note of a cell goes in the file. Notes have a
/// frame; the stand-in of a thread gets the place of a note that was never
/// moved (see `Sheet::note_frame`).
fn shape(sheet: &Sheet, (row, col): (u32, u32), visible: bool) -> Shape {
    // The geometry Excel will see: the sizes as they are written.
    let cols = Track {
        default: DEFAULT_COL_WIDTH,
        sizes: &sheet.col_widths,
        hidden: &sheet.hidden_cols,
        max: MAX_COL_WIDTH_PX,
        count: MAX_COLS,
    };
    let rows = Track {
        default: DEFAULT_ROW_HEIGHT,
        sizes: &sheet.row_heights,
        hidden: &sheet.hidden_rows,
        max: points_to_px(MAX_ROW_HEIGHT_PT),
        count: MAX_ROWS,
    };
    let frame = sheet.note_frame(row, col).unwrap_or_else(|| {
        Frame::new(
            cols.span(0, (col + 1).min(MAX_COLS - 1)) + 15.0,
            (rows.span(0, row) - 10.0).max(2.0),
            NOTE_SIZE.0 as f64,
            NOTE_SIZE.1 as f64,
        )
    });
    let whole = |px: f64| {
        if px.is_finite() {
            px.max(0.0).round()
        } else {
            0.0
        }
    };
    let (x, y) = (whole(frame.x), whole(frame.y));
    let (width, height) = (whole(frame.width).max(1.0), whole(frame.height).max(1.0));
    let (c0, dx0) = cols.locate(0, x);
    let (r0, dy0) = rows.locate(0, y);
    let (c1, dx1) = cols.locate(0, x + width);
    let (r1, dy1) = rows.locate(0, y + height);
    let px = |offset: f64| offset.round() as u32;
    Shape {
        cell: (row, col),
        frame: [x, y, width, height],
        anchor: [c0, px(dx0), r0, px(dy0), c1, px(dx1), r1, px(dy1)],
        visible,
    }
}

/// The VML drawing of the notes of one sheet, shape for shape as Excel
/// writes it for new notes: boxes in the colours of the system's tooltips,
/// text flush left. `first_block` is the first `idmap` number of the sheet.
fn vml_xml(shapes: &[Shape], first_block: usize) -> String {
    let mut x = String::from(
        "<xml xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:o=\"urn:schemas-microsoft-com:office:office\" xmlns:x=\"urn:schemas-microsoft-com:office:excel\">",
    );
    x.push_str("<o:shapelayout v:ext=\"edit\"><o:idmap v:ext=\"edit\" data=\"");
    for block in 0..blocks(shapes.len()) {
        if block > 0 {
            x.push(',');
        }
        w!(x, "{}", first_block + block);
    }
    x.push_str("\"/></o:shapelayout>");
    x.push_str("<v:shapetype id=\"_x0000_t202\" coordsize=\"21600,21600\" o:spt=\"202\" path=\"m,l,21600r21600,l21600,xe\"><v:stroke joinstyle=\"miter\"/><v:path gradientshapeok=\"t\" o:connecttype=\"rect\"/></v:shapetype>");
    for (i, shape) in shapes.iter().enumerate() {
        w!(
            x,
            "<v:shape id=\"_x0000_s{}\" type=\"#_x0000_t202\" style=\"position:absolute;",
            first_block * SHAPE_ID_BLOCK + i + 1
        );
        // The style is in points; pixels are three quarters of one.
        for (name, px) in ["margin-left", "margin-top", "width", "height"]
            .iter()
            .zip(shape.frame)
        {
            w!(x, "{name}:");
            push_number(&mut x, px * 0.75);
            x.push_str("pt;");
        }
        w!(x, "z-index:{}", i + 1);
        if !shape.visible {
            x.push_str(";visibility:hidden");
        }
        x.push_str(
            "\" fillcolor=\"infoBackground [80]\" strokecolor=\"none [81]\" o:insetmode=\"auto\">",
        );
        x.push_str("<v:fill color2=\"infoBackground [80]\"/><v:shadow color=\"none [81]\" obscured=\"t\"/><v:path o:connecttype=\"none\"/>");
        x.push_str("<v:textbox style=\"mso-direction-alt:auto\"><div style=\"text-align:left\"></div></v:textbox>");
        x.push_str(
            "<x:ClientData ObjectType=\"Note\"><x:MoveWithCells/><x:SizeWithCells/><x:Anchor>",
        );
        for (i, n) in shape.anchor.iter().enumerate() {
            if i > 0 {
                x.push_str(", ");
            }
            w!(x, "{n}");
        }
        w!(
            x,
            "</x:Anchor><x:AutoFill>False</x:AutoFill><x:Row>{}</x:Row><x:Column>{}</x:Column>",
            shape.cell.0,
            shape.cell.1
        );
        if shape.visible {
            x.push_str("<x:Visible/>");
        }
        x.push_str("</x:ClientData></v:shape>");
    }
    x.push_str("</xml>");
    x
}

/// How many blocks of shape ids a drawing of `shapes` shapes takes.
fn blocks(shapes: usize) -> usize {
    shapes / SHAPE_ID_BLOCK + 1
}

/// Append a run of the text of a note.
fn push_run(x: &mut String, run: &NoteRun) {
    let font = &run.font;
    x.push_str("<r><rPr>");
    if font.bold {
        x.push_str("<b/>");
    }
    if font.italic {
        x.push_str("<i/>");
    }
    if font.strike {
        x.push_str("<strike/>");
    }
    match font.underline {
        Underline::None => {}
        Underline::Single => x.push_str("<u/>"),
        Underline::Double => x.push_str("<u val=\"double\"/>"),
    }
    w!(x, "<sz val=\"{}\"/>", font.size.max(2) as f64 / 2.0);
    match font.color {
        Some(color) => {
            w!(x, "<color rgb=\"FF{:06X}\"/>", color.value() & 0xFF_FFFF);
        }
        None => {
            w!(x, "<color indexed=\"{INDEXED_NOTE_TEXT}\"/>");
        }
    }
    x.push_str("<rFont val=\"");
    push_attr(x, &font.name);
    x.push_str("\"/></rPr>");
    push_t(x, &run.text);
    x.push_str("</r>");
}

/// How many UTF-16 units the first `bytes` bytes of a text take: files
/// count the places of mentions that way.
fn utf16_len(text: &str, bytes: usize) -> usize {
    text.get(..bytes).map_or(0, |head| head.encode_utf16().count())
}

/// The byte offset of the character that starts `units` UTF-16 units into
/// a text; `None` past its end or inside a character.
fn byte_offset(text: &str, units: usize) -> Option<usize> {
    let mut seen = 0;
    for (offset, ch) in text.char_indices() {
        if seen == units {
            return Some(offset);
        }
        seen += ch.len_utf16();
    }
    (seen == units).then_some(text.len())
}

/// The id of a mention: that of its post and its place among the
/// mentions of the post.
fn mention_id(post: &str, index: usize) -> String {
    guid("mention", &[post.as_bytes(), &(index as u64).to_le_bytes()])
}

/// Append a post of a thread. `parent` is the id of the comment a reply
/// answers; `done` marks the first comment of a resolved thread.
fn push_post(x: &mut String, cell: &str, post: &Post, id: &str, parent: Option<&str>, done: bool) {
    w!(x, "<threadedComment ref=\"{cell}\"");
    if let Some(time) = post_time(post.time) {
        w!(x, " dT=\"{time}\"");
    }
    w!(x, " personId=\"{}\" id=\"{id}\"", person_id(&post.author));
    if let Some(parent) = parent {
        w!(x, " parentId=\"{parent}\"");
    }
    if done {
        x.push_str(" done=\"1\"");
    }
    x.push_str("><text>");
    push_cell_text(x, &post.text);
    x.push_str("</text>");
    if !post.mentions.is_empty() {
        x.push_str("<mentions>");
        for (index, mention) in post.mentions.iter().enumerate() {
            let start = utf16_len(&post.text, mention.start);
            let end = utf16_len(&post.text, mention.start + mention.len);
            w!(
                x,
                "<mention mentionpersonId=\"{}\" mentionId=\"{}\" startIndex=\"{start}\" length=\"{}\"/>",
                person_id(&mention.person),
                mention_id(id, index),
                end - start
            );
        }
        x.push_str("</mentions>");
    }
    x.push_str("</threadedComment>");
}

/// The people of a workbook's threads (`xl/persons/person.xml`).
fn persons_xml(people: &[&Person]) -> String {
    let mut x = String::from(XML_DECL);
    w!(
        x,
        "<personList xmlns=\"{NS_THREADS}\" xmlns:x=\"{NS_MAIN}\">"
    );
    for person in people {
        x.push_str("<person displayName=\"");
        push_attr(&mut x, &person.name);
        w!(x, "\" id=\"{}\" userId=\"", person_id(person));
        push_attr(&mut x, &person.user_id);
        x.push_str("\" providerId=\"");
        // People without an account are "None" to Excel.
        push_attr(
            &mut x,
            if person.provider_id.is_empty() {
                "None"
            } else {
                &person.provider_id
            },
        );
        x.push_str("\"/>");
    }
    x.push_str("</personList>");
    x
}

/// What the notes and comment threads of a workbook add to its package.
pub(crate) fn add_parts(wb: &Workbook, extras: &mut Extras) {
    let mut people: Vec<&Person> = Vec::new();
    // Parts are numbered in the order of the sheets that need them.
    let (mut notes_parts, mut thread_parts) = (0, 0);
    let mut next_block = 1;
    for (index, sheet) in wb.sheets.iter().enumerate() {
        let links = SheetLinks::of(sheet);
        let (Some(vml_id), Some(comments_id)) = (links.vml, links.comments) else {
            continue;
        };
        // Authors are listed once and named by their place in the list.
        let mut authors: Vec<String> = Vec::new();
        let mut places: HashMap<String, usize> = HashMap::new();
        let mut author_id = |name: &str| {
            *places.entry(name.to_string()).or_insert_with(|| {
                authors.push(name.to_string());
                authors.len() - 1
            })
        };
        let mut list = String::new();
        let mut threads = String::new();
        let mut shapes = Vec::with_capacity(sheet.comments.len());
        for (&cell, comment) in &sheet.comments {
            let name = cell_name(cell.0, cell.1);
            w!(list, "<comment ref=\"{name}\"");
            match comment {
                Comment::Note(note) => {
                    let author = author_id(&note.author);
                    w!(list, " authorId=\"{author}\" shapeId=\"0\"><text>");
                    for run in &note.runs {
                        push_run(&mut list, run);
                    }
                    if note.runs.is_empty() {
                        list.push_str("<t/>");
                    }
                }
                Comment::Thread(thread) => {
                    let id = post_id(index, cell, 0);
                    let author = author_id(&format!("{THREAD_AUTHOR}{id}"));
                    w!(
                        list,
                        " authorId=\"{author}\" shapeId=\"0\" xr:uid=\"{id}\"><text>"
                    );
                    push_t(&mut list, &thread.legacy_text(wb.locale));
                    for (n, post) in thread.posts().enumerate() {
                        if n == 0 {
                            push_post(&mut threads, &name, post, &id, None, thread.resolved);
                        } else {
                            let reply = post_id(index, cell, n);
                            push_post(&mut threads, &name, post, &reply, Some(&id), false);
                        }
                        // Those who write, then those they name.
                        let named = post.mentions.iter().map(|mention| &mention.person);
                        for person in std::iter::once(&post.author).chain(named) {
                            if !people.contains(&person) {
                                people.push(person);
                            }
                        }
                    }
                }
            }
            list.push_str("</text></comment>");
            let visible = comment.as_note().is_some_and(|note| note.visible);
            shapes.push(shape(sheet, cell, visible));
        }

        notes_parts += 1;
        extras.add_default("vml", CT_VML);
        extras.add_part(
            format!("xl/drawings/vmlDrawing{notes_parts}.vml"),
            None,
            vml_xml(&shapes, next_block),
        );
        extras.relate_sheet(
            index,
            vml_id,
            &format!("{NS_DOC_REL}/vmlDrawing"),
            &format!("../drawings/vmlDrawing{notes_parts}.vml"),
        );

        let mut x = String::from(XML_DECL);
        w!(x, "<comments xmlns=\"{NS_MAIN}\"");
        if links.threads.is_some() {
            // The ids tying the stand-ins to their threads are an
            // extension older readers are told they may skip.
            w!(
                x,
                " xmlns:mc=\"{NS_COMPATIBILITY}\" mc:Ignorable=\"xr\" xmlns:xr=\"{NS_REVISION}\""
            );
        }
        x.push_str("><authors>");
        for author in &authors {
            x.push_str("<author>");
            push_cell_text(&mut x, author);
            x.push_str("</author>");
        }
        w!(x, "</authors><commentList>{list}</commentList></comments>");
        extras.add_part(
            format!("xl/comments{notes_parts}.xml"),
            Some(CT_COMMENTS),
            x,
        );
        extras.relate_sheet(
            index,
            comments_id,
            &format!("{NS_DOC_REL}/comments"),
            &format!("../comments{notes_parts}.xml"),
        );
        next_block += blocks(shapes.len());

        if let Some(threads_id) = links.threads {
            thread_parts += 1;
            extras.add_part(
                format!("xl/threadedComments/threadedComment{thread_parts}.xml"),
                Some(CT_THREADS),
                format!(
                    "{XML_DECL}<ThreadedComments xmlns=\"{NS_THREADS}\" xmlns:x=\"{NS_MAIN}\">{threads}</ThreadedComments>"
                ),
            );
            extras.relate_sheet(
                index,
                threads_id,
                &format!("{NS_REL_2017}/threadedComment"),
                &format!("../threadedComments/threadedComment{thread_parts}.xml"),
            );
        }
    }
    if !people.is_empty() {
        extras.add_part(
            "xl/persons/person.xml".to_string(),
            Some(CT_PERSONS),
            persons_xml(&people),
        );
        extras.relate_workbook(&format!("{NS_REL_2017}/person"), "persons/person.xml");
    }
}

// ---- reading -------------------------------------------------------------

/// What the notes and threads of every sheet are read against.
pub(crate) struct Context<'a> {
    /// The people of the threads, by their id in capitals.
    persons: HashMap<String, Person>,
    palette: &'a Palette,
    /// The font of the text of notes that names none: the font of the
    /// workbook, as in Excel.
    font: Font,
}

impl<'a> Context<'a> {
    /// `workbook_rels` leads to the people; `font` is the font of the
    /// cells without a format.
    pub fn read<R: Read + Seek>(
        pkg: &mut Package<R>,
        workbook_rels: &[Rel],
        palette: &'a Palette,
        font: &Font,
    ) -> Context<'a> {
        let mut persons = HashMap::new();
        for rel in workbook_rels.iter().filter(|rel| rel.kind == "person") {
            if let Ok(Some(xml)) = pkg.read(&rel.target) {
                parse_persons(&xml, &mut persons);
            }
        }
        Context {
            persons,
            palette,
            font: font.clone(),
        }
    }
}

fn parse_persons(xml: &str, persons: &mut HashMap<String, Person>) {
    let mut r = Reader::new(xml);
    loop {
        match r.next() {
            Event::Open(t) if t.name == "person" => {
                let text = |name: &str| t.attr(name).map(|v| v.into_owned());
                if let Some(id) = text("id") {
                    persons.entry(id.to_uppercase()).or_insert(Person {
                        name: text("displayName").unwrap_or_default(),
                        user_id: text("userId").unwrap_or_default(),
                        provider_id: text("providerId").unwrap_or_else(|| "None".to_string()),
                    });
                }
                r.skip(&t);
            }
            Event::Eof => break,
            _ => {}
        }
    }
}

/// A `<threadedComment>`.
struct RawPost {
    cell: Option<(u32, u32)>,
    id: String,
    parent: Option<String>,
    done: bool,
    post: Post,
}

/// The threads of a part, with their cells. Excel writes a thread as its
/// first comment followed by the replies, which name it as their parent.
/// Whatever else a file holds is kept as close to that as it can be: a
/// reply to a reply joins the thread, a reply to nothing joins the thread
/// of its cell or starts one.
fn parse_threads(xml: &str, persons: &HashMap<String, Person>) -> Vec<((u32, u32), Thread)> {
    let mut posts: Vec<RawPost> = Vec::new();
    let mut r = Reader::new(xml);
    loop {
        match r.next() {
            Event::Open(t) if t.name == "threadedComment" => {
                let id = |name: &str| t.attr(name).map(|v| v.trim().to_uppercase());
                let author = id("personId").and_then(|id| persons.get(&id)).cloned();
                let mut text = String::new();
                // Who is named, from which UTF-16 unit, for how many.
                let mut named: Vec<(Person, usize, usize)> = Vec::new();
                if !t.empty {
                    loop {
                        match r.next() {
                            Event::Open(c) if c.name == "text" => {
                                text = decode_cell_text(&r.text(&c)).into_owned();
                            }
                            Event::Open(c) if c.name == "mentions" && !c.empty => loop {
                                match r.next() {
                                    Event::Open(m) if m.name == "mention" => {
                                        let who = m.attr("mentionpersonId").map(|v| v.trim().to_uppercase());
                                        let number = |name: &str| m.attr(name).and_then(|v| v.trim().parse::<usize>().ok());
                                        if let (Some(person), Some(start), Some(length)) = (
                                            who.and_then(|id| persons.get(&id)),
                                            number("startIndex"),
                                            number("length"),
                                        ) {
                                            named.push((person.clone(), start, length));
                                        }
                                        r.skip(&m);
                                    }
                                    Event::Open(m) => r.skip(&m),
                                    Event::Close(_) | Event::Eof => break,
                                    _ => {}
                                }
                            },
                            Event::Open(c) => r.skip(&c),
                            Event::Close(_) | Event::Eof => break,
                            _ => {}
                        }
                    }
                }
                // A mention of nobody the file knows, or that does not fit
                // the text, stays as the text it is.
                let mentions = named
                    .into_iter()
                    .filter_map(|(person, start, length)| {
                        let from = byte_offset(&text, start)?;
                        let to = byte_offset(&text, start.checked_add(length)?)?;
                        Some(Mention {
                            person,
                            start: from,
                            len: to - from,
                        })
                    })
                    .collect();
                posts.push(RawPost {
                    cell: t.attr("ref").and_then(|name| parse_cell_name(&name)),
                    id: id("id").unwrap_or_default(),
                    parent: id("parentId").filter(|parent| !parent.is_empty()),
                    done: t.attr_bool("done").unwrap_or(false),
                    post: Post::new(
                        // Someone the list of people does not know has no
                        // name, but still said something.
                        &author.unwrap_or_else(|| Person {
                            provider_id: "None".to_string(),
                            ..Person::default()
                        }),
                        t.attr("dT")
                            .and_then(|time| time_from_iso(&time))
                            .unwrap_or(0.0),
                        &text,
                    )
                    .with_mentions(mentions),
                });
            }
            Event::Open(t) if t.name != "ThreadedComments" => r.skip(&t),
            Event::Eof => break,
            _ => {}
        }
    }

    // First comments before replies, each kind in the order of the file.
    posts.sort_by_key(|raw| raw.parent.is_some());
    let mut threads: Vec<((u32, u32), Thread)> = Vec::new();
    let mut by_id: HashMap<String, usize> = HashMap::new();
    let mut by_cell: HashMap<(u32, u32), usize> = HashMap::new();
    for raw in posts {
        let home = raw
            .parent
            .as_ref()
            .and_then(|parent| by_id.get(parent))
            .or_else(|| raw.cell.as_ref().and_then(|cell| by_cell.get(cell)))
            .copied();
        let index = match (home, raw.cell) {
            (Some(index), _) => {
                threads[index].1.replies.push(raw.post);
                index
            }
            (None, Some(cell)) => {
                threads.push((
                    cell,
                    Thread {
                        first: raw.post,
                        replies: Vec::new(),
                        resolved: raw.done,
                    },
                ));
                by_cell.insert(cell, threads.len() - 1);
                threads.len() - 1
            }
            (None, None) => continue,
        };
        if !raw.id.is_empty() {
            by_id.entry(raw.id).or_insert(index);
        }
    }
    threads
}

/// The font of a run (`<rPr>`): the font of the workbook, changed by what
/// the element says. Excel itself always says everything.
fn parse_run_font<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, context: &Context) -> Font {
    let mut font = context.font.clone();
    if tag.empty {
        return font;
    }
    loop {
        match r.next() {
            Event::Open(t) => {
                let on = t.attr_bool("val").unwrap_or(true);
                match t.name {
                    "b" => font.bold = on,
                    "i" => font.italic = on,
                    "strike" => font.strike = on,
                    "u" => {
                        font.underline = match t.attr("val").as_deref() {
                            None | Some("single" | "singleAccounting") => Underline::Single,
                            Some("double" | "doubleAccounting") => Underline::Double,
                            Some(_) => Underline::None,
                        }
                    }
                    "sz" => {
                        if let Some(points) = t.attr_f64("val") {
                            font.set_size_pt(points as f32);
                        }
                    }
                    "rFont" => {
                        if let Some(name) = t.attr("val").filter(|name| !name.is_empty()) {
                            font.name = Arc::from(name.as_ref());
                        }
                    }
                    "color" => {
                        // The tooltip colour is beyond the palette and so
                        // automatic, like the black of plain text.
                        let raw = RawColor::from_tag(&t);
                        // The text of a note does not follow the theme: a
                        // colour of the theme is read as the colour it is.
                        font.color = raw
                            .resolve(context.palette)
                            .map(Color::plain)
                            .filter(|color| !(raw.is_default_text() && *color == Color::BLACK));
                    }
                    _ => {}
                }
                r.skip(&t);
            }
            Event::Close(_) | Event::Eof => break,
            _ => {}
        }
    }
    font
}

/// The runs of a `<text>`: `<r>` elements with a font each, or one bare
/// `<t>`. Phonetic hints are left out.
fn parse_runs<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, context: &Context) -> Vec<NoteRun> {
    let mut runs = Vec::new();
    if tag.empty {
        return runs;
    }
    loop {
        match r.next() {
            Event::Open(t) => match t.name {
                "t" => runs.push(NoteRun {
                    text: decode_cell_text(&r.text(&t)).into_owned(),
                    font: context.font.clone(),
                }),
                "r" if !t.empty => {
                    let mut run = NoteRun {
                        text: String::new(),
                        font: context.font.clone(),
                    };
                    loop {
                        match r.next() {
                            Event::Open(c) => match c.name {
                                "rPr" => run.font = parse_run_font(r, &c, context),
                                "t" => run.text.push_str(&decode_cell_text(&r.text(&c))),
                                _ => r.skip(&c),
                            },
                            Event::Close(_) | Event::Eof => break,
                            _ => {}
                        }
                    }
                    runs.push(run);
                }
                _ => r.skip(&t),
            },
            Event::Close(_) | Event::Eof => break,
            _ => {}
        }
    }
    runs.retain(|run| !run.text.is_empty());
    runs
}

/// A `<comment>`.
struct RawNote {
    cell: (u32, u32),
    author: Option<usize>,
    runs: Vec<NoteRun>,
}

/// The notes of a part and its list of authors.
fn parse_comments(xml: &str, context: &Context) -> (Vec<String>, Vec<RawNote>) {
    let mut authors = Vec::new();
    let mut notes = Vec::new();
    let mut r = Reader::new(xml);
    loop {
        match r.next() {
            Event::Open(t) => match t.name {
                "comments" | "authors" | "commentList" => {}
                "author" => authors.push(decode_cell_text(&r.text(&t)).into_owned()),
                "comment" => {
                    let cell = t.attr("ref").and_then(|name| parse_cell_name(&name));
                    let mut runs = Vec::new();
                    if !t.empty {
                        loop {
                            match r.next() {
                                Event::Open(c) if c.name == "text" => {
                                    runs = parse_runs(&mut r, &c, context);
                                }
                                Event::Open(c) => r.skip(&c),
                                Event::Close(_) | Event::Eof => break,
                                _ => {}
                            }
                        }
                    }
                    if let Some(cell) = cell {
                        notes.push(RawNote {
                            cell,
                            author: t.attr_u32("authorId").map(|id| id as usize),
                            runs,
                        });
                    }
                }
                _ => r.skip(&t),
            },
            Event::Eof => break,
            _ => {}
        }
    }
    (authors, notes)
}

/// What a VML drawing says of the box of a note.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct NoteShape {
    /// See [`Shape::anchor`].
    anchor: Option<[f64; 8]>,
    visible: bool,
}

/// The eight numbers of an `<x:Anchor>`.
fn parse_anchor(text: &str) -> Option<[f64; 8]> {
    let mut anchor = [0.0; 8];
    let mut numbers = text.split(',');
    for slot in &mut anchor {
        *slot = numbers
            .next()?
            .trim()
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite() && *n >= 0.0)?;
    }
    numbers.next().is_none().then_some(anchor)
}

/// Collect the boxes of the notes of a VML drawing, by cell. The drawing
/// also holds the buttons and check boxes of a sheet, and is not always
/// tidy XML: shapes are picked out of it without relying on how elements
/// nest.
fn parse_vml(xml: &str, boxes: &mut HashMap<(u32, u32), NoteShape>) {
    #[derive(Default)]
    struct Open {
        note: bool,
        row: Option<u32>,
        col: Option<u32>,
        shape: NoteShape,
    }
    fn close(open: Option<Open>, boxes: &mut HashMap<(u32, u32), NoteShape>) {
        if let Some(Open {
            note: true,
            row: Some(row),
            col: Some(col),
            shape,
        }) = open
            && row < MAX_ROWS
            && col < MAX_COLS
        {
            boxes.entry((row, col)).or_insert(shape);
        }
    }
    let mut open: Option<Open> = None;
    let mut r = Reader::new(xml);
    loop {
        match r.next() {
            Event::Open(t) if t.name == "shape" => {
                close(open.take(), boxes);
                open = (!t.empty).then(Open::default);
            }
            Event::Open(t) => {
                let Some(open) = &mut open else {
                    continue;
                };
                match t.name {
                    "ClientData" => {
                        open.note = t
                            .attr("ObjectType")
                            .is_some_and(|kind| kind.eq_ignore_ascii_case("Note"));
                    }
                    "Row" => open.row = r.text(&t).trim().parse().ok(),
                    "Column" => open.col = r.text(&t).trim().parse().ok(),
                    "Anchor" => open.shape.anchor = parse_anchor(&r.text(&t)),
                    // Present and empty means shown.
                    "Visible" => {
                        open.shape.visible = !r.text(&t).trim().eq_ignore_ascii_case("false");
                    }
                    _ => {}
                }
            }
            Event::Close("shape") => close(open.take(), boxes),
            Event::Eof => break,
            _ => {}
        }
    }
    close(open, boxes);
}

/// The box an `Anchor` describes, measured on the sheet as the file lays
/// it out: `cell_size` is the width and the height the file gives to the
/// columns and rows without a size of their own.
fn anchored_box(sheet: &Sheet, anchor: [f64; 8], cell_size: (f32, f32)) -> Option<NoteBox> {
    let cols = Track {
        default: cell_size.0,
        sizes: &sheet.col_widths,
        hidden: &sheet.hidden_cols,
        max: f32::INFINITY,
        count: MAX_COLS,
    };
    let rows = Track {
        default: cell_size.1,
        sizes: &sheet.row_heights,
        hidden: &sheet.hidden_rows,
        max: f32::INFINITY,
        count: MAX_ROWS,
    };
    // A corner is held in its cell, and on the sheet.
    let corner = |track: &Track, cell: f64, offset: f64| {
        let cell = (cell as u32).min(track.count - 1);
        (cell, offset.min(track.span(cell, cell + 1)))
    };
    let ((c0, dx0), (r0, dy0)) = (
        corner(&cols, anchor[0], anchor[1]),
        corner(&rows, anchor[2], anchor[3]),
    );
    let ((c1, dx1), (r1, dy1)) = (
        corner(&cols, anchor[4], anchor[5]),
        corner(&rows, anchor[6], anchor[7]),
    );
    let width = cols.span(c0, c1) + dx1 - dx0;
    let height = rows.span(r0, r1) + dy1 - dy0;
    // Corners the wrong way round, or a box of the size of a sheet, are no
    // box: the note then sits next to its cell.
    let sane = |size: f64| (1.0..=100_000.0).contains(&size);
    (sane(width) && sane(height)).then_some(NoteBox {
        anchor: (r0, c0),
        offset: (dx0 as f32, dy0 as f32),
        size: (width as f32, height as f32),
    })
}

fn set_frame(sheet: &mut Sheet, cell: (u32, u32), frame: Option<NoteBox>) {
    if let Some(Comment::Note(note)) = sheet.comments.get_mut(&cell) {
        note.frame = frame;
    }
}

/// Read the notes and the comment threads of a sheet into it. `sheet_part`
/// is the name of its worksheet part, whose relationships lead to them (to
/// every VML drawing too: the one `<legacyDrawing>` names is among them, and
/// the others hold no notes). The sheet must already have its columns and
/// rows, against which boxes are measured (see [`anchored_box`] for
/// `cell_size`). A part that is missing or makes no sense costs what it
/// holds and nothing else.
pub(crate) fn read_sheet<R: Read + Seek>(
    pkg: &mut Package<R>,
    sheet_part: &str,
    sheet: &mut Sheet,
    cell_size: (f32, f32),
    context: &Context,
) {
    let rels = relationships(pkg, sheet_part);
    let mut related = |kind: &str| -> Vec<String> {
        rels.iter()
            .filter(|rel| rel.kind == kind)
            .filter_map(|rel| pkg.read(&rel.target).ok().flatten())
            .collect()
    };
    // A cell has a thread or a note, and the thread comes first: the note
    // found on its cell is the stand-in Excel wrote for it.
    for xml in related("threadedComment") {
        for (cell, thread) in parse_threads(&xml, &context.persons) {
            sheet
                .comments
                .entry(cell)
                .or_insert(Comment::Thread(thread));
        }
    }
    let parts: Vec<_> = related("comments")
        .iter()
        .map(|xml| parse_comments(xml, context))
        .collect();
    if parts.iter().all(|(_, notes)| notes.is_empty()) {
        return;
    }
    let mut boxes = HashMap::new();
    for xml in related("vmlDrawing") {
        parse_vml(&xml, &mut boxes);
    }
    for (authors, notes) in parts {
        for raw in notes {
            let cell = raw.cell;
            if sheet.comments.contains_key(&cell) {
                continue;
            }
            // The stand-in of a thread that is gone (a file passed through
            // an application that only knows notes) is a note like others,
            // but nobody wrote it.
            let author = raw
                .author
                .and_then(|id| authors.get(id))
                .filter(|author| !author.starts_with(THREAD_AUTHOR))
                .cloned()
                .unwrap_or_default();
            let shape = boxes.get(&cell).copied().unwrap_or_default();
            let note = Note {
                author,
                runs: raw.runs,
                visible: shape.visible,
                frame: None,
            };
            sheet.comments.insert(cell, Comment::Note(note));
            let Some(anchor) = shape.anchor else {
                continue;
            };
            // A box found where a note that was never moved is written is
            // no box of its own: the note keeps following its cell.
            let home = self::shape(sheet, cell, false).anchor;
            let at_home = anchor
                .iter()
                .zip(home)
                .enumerate()
                .all(|(i, (found, home))| {
                    let slack = if i % 2 == 0 { 0.0 } else { 1.0 };
                    (found - home as f64).abs() <= slack
                });
            let Some(frame) = anchored_box(sheet, anchor, cell_size).filter(|_| !at_home) else {
                continue;
            };
            set_frame(sheet, cell, Some(frame));
            // The size is the one the box had in the file; its corner is
            // named the way the sheet names that place (an offset running
            // past its cell is the cell after it).
            if let Some(placed) = sheet.note_frame(cell.0, cell.1) {
                let tidy = sheet.note_box(placed);
                set_frame(sheet, cell, Some(tidy));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guids_look_like_excels_and_differ() {
        let id = guid("post", &[b"a", b"bc"]);
        assert_eq!(id.len(), 38);
        let bytes = id.as_bytes();
        assert_eq!((bytes[0], bytes[37]), (b'{', b'}'));
        for (i, b) in bytes[1..37].iter().enumerate() {
            match i {
                8 | 13 | 18 | 23 => assert_eq!(*b, b'-', "{id}"),
                14 => assert_eq!(*b, b'4', "{id}"),
                19 => assert!(b"89AB".contains(b), "{id}"),
                _ => assert!(b.is_ascii_digit() || (b'A'..=b'F').contains(b), "{id}"),
            }
        }
        assert_eq!(id, guid("post", &[b"a", b"bc"]));
        assert_ne!(id, guid("post", &[b"ab", b"c"]));
        assert_ne!(id, guid("person", &[b"a", b"bc"]));

        // No two posts of a workbook share an id.
        let mut seen = std::collections::HashSet::new();
        for sheet in 0..4 {
            for row in 0..40 {
                for col in 0..10 {
                    for post in 0..5 {
                        assert!(seen.insert(post_id(sheet, (row, col), post)));
                    }
                }
            }
        }
        assert_ne!(
            person_id(&Person::new("Ann")),
            person_id(&Person::new("Bob"))
        );
    }

    #[test]
    fn anchors() {
        assert_eq!(
            parse_anchor("\n    1, 15, 0, 2, 3, 31, 4, 1"),
            Some([1.0, 15.0, 0.0, 2.0, 3.0, 31.0, 4.0, 1.0])
        );
        assert_eq!(parse_anchor("1,2,3,4,5,6,7"), None);
        assert_eq!(parse_anchor("1,2,3,4,5,6,7,8,9"), None);
        assert_eq!(parse_anchor("1,2,3,4,5,6,7,x"), None);
        assert_eq!(parse_anchor("1,2,3,4,5,6,7,-8"), None);
        assert_eq!(parse_anchor("1,2,3,4,5,6,7,NaN"), None);
        assert_eq!(parse_anchor(""), None);

        let sheet = Sheet::new("S");
        let frame = anchored_box(
            &sheet,
            [1.0, 15.0, 0.0, 2.0, 3.0, 31.0, 4.0, 1.0],
            (64.0, 20.0),
        );
        assert_eq!(
            frame,
            Some(NoteBox {
                anchor: (0, 1),
                offset: (15.0, 2.0),
                size: (144.0, 79.0)
            })
        );
        // The same anchor on a sheet with wider columns and taller rows.
        let frame = anchored_box(
            &sheet,
            [1.0, 15.0, 0.0, 2.0, 3.0, 31.0, 4.0, 1.0],
            (80.0, 21.0),
        );
        assert_eq!(frame.map(|f| f.size), Some((176.0, 83.0)));
        // Offsets are held in their cells.
        let frame = anchored_box(
            &sheet,
            [1.0, 1e12, 0.0, 30.0, 3.0, 200.0, 4.0, 1.0],
            (64.0, 20.0),
        );
        assert_eq!(
            frame,
            Some(NoteBox {
                anchor: (0, 1),
                offset: (64.0, 20.0),
                size: (128.0, 61.0)
            })
        );
        // Upside down, or absurd.
        for anchor in [
            [3.0, 31.0, 4.0, 1.0, 1.0, 15.0, 0.0, 2.0],
            [1.0, 15.0, 0.0, 2.0, 1.0, 15.0, 0.0, 2.0],
            [0.0, 0.0, 0.0, 0.0, 1e9, 0.0, 1e9, 0.0],
        ] {
            assert_eq!(
                anchored_box(&sheet, anchor, (64.0, 20.0)),
                None,
                "{anchor:?}"
            );
        }
    }

    #[test]
    fn shape_ids_come_in_blocks() {
        assert_eq!(blocks(0), 1);
        assert_eq!(blocks(1), 1);
        assert_eq!(blocks(1023), 1);
        // The 1024th shape of the first block would be number 2048, which
        // belongs to the second.
        assert_eq!(blocks(1024), 2);
        let shape = |row| Shape {
            cell: (row, 0),
            frame: [79.0, 2.0, 144.0, 79.0],
            anchor: [1, 15, 0, 2, 3, 31, 4, 1],
            visible: false,
        };
        let shapes: Vec<Shape> = (0..1025).map(shape).collect();
        let xml = vml_xml(&shapes, 3);
        assert!(xml.contains("<o:idmap v:ext=\"edit\" data=\"3,4\"/>"));
        assert!(
            xml.contains("<v:shape id=\"_x0000_s3073\"") && xml.contains("id=\"_x0000_s4097\"")
        );
        assert!(xml.contains("margin-left:59.25pt;margin-top:1.5pt;width:108pt;height:59.25pt;z-index:1;visibility:hidden\""));
    }

    #[test]
    fn times_a_file_cannot_hold_are_left_out() {
        assert_eq!(
            post_time(45352.5).as_deref(),
            Some("2024-03-01T12:00:00.00")
        );
        for serial in [0.0, -3.0, 60.5, f64::NAN, f64::INFINITY, 2_958_466.0] {
            assert_eq!(post_time(serial), None, "{serial}");
        }
    }
}
