//! Notes and comment threads in `.xlsx` files: what the writer produces,
//! that it comes back unchanged, and what is made of the parts Excel and
//! other applications write.

mod common;

use common::{assert_well_formed, at, enter, package, part, part_names};
use opensheet_engine::ast::Expr;
use opensheet_engine::chart::{
    Chart, ChartKind, ChartTitle, DEFAULT_CHART_SIZE, LegendPos, Series, SeriesName,
};
use opensheet_engine::comment::{Comment, Note, NoteBox, NoteRun, Person, Thread};
use opensheet_engine::drawing::Frame;
use opensheet_engine::parser::parse_formula;
use opensheet_engine::style::{Font, Underline};
use opensheet_engine::{Color, Locale, Workbook};
use opensheet_io::{read_xlsx, write_xlsx};
use std::collections::HashSet;
use std::io::Cursor;
use std::sync::Arc;

const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const REL_2017: &str = "http://schemas.microsoft.com/office/2017/10/relationships";
const PKG_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";
const THREADS: &str = "http://schemas.microsoft.com/office/spreadsheetml/2018/threadedcomments";
const DECL: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n";

fn to_bytes(wb: &Workbook) -> Vec<u8> {
    write_xlsx(wb, Cursor::new(Vec::new()))
        .unwrap()
        .into_inner()
}

fn load(bytes: &[u8]) -> Workbook {
    read_xlsx(Cursor::new(bytes), Locale::en()).expect("the package loads")
}

/// Set `OPENSHEET_COMMENT_SAMPLES` to a directory to keep the packages
/// these tests build, to open them in Excel.
fn keep(name: &str, bytes: &[u8]) {
    if let Some(dir) = std::env::var_os("OPENSHEET_COMMENT_SAMPLES") {
        std::fs::write(std::path::Path::new(&dir).join(name), bytes).unwrap();
    }
}

fn note_at<'a>(wb: &'a Workbook, sheet: usize, cell: &str) -> &'a Note {
    let (row, col) = at(cell);
    match wb.comment_at(sheet, row, col) {
        Some(Comment::Note(note)) => note,
        other => panic!("{cell}: expected a note, found {other:?}"),
    }
}

fn thread_at<'a>(wb: &'a Workbook, sheet: usize, cell: &str) -> &'a Thread {
    let (row, col) = at(cell);
    match wb.comment_at(sheet, row, col) {
        Some(Comment::Thread(thread)) => thread,
        other => panic!("{cell}: expected a thread, found {other:?}"),
    }
}

/// The cells of a sheet that have a note or a thread, by name.
fn cells(wb: &Workbook, sheet: usize) -> Vec<String> {
    wb.sheets[sheet]
        .comments
        .keys()
        .map(|(row, col)| opensheet_engine::refs::cell_name(*row, *col))
        .collect()
}

fn font(change: impl Fn(&mut Font)) -> Font {
    let mut font = Note::font();
    change(&mut font);
    font
}

fn run(text: &str, font: Font) -> NoteRun {
    NoteRun {
        text: text.to_string(),
        font,
    }
}

fn account(name: &str, user_id: &str, provider_id: &str) -> Person {
    Person {
        name: name.to_string(),
        user_id: user_id.to_string(),
        provider_id: provider_id.to_string(),
    }
}

const AWKWARD: &str = "  <b> & \"quotes\" 'single'\n\ttab, _x000D_ spelled out,\r\nCR LF and é€  ";

/// A workbook with notes and threads of every sort, on three of its four
/// sheets.
fn sample(locale: &'static Locale) -> Workbook {
    let mut wb = Workbook::new(locale);
    wb.add_sheet(1);
    wb.add_sheet(2);
    wb.add_sheet(3);
    assert!(wb.rename_sheet(0, "Notes"));
    assert!(wb.rename_sheet(1, "Threads"));
    assert!(wb.rename_sheet(2, "Both & chart"));
    assert!(wb.rename_sheet(3, "Plain"));
    for (cell, text) in [
        ("A1", "Budget"),
        ("C3", "1250"),
        ("E8", "=C3*2"),
        ("G2", "x"),
    ] {
        enter(&mut wb, 0, cell, text);
    }
    // Geometry the boxes have to cope with.
    wb.set_col_width(0, 1, Some(120.0));
    wb.sheets[0].hidden_cols.insert(3);
    wb.set_row_height(0, 4, Some(40.0));

    // A new note, as the application makes it: hidden, next to its cell.
    assert!(wb.set_note(
        0,
        0,
        0,
        "Thomas Tastet",
        "Thomas Tastet:\nCheck this figure."
    ));
    // Runs in several fonts, left showing.
    let (row, col) = at("C3");
    wb.sheets[0].comments.insert(
        (row, col),
        Comment::Note(Note {
            author: "Ann Lee".to_string(),
            runs: vec![
                run("Ann Lee:", font(|f| f.bold = true)),
                run(
                    "\nred italic, ",
                    font(|f| {
                        f.italic = true;
                        f.color = Some(Color(0xFF0000));
                    }),
                ),
                run(
                    "big Arial",
                    font(|f| {
                        f.name = Arc::from("Arial");
                        f.size = 29;
                        f.underline = Underline::Single;
                    }),
                ),
                run(
                    " struck",
                    font(|f| {
                        f.strike = true;
                        f.underline = Underline::Double;
                        f.color = Some(Color(0x00B050));
                    }),
                ),
            ],
            visible: true,
            frame: None,
        }),
    );
    // Moved and resized, hidden.
    assert!(wb.set_note(
        0,
        7,
        4,
        "Thomas Tastet",
        "Thomas Tastet:\nMoved and resized."
    ));
    assert!(wb.set_note_frame(0, 7, 4, Some(Frame::new(500.0, 200.0, 220.0, 120.0))));
    // Moved, shown, across the wide column, the hidden one and the tall row.
    assert!(wb.set_note(0, 11, 1, "Ann Lee", "Ann Lee:\nShown."));
    assert!(wb.set_note_frame(0, 11, 1, Some(Frame::new(70.0, 70.0, 300.0, 60.0))));
    assert!(wb.set_note_visible(0, 11, 1, true));
    // Nobody's, with everything XML needs escaping for.
    assert!(wb.set_note(0, 1, 6, "", AWKWARD));
    // Empty.
    assert!(wb.set_note(0, 19, 0, "Ann Lee", ""));

    let thomas = account(
        "Thomas Tastet",
        "S::thomas@example.com::9d0e7c1a-1b2c-4d3e-8f90-a1b2c3d4e5f6",
        "AD",
    );
    let ann = Person::new("Ann Lee");
    enter(&mut wb, 1, "B2", "42");
    assert!(wb.add_comment(1, 1, 1, &thomas, 45352.375, "Is this number right?"));
    assert!(wb.add_reply(1, 1, 1, &ann, 45352.5, "Yes, checked against the ledger."));
    assert!(wb.add_reply(1, 1, 1, &thomas, 45353.25, "Thanks!"));
    // Resolved.
    assert!(wb.add_comment(1, 4, 3, &ann, 45360.75, "Please update."));
    assert!(wb.add_reply(1, 4, 3, &thomas, 45361.125, "Done."));
    assert!(wb.set_comment_resolved(1, 4, 3, true));
    // Awkward text and an awkward name.
    let ops = account("Zoë <Ops> & \"Co\"", "", "None");
    assert!(wb.add_comment(1, 8, 0, &ops, 45400.0, AWKWARD));
    // A note among threads.
    assert!(wb.set_note(1, 2, 5, "Ann Lee", "Ann Lee:\nA note among threads."));
    assert!(wb.set_note_visible(1, 2, 5, true));

    // A sheet whose worksheet also points at a drawing.
    for (i, value) in ["3", "1", "2"].into_iter().enumerate() {
        wb.set_cell_text(2, i as u32, 0, value).unwrap();
    }
    let values = match parse_formula("'Both & chart'!$A$1:$A$3", Locale::en()) {
        Ok(Expr::Ref(range)) => range,
        other => panic!("not a reference: {other:?}"),
    };
    let chart = Chart {
        id: 0,
        kind: ChartKind::Column,
        title: ChartTitle::Auto,
        legend: LegendPos::None,
        series: vec![Series {
            name: SeriesName::None,
            values,
            categories: None,
            color: None,
        }],
        data_labels: false,
        anchor: (5, 3),
        offset: (0.0, 0.0),
        size: DEFAULT_CHART_SIZE,
    };
    assert!(wb.add_chart(2, chart).is_some());
    assert!(wb.set_note(2, 0, 0, "Thomas Tastet", "Thomas Tastet:\nBeside a chart."));
    assert!(wb.add_comment(2, 1, 0, &ann, 45352.5, "And a thread."));
    wb.commit("comments");
    wb
}

#[test]
fn notes_and_threads_round_trip() {
    let original = sample(Locale::en());
    let bytes = to_bytes(&original);
    keep("opensheet-comments.xlsx", &bytes);
    let loaded = load(&bytes);
    for (index, sheet) in original.sheets.iter().enumerate() {
        assert_eq!(
            sheet.comments, loaded.sheets[index].comments,
            "{}",
            sheet.name
        );
    }
    assert_eq!(cells(&loaded, 0), ["A1", "G2", "C3", "E8", "B12", "A20"]);
    assert_eq!(cells(&loaded, 1), ["B2", "F3", "D5", "A9"]);
    assert_eq!(cells(&loaded, 2), ["A1", "A2"]);
    assert!(loaded.sheets[3].comments.is_empty());
    assert!(!loaded.modified && !loaded.can_undo());

    // What the trip has to carry, spelled out.
    let note = note_at(&loaded, 0, "C3");
    assert_eq!(note.author, "Ann Lee");
    assert_eq!(note.text(), "Ann Lee:\nred italic, big Arial struck");
    assert!(note.visible && note.frame.is_none());
    assert!(note.runs[0].font.bold && note.runs[0].font.color.is_none());
    assert_eq!(&*note.runs[0].font.name, "Tahoma");
    assert_eq!(note.runs[2].font.size_pt(), 14.5);
    assert_eq!(note.runs[3].font.color, Some(Color(0x00B050)));
    let note = note_at(&loaded, 0, "E8");
    assert!(!note.visible);
    assert_eq!(
        loaded.sheets[0].note_frame(7, 4),
        Some(Frame::new(500.0, 200.0, 220.0, 120.0))
    );
    assert_eq!(note_at(&loaded, 0, "G2").text(), AWKWARD);
    assert!(note_at(&loaded, 0, "A20").runs.is_empty());
    let thread = thread_at(&loaded, 1, "B2");
    assert_eq!(thread.replies.len(), 2);
    assert_eq!(thread.first.author.provider_id, "AD");
    assert_eq!(thread.replies[0].author, Person::new("Ann Lee"));
    assert_eq!(thread.replies[1].time, 45353.25);
    assert!(!thread.resolved && thread_at(&loaded, 1, "D5").resolved);
    assert_eq!(thread_at(&loaded, 1, "A9").first.text, AWKWARD);
    assert_eq!(loaded.sheets[2].charts.len(), 1);

    // A second trip changes nothing, down to the bytes of every part.
    let again = to_bytes(&loaded);
    assert_eq!(part_names(&bytes), part_names(&again));
    for name in part_names(&bytes) {
        assert_eq!(part(&bytes, &name), part(&again, &name), "{name}");
    }
}

/// The targets of the relationships of a part, resolved to part names,
/// with the last word of their types.
fn targets(bytes: &[u8], rels: &str, base: &str) -> Vec<(String, String)> {
    part(bytes, rels)
        .split("<Relationship ")
        .skip(1)
        .map(|rel| {
            let attr = |name: &str| {
                let start = rel.find(&format!("{name}=\"")).unwrap() + name.len() + 2;
                rel[start..].split('"').next().unwrap().to_string()
            };
            let target = attr("Target");
            let target = match target.strip_prefix("../") {
                Some(up) => format!("xl/{up}"),
                None => format!("{base}{target}"),
            };
            (attr("Type").rsplit('/').next().unwrap().to_string(), target)
        })
        .collect()
}

#[test]
fn package_lists_the_comment_parts() {
    let bytes = to_bytes(&sample(Locale::en()));
    let names = part_names(&bytes);
    let expected = [
        "xl/comments1.xml",
        "xl/comments2.xml",
        "xl/comments3.xml",
        "xl/drawings/vmlDrawing1.vml",
        "xl/drawings/vmlDrawing2.vml",
        "xl/drawings/vmlDrawing3.vml",
        "xl/threadedComments/threadedComment1.xml",
        "xl/threadedComments/threadedComment2.xml",
        "xl/persons/person.xml",
        "xl/worksheets/_rels/sheet1.xml.rels",
        "xl/worksheets/_rels/sheet2.xml.rels",
        "xl/worksheets/_rels/sheet3.xml.rels",
    ];
    for name in expected {
        assert!(names.iter().any(|n| n == name), "missing {name}");
    }
    assert!(
        !names.iter().any(|n| n.contains("comments4")
            || n.contains("vmlDrawing4")
            || n.contains("threadedComment3")
            || n.contains("sheet4.xml.rels")),
        "a sheet without notes has none of their parts: {names:?}"
    );
    assert_eq!(names.iter().collect::<HashSet<_>>().len(), names.len());
    for name in &names {
        let text = part(&bytes, name);
        if name.ends_with(".vml") {
            // VML drawings carry no XML declaration.
            assert!(text.starts_with("<xml xmlns:v="), "{name}");
            assert_well_formed(name, &format!("{DECL}{text}"));
        } else {
            assert_well_formed(name, &text);
        }
    }

    // Every part has a content type, of the right kind.
    let types = part(&bytes, "[Content_Types].xml");
    assert_eq!(
        types.matches("<Default Extension=\"vml\" ContentType=\"application/vnd.openxmlformats-officedocument.vmlDrawing\"/>").count(),
        1
    );
    assert!(
        types.find("<Default Extension=\"vml\"").unwrap() < types.find("<Override").unwrap(),
        "defaults come before the parts named one by one"
    );
    for name in names
        .iter()
        .filter(|n| n.ends_with(".xml") && !n.starts_with('['))
    {
        let kind = if name.starts_with("xl/comments") {
            "application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml"
        } else if name.starts_with("xl/threadedComments/") {
            "application/vnd.ms-excel.threadedcomments+xml"
        } else if name.starts_with("xl/persons/") {
            "application/vnd.ms-excel.person+xml"
        } else {
            ""
        };
        assert!(
            types.contains(&format!(
                "<Override PartName=\"/{name}\" ContentType=\"{kind}"
            )),
            "{name} is not declared as {kind:?}"
        );
    }

    // Sheet → parts, by relationships whose targets exist, under the ids
    // the worksheet uses.
    let kinds = |sheet: usize| -> Vec<(String, String)> {
        targets(
            &bytes,
            &format!("xl/worksheets/_rels/sheet{sheet}.xml.rels"),
            "xl/worksheets/",
        )
    };
    let pair = |kind: &str, target: &str| (kind.to_string(), target.to_string());
    assert_eq!(
        kinds(1),
        [
            pair("vmlDrawing", "xl/drawings/vmlDrawing1.vml"),
            pair("comments", "xl/comments1.xml"),
        ]
    );
    assert_eq!(
        kinds(2),
        [
            pair("vmlDrawing", "xl/drawings/vmlDrawing2.vml"),
            pair("comments", "xl/comments2.xml"),
            pair(
                "threadedComment",
                "xl/threadedComments/threadedComment1.xml"
            ),
        ]
    );
    assert_eq!(
        kinds(3),
        [
            pair("drawing", "xl/drawings/drawing1.xml"),
            pair("vmlDrawing", "xl/drawings/vmlDrawing3.vml"),
            pair("comments", "xl/comments3.xml"),
            pair(
                "threadedComment",
                "xl/threadedComments/threadedComment2.xml"
            ),
        ]
    );
    for sheet in 1..=3 {
        let rels = part(
            &bytes,
            &format!("xl/worksheets/_rels/sheet{sheet}.xml.rels"),
        );
        assert!(rels.starts_with(&format!(
            "{DECL}<Relationships xmlns=\"{PKG_REL}\"><Relationship Id=\"rId1\" "
        )));
        for (n, (_, target)) in kinds(sheet).iter().enumerate() {
            assert!(names.contains(target), "dangling relationship to {target}");
            assert!(rels.contains(&format!("<Relationship Id=\"rId{}\" ", n + 1)));
        }
        assert_eq!(rels.matches("<Relationship ").count(), kinds(sheet).len());
    }
    let rels = part(&bytes, "xl/worksheets/_rels/sheet2.xml.rels");
    assert!(rels.contains(&format!("Type=\"{REL}/vmlDrawing\"")));
    assert!(rels.contains(&format!("Type=\"{REL}/comments\"")));
    assert!(rels.contains(&format!("Type=\"{REL_2017}/threadedComment\"")));

    // The worksheet names the VML drawing, after the drawing of charts.
    let sheet = |n: usize| part(&bytes, &format!("xl/worksheets/sheet{n}.xml"));
    assert!(sheet(1).ends_with("/><legacyDrawing r:id=\"rId1\"/></worksheet>"));
    assert!(sheet(2).ends_with("/><legacyDrawing r:id=\"rId1\"/></worksheet>"));
    assert!(
        sheet(3).ends_with("/><drawing r:id=\"rId1\"/><legacyDrawing r:id=\"rId2\"/></worksheet>")
    );
    assert!(!sheet(4).contains("egacyDrawing") && !sheet(4).contains("<drawing"));

    // The workbook names the people.
    let workbook_rels = targets(&bytes, "xl/_rels/workbook.xml.rels", "xl/");
    assert_eq!(
        workbook_rels.last(),
        Some(&pair("person", "xl/persons/person.xml"))
    );
    let rels = part(&bytes, "xl/_rels/workbook.xml.rels");
    assert!(rels.contains(&format!(
        "Type=\"{REL_2017}/person\" Target=\"persons/person.xml\"/>"
    )));
    let ids: Vec<&str> = rels
        .split(" Id=\"")
        .skip(1)
        .map(|rel| rel.split('"').next().unwrap())
        .collect();
    assert_eq!(ids.len(), 4 + 2 + 1, "sheets, styles, strings, people");
    assert_eq!(
        ids.iter().collect::<HashSet<_>>().len(),
        ids.len(),
        "{ids:?}"
    );

    // Without threads there are no people and nothing of 2017.
    let mut plain = Workbook::new(Locale::en());
    assert!(plain.set_note(0, 0, 0, "A", "A:\nnote"));
    let bytes = to_bytes(&plain);
    let names = part_names(&bytes);
    assert!(names.contains(&"xl/comments1.xml".to_string()));
    assert!(
        !names
            .iter()
            .any(|n| n.contains("person") || n.contains("threaded"))
    );
    assert!(!part(&bytes, "xl/_rels/workbook.xml.rels").contains("person"));
    assert!(!part(&bytes, "xl/comments1.xml").contains("xr:"));

    // Without notes or threads the package is what it always was.
    let bytes = to_bytes(&Workbook::new(Locale::en()));
    let names = part_names(&bytes);
    assert!(
        !names
            .iter()
            .any(|n| n.contains("comments") || n.contains("vml") || n.contains("_rels/sheet"))
    );
    assert!(!part(&bytes, "[Content_Types].xml").contains("vml"));
}

/// The value of an attribute of the first `element` that holds `marker`,
/// in its attributes or in what it contains.
fn attr_of(xml: &str, element: &str, marker: &str, name: &str) -> String {
    let found = xml
        .split(&format!("<{element} "))
        .skip(1)
        .find(|rest| rest.contains(marker))
        .unwrap_or_else(|| panic!("no <{element}> with {marker}"));
    let tag = format!(" {}", found.split('>').next().unwrap());
    let start = tag
        .find(&format!(" {name}=\""))
        .unwrap_or_else(|| panic!("<{element}{tag}> has no {name}"))
        + name.len()
        + 3;
    tag[start..].split('"').next().unwrap().to_string()
}

fn is_guid(id: &str) -> bool {
    let bytes = id.as_bytes();
    bytes.len() == 38
        && bytes[0] == b'{'
        && bytes[37] == b'}'
        && bytes[1..37].iter().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => *b == b'-',
            _ => b.is_ascii_digit() || (b'A'..=b'F').contains(b),
        })
}

#[test]
fn parts_say_what_the_model_says() {
    let bytes = to_bytes(&sample(Locale::en()));

    // ---- notes ----
    let comments = part(&bytes, "xl/comments1.xml");
    assert!(comments.starts_with(&format!(
        "{DECL}<comments xmlns=\"{MAIN}\"><authors><author>Thomas Tastet</author><author></author><author>Ann Lee</author></authors><commentList><comment ref=\"A1\" authorId=\"0\" shapeId=\"0\"><text>"
    )), "{comments}");
    // A new note: the author in bold, the rest plain, both Tahoma 9 in the
    // automatic colour of notes.
    assert!(comments.contains(concat!(
        "<comment ref=\"A1\" authorId=\"0\" shapeId=\"0\"><text>",
        "<r><rPr><b/><sz val=\"9\"/><color indexed=\"81\"/><rFont val=\"Tahoma\"/></rPr><t>Thomas Tastet:</t></r>",
        "<r><rPr><sz val=\"9\"/><color indexed=\"81\"/><rFont val=\"Tahoma\"/></rPr><t xml:space=\"preserve\">\nCheck this figure.</t></r>",
        "</text></comment>"
    )), "{comments}");
    assert!(comments.contains(concat!(
        "<r><rPr><i/><sz val=\"9\"/><color rgb=\"FFFF0000\"/><rFont val=\"Tahoma\"/></rPr><t xml:space=\"preserve\">\nred italic, </t></r>",
        "<r><rPr><u/><sz val=\"14.5\"/><color indexed=\"81\"/><rFont val=\"Arial\"/></rPr><t>big Arial</t></r>",
        "<r><rPr><strike/><u val=\"double\"/><sz val=\"9\"/><color rgb=\"FF00B050\"/><rFont val=\"Tahoma\"/></rPr><t xml:space=\"preserve\"> struck</t></r>"
    )), "{comments}");
    assert!(comments.contains(
        "<comment ref=\"G2\" authorId=\"1\" shapeId=\"0\"><text><r><rPr><sz val=\"9\"/><color indexed=\"81\"/><rFont val=\"Tahoma\"/></rPr><t xml:space=\"preserve\">  &lt;b&gt; &amp; \"quotes\" 'single'\n\ttab, _x005F_x000D_ spelled out,_x000D_\nCR LF and é€  </t></r></text></comment>"
    ), "{comments}");
    assert!(
        comments.contains(
            "<comment ref=\"A20\" authorId=\"2\" shapeId=\"0\"><text><t/></text></comment>"
        )
    );
    assert!(comments.ends_with("</text></comment></commentList></comments>"));
    assert_eq!(comments.matches("<comment ").count(), 6);

    // ---- their boxes ----
    let vml = part(&bytes, "xl/drawings/vmlDrawing1.vml");
    assert!(vml.starts_with(concat!(
        "<xml xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:o=\"urn:schemas-microsoft-com:office:office\" xmlns:x=\"urn:schemas-microsoft-com:office:excel\">",
        "<o:shapelayout v:ext=\"edit\"><o:idmap v:ext=\"edit\" data=\"1\"/></o:shapelayout>",
        "<v:shapetype id=\"_x0000_t202\" coordsize=\"21600,21600\" o:spt=\"202\" path=\"m,l,21600r21600,l21600,xe\"><v:stroke joinstyle=\"miter\"/><v:path gradientshapeok=\"t\" o:connecttype=\"rect\"/></v:shapetype>"
    )), "{vml}");
    // A1, never moved: the anchor Excel gives it, 108 × 59.25 points.
    assert!(vml.contains(concat!(
        "<v:shape id=\"_x0000_s1025\" type=\"#_x0000_t202\" style=\"position:absolute;margin-left:59.25pt;margin-top:1.5pt;width:108pt;height:59.25pt;z-index:1;visibility:hidden\" fillcolor=\"infoBackground [80]\" strokecolor=\"none [81]\" o:insetmode=\"auto\">",
        "<v:fill color2=\"infoBackground [80]\"/><v:shadow color=\"none [81]\" obscured=\"t\"/><v:path o:connecttype=\"none\"/>",
        "<v:textbox style=\"mso-direction-alt:auto\"><div style=\"text-align:left\"></div></v:textbox>",
        "<x:ClientData ObjectType=\"Note\"><x:MoveWithCells/><x:SizeWithCells/><x:Anchor>1, 15, 0, 2, 2, 39, 4, 1</x:Anchor><x:AutoFill>False</x:AutoFill><x:Row>0</x:Row><x:Column>0</x:Column></x:ClientData></v:shape>"
    )), "{vml}");
    assert_eq!(vml.matches("<v:shape ").count(), 6);
    assert_eq!(vml.matches("ObjectType=\"Note\"").count(), 6);
    // Shown notes say so twice: no `visibility:hidden`, and `<x:Visible/>`.
    assert_eq!(vml.matches("visibility:hidden").count(), 4);
    assert_eq!(vml.matches("<x:Visible/>").count(), 2);
    let shown = vml
        .split("<v:shape ")
        .find(|shape| shape.contains("<x:Row>2</x:Row><x:Column>2</x:Column>"))
        .unwrap();
    assert!(
        shown.contains("z-index:3\" fillcolor")
            && shown.ends_with("<x:Visible/></x:ClientData></v:shape>"),
        "{shown}"
    );
    // E8, moved by hand: 500 px from the left is 60 px into column H
    // (A 64, B 120, C 64, D hidden, E to G 64 each).
    assert!(vml.contains(
        "margin-left:375pt;margin-top:150pt;width:165pt;height:90pt;z-index:4;visibility:hidden"
    ));
    assert!(vml.contains("<x:Anchor>7, 60, 9, 0, 11, 24, 15, 0</x:Anchor><x:AutoFill>False</x:AutoFill><x:Row>7</x:Row><x:Column>4</x:Column>"));
    // B12: from 70 px (6 into B) across B, C and the hidden D to 370 px,
    // 58 into F; from 70 px down (row 4 starts at 60) to 130, 10 into row
    // 6 (row 5 is 40 px tall).
    assert!(
        vml.contains("<x:Anchor>1, 6, 3, 10, 5, 58, 5, 10</x:Anchor>"),
        "{vml}"
    );
    // Ids are unique in the workbook: each sheet has a block of its own.
    for (n, first) in [(1, 1025), (2, 2049), (3, 3073)] {
        let vml = part(&bytes, &format!("xl/drawings/vmlDrawing{n}.vml"));
        assert!(vml.contains(&format!("<o:idmap v:ext=\"edit\" data=\"{n}\"/>")));
        assert!(vml.contains(&format!("<v:shape id=\"_x0000_s{first}\" ")));
    }

    // ---- threads ----
    let threads = part(&bytes, "xl/threadedComments/threadedComment1.xml");
    assert!(threads.starts_with(&format!(
        "{DECL}<ThreadedComments xmlns=\"{THREADS}\" xmlns:x=\"{MAIN}\"><threadedComment ref=\"B2\" dT=\"2024-03-01T09:00:00.00\" personId=\"{{"
    )), "{threads}");
    assert_eq!(threads.matches("<threadedComment ").count(), 6);
    let persons = part(&bytes, "xl/persons/person.xml");
    assert!(persons.starts_with(&format!(
        "{DECL}<personList xmlns=\"{THREADS}\" xmlns:x=\"{MAIN}\"><person displayName=\"Thomas Tastet\" id=\"{{"
    )), "{persons}");
    assert_eq!(
        persons.matches("<person ").count(),
        3,
        "each person once: {persons}"
    );
    let thomas = attr_of(&persons, "person", "Thomas Tastet", "id");
    let ann = attr_of(&persons, "person", "Ann Lee", "id");
    let ops = attr_of(&persons, "person", "Zoë", "id");
    assert!(is_guid(&thomas) && is_guid(&ann) && is_guid(&ops));
    assert_eq!(
        [&thomas, &ann, &ops]
            .into_iter()
            .collect::<HashSet<_>>()
            .len(),
        3
    );
    assert!(persons.contains(&format!(
        "<person displayName=\"Thomas Tastet\" id=\"{thomas}\" userId=\"S::thomas@example.com::9d0e7c1a-1b2c-4d3e-8f90-a1b2c3d4e5f6\" providerId=\"AD\"/>"
    )));
    assert!(persons.contains(&format!(
        "<person displayName=\"Ann Lee\" id=\"{ann}\" userId=\"Ann Lee\" providerId=\"None\"/>"
    )));
    assert!(persons.contains(&format!(
        "<person displayName=\"Zoë &lt;Ops&gt; &amp; &quot;Co&quot;\" id=\"{ops}\" userId=\"\" providerId=\"None\"/>"
    )));

    // A thread: its first comment, then the replies naming it as parent.
    let root = attr_of(&threads, "threadedComment", "Is this number right?", "id");
    let reply = attr_of(&threads, "threadedComment", "Yes, checked", "id");
    let thanks = attr_of(&threads, "threadedComment", "Thanks!", "id");
    assert!(threads.contains(&format!(
        "<threadedComment ref=\"B2\" dT=\"2024-03-01T09:00:00.00\" personId=\"{thomas}\" id=\"{root}\"><text>Is this number right?</text></threadedComment><threadedComment ref=\"B2\" dT=\"2024-03-01T12:00:00.00\" personId=\"{ann}\" id=\"{reply}\" parentId=\"{root}\"><text>Yes, checked against the ledger.</text></threadedComment><threadedComment ref=\"B2\" dT=\"2024-03-02T06:00:00.00\" personId=\"{thomas}\" id=\"{thanks}\" parentId=\"{root}\"><text>Thanks!</text></threadedComment>"
    )), "{threads}");
    // Resolved: `done` on the first comment only.
    let resolved = attr_of(&threads, "threadedComment", "Please update.", "id");
    assert!(threads.contains(&format!(
        "personId=\"{ann}\" id=\"{resolved}\" done=\"1\"><text>Please update.</text>"
    )));
    assert_eq!(threads.matches(" done=\"1\"").count(), 1);
    assert!(threads.contains("<text>  &lt;b&gt; &amp; \"quotes\" 'single'\n\ttab, _x005F_x000D_ spelled out,_x000D_\nCR LF and é€  </text>"));
    // Every post has an id of its own, in Excel's form.
    let mut ids = HashSet::new();
    for element in threads.split("<threadedComment ").skip(1) {
        let id = element
            .split(" id=\"")
            .nth(1)
            .unwrap()
            .split('"')
            .next()
            .unwrap();
        assert!(is_guid(id), "{id}");
        assert!(ids.insert(id.to_string()), "{id} twice");
    }
    let elsewhere = part(&bytes, "xl/threadedComments/threadedComment2.xml");
    let other = attr_of(&elsewhere, "threadedComment", "And a thread.", "id");
    assert!(is_guid(&other) && !ids.contains(&other));
    assert!(
        elsewhere.contains(&format!("personId=\"{ann}\"")),
        "one id per person in the workbook"
    );

    // Beside each thread, the note older versions show in its place: its
    // author is the id of the thread.
    let comments = part(&bytes, "xl/comments2.xml");
    assert!(comments.starts_with(&format!(
        "{DECL}<comments xmlns=\"{MAIN}\" xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" mc:Ignorable=\"xr\" xmlns:xr=\"http://schemas.microsoft.com/office/spreadsheetml/2014/revision\"><authors><author>tc={root}</author><author>Ann Lee</author><author>tc={resolved}</author><author>tc={{"
    )), "{comments}");
    assert!(comments.contains(&format!(
        "<comment ref=\"B2\" authorId=\"0\" shapeId=\"0\" xr:uid=\"{root}\"><text><t xml:space=\"preserve\">[Threaded comment]\n\nYour version of Excel allows you to read this threaded comment; however, any edits to it will get removed if the file is opened in a newer version of Excel. Learn more: https://go.microsoft.com/fwlink/?linkid=870924\n\nComment:\n    Is this number right?\nReply:\n    Yes, checked against the ledger.\nReply:\n    Thanks!</t></text></comment>"
    )), "{comments}");
    assert!(comments.contains("<comment ref=\"F3\" authorId=\"1\" shapeId=\"0\"><text><r>"));
    // Each has a box like a note's, hidden.
    let vml = part(&bytes, "xl/drawings/vmlDrawing2.vml");
    assert_eq!(vml.matches("<v:shape ").count(), 4);
    assert_eq!(vml.matches("visibility:hidden").count(), 3);
    assert!(vml.contains("<x:Anchor>2, 15, 0, 10, 4, 31, 4, 9</x:Anchor><x:AutoFill>False</x:AutoFill><x:Row>1</x:Row><x:Column>1</x:Column></x:ClientData>"), "{vml}");
}

#[test]
fn the_stand_in_of_a_thread_speaks_the_language_of_the_workbook() {
    let bytes = to_bytes(&sample(Locale::fr()));
    let comments = part(&bytes, "xl/comments2.xml");
    assert!(comments.contains("[Commentaire à thread]\n\nVotre version d’Excel vous permet de lire ce commentaire à thread."), "{comments}");
    assert!(comments.contains("Commentaire\u{a0}:\n    Is this number right?\nRéponse\u{a0}:\n    Yes, checked against the ledger."));
    let loaded = read_xlsx(Cursor::new(&bytes), Locale::fr()).unwrap();
    let original = sample(Locale::fr());
    for (index, sheet) in original.sheets.iter().enumerate() {
        assert_eq!(
            sheet.comments, loaded.sheets[index].comments,
            "{}",
            sheet.name
        );
    }
}

// ---- element order ---------------------------------------------------------------

/// The children an element may have, in the order of the schemas
/// (ECMA-376 `sml.xsd` for the worksheet and the notes, Microsoft's
/// `threadedcomments` schema for the rest). Elements not listed hold no
/// elements, or any (the properties of a run).
const CONTENT: &[(&str, &[&str])] = &[
    (
        "worksheet",
        &[
            "sheetPr",
            "dimension",
            "sheetViews",
            "sheetFormatPr",
            "cols",
            "sheetData",
            "sheetCalcPr",
            "sheetProtection",
            "protectedRanges",
            "scenarios",
            "autoFilter",
            "sortState",
            "dataConsolidate",
            "customSheetViews",
            "mergeCells",
            "phoneticPr",
            "conditionalFormatting",
            "dataValidations",
            "hyperlinks",
            "printOptions",
            "pageMargins",
            "pageSetup",
            "headerFooter",
            "rowBreaks",
            "colBreaks",
            "customProperties",
            "cellWatches",
            "ignoredErrors",
            "smartTags",
            "drawing",
            "legacyDrawing",
            "legacyDrawingHF",
            "drawingHF",
            "picture",
            "oleObjects",
            "controls",
            "webPublishItems",
            "tableParts",
            "extLst",
        ],
    ),
    ("comments", &["authors", "commentList", "extLst"]),
    ("authors", &["author"]),
    ("commentList", &["comment"]),
    ("comment", &["text", "commentPr"]),
    ("text", &["t", "r", "rPh", "phoneticPr"]),
    ("r", &["rPr", "t"]),
    ("ThreadedComments", &["threadedComment", "extLst"]),
    ("threadedComment", &["text", "mentions", "extLst"]),
    ("personList", &["person", "extLst"]),
];

/// Elements that may follow one another.
const REPEATABLE: &[&str] = &[
    "author",
    "comment",
    "r",
    "threadedComment",
    "person",
    "conditionalFormatting",
];

/// Check the order of the children of the elements listed in [`CONTENT`].
fn assert_schema_order(part_name: &str, xml: &str) {
    use quick_xml::events::Event;
    // (element, position of its last child in its content model)
    let mut open: Vec<(String, Option<usize>)> = Vec::new();
    let mut reader = quick_xml::Reader::from_str(xml);
    loop {
        let (name, empty) = match reader.read_event().unwrap() {
            Event::Start(e) => (
                String::from_utf8(e.name().as_ref().to_vec()).unwrap(),
                false,
            ),
            Event::Empty(e) => (String::from_utf8(e.name().as_ref().to_vec()).unwrap(), true),
            Event::End(_) => {
                open.pop();
                continue;
            }
            Event::Eof => break,
            _ => continue,
        };
        if let Some((parent, last)) = open.last_mut()
            && let Some((_, allowed)) = CONTENT.iter().find(|(element, _)| element == parent)
        {
            let at = allowed
                .iter()
                .position(|child| *child == name)
                .unwrap_or_else(|| panic!("{part_name}: <{name}> is not allowed in <{parent}>"));
            assert!(
                last.is_none_or(
                    |last| at > last || (at == last && REPEATABLE.contains(&name.as_str()))
                ),
                "{part_name}: <{name}> is out of order in <{parent}>"
            );
            *last = Some(at);
        }
        if !empty {
            open.push((name, None));
        }
    }
}

#[test]
fn elements_come_in_schema_order() {
    let bytes = to_bytes(&sample(Locale::en()));
    let mut checked = 0;
    for name in part_names(&bytes) {
        if name.starts_with("xl/worksheets/sheet")
            || name.starts_with("xl/comments")
            || name.starts_with("xl/threadedComments/")
            || name.starts_with("xl/persons/")
        {
            assert_schema_order(&name, &part(&bytes, &name));
            checked += 1;
        }
    }
    assert_eq!(checked, 4 + 3 + 2 + 1);

    // The check itself notices what it is there for.
    let sheet = part(&bytes, "xl/worksheets/sheet3.xml");
    let comments = part(&bytes, "xl/comments1.xml");
    for (xml, good, bad) in [
        (
            &sheet,
            "<drawing r:id=\"rId1\"/><legacyDrawing r:id=\"rId2\"/>",
            "<legacyDrawing r:id=\"rId2\"/><drawing r:id=\"rId1\"/>",
        ),
        (
            &sheet,
            "<legacyDrawing r:id=\"rId2\"/>",
            "<legacyDrawing r:id=\"rId2\"/><pageMargins/>",
        ),
        (&comments, "<commentList>", "<commentList><author/>"),
        (&comments, "</authors>", "</authors><authors/>"),
    ] {
        assert!(xml.contains(good));
        let broken = xml.replacen(good, bad, 1);
        assert!(
            std::panic::catch_unwind(|| assert_schema_order("broken", &broken)).is_err(),
            "{bad} should be refused"
        );
    }
}

#[test]
fn boxes_follow_the_cells() {
    // A note that was never moved is written where the application shows
    // it: 15 px right of its cell and 10 px above its top, whatever the
    // sizes of the columns and rows around.
    let mut wb = Workbook::new(Locale::en());
    wb.set_col_width(0, 3, Some(200.0));
    wb.set_row_height(0, 5, Some(50.0));
    wb.sheets[0].hidden_rows.insert(7);
    assert!(wb.set_note(0, 6, 2, "A", "A:\nnext to D"));
    assert!(wb.set_note(0, 0, 16_383, "A", "A:\nlast column"));
    assert!(wb.set_note(0, 1_048_575, 0, "A", "A:\nlast row"));
    let bytes = to_bytes(&wb);
    let vml = part(&bytes, "xl/drawings/vmlDrawing1.vml");
    // C7: left edge of D + 15; row 6 starts at 5 × 20 + 50 = 150, so the
    // top is at 140, 40 px into the tall row 5; 79 px further down is 219,
    // past row 6 (150 to 170), the hidden row 7 and rows 8 and 9, 9 px
    // into row 10.
    assert!(vml.contains("<x:Anchor>3, 15, 5, 40, 3, 159, 10, 9</x:Anchor><x:AutoFill>False</x:AutoFill><x:Row>6</x:Row><x:Column>2</x:Column>"), "{vml}");
    // Nothing lies beyond the last column or the last row.
    assert!(
        vml.contains("<x:Anchor>16383, 15, 0, 2, 16383, 64, 4, 1</x:Anchor>"),
        "{vml}"
    );
    assert!(vml.contains("<x:Row>1048575</x:Row><x:Column>0</x:Column>"));
    let loaded = load(&bytes);
    for cell in [(6, 2), (0, 16_383), (1_048_575, 0)] {
        let note = loaded.sheets[0].comments[&cell].as_note().unwrap();
        assert_eq!(note.frame, None, "{cell:?}");
    }
    assert_eq!(to_bytes(&loaded), bytes);

    // A box that was placed by hand comes back where it was, whole pixels.
    let mut wb = Workbook::new(Locale::en());
    assert!(wb.set_note(0, 2, 2, "A", "A:\nmoved"));
    assert!(wb.set_note_frame(0, 2, 2, Some(Frame::new(10.4, 300.6, 99.5, 41.2))));
    let loaded = load(&to_bytes(&wb));
    assert_eq!(
        note_at(&loaded, 0, "C3").frame,
        Some(NoteBox {
            anchor: (15, 0),
            offset: (10.0, 1.0),
            size: (100.0, 41.0)
        })
    );
    assert_eq!(
        loaded.sheets[0].note_frame(2, 2),
        Some(Frame::new(10.0, 301.0, 100.0, 41.0))
    );
}

#[test]
fn many_notes_on_many_sheets() {
    // More notes than one block of shape ids holds: the next sheet starts
    // after the blocks of this one.
    let mut wb = Workbook::new(Locale::en());
    wb.add_sheet(1);
    for i in 0..1100u32 {
        let (row, col) = (i / 10 * 6, i % 10 * 4);
        assert!(wb.set_note(0, row, col, "Ann", &format!("Ann:\nnote {i}")));
    }
    assert!(wb.set_note(1, 0, 0, "Bob", "Bob:\non the next sheet"));
    assert!(wb.add_comment(1, 3, 3, &Person::new("Bob"), 45352.5, "a thread"));
    let bytes = to_bytes(&wb);
    keep("opensheet-many-notes.xlsx", &bytes);
    let first = part(&bytes, "xl/drawings/vmlDrawing1.vml");
    assert!(first.contains("<o:idmap v:ext=\"edit\" data=\"1,2\"/>"));
    assert_eq!(first.matches("<v:shape ").count(), 1100);
    assert!(first.contains("id=\"_x0000_s1025\"") && first.contains("id=\"_x0000_s2124\""));
    assert!(first.contains("z-index:1100;visibility:hidden"));
    let second = part(&bytes, "xl/drawings/vmlDrawing2.vml");
    assert!(second.contains("<o:idmap v:ext=\"edit\" data=\"3\"/>"));
    assert!(second.contains("id=\"_x0000_s3073\"") && second.contains("id=\"_x0000_s3074\""));
    // No shape id is used twice.
    let mut ids = HashSet::new();
    for vml in [&first, &second] {
        for shape in vml.split("<v:shape id=\"").skip(1) {
            assert!(ids.insert(shape.split('"').next().unwrap().to_string()));
        }
    }
    assert_eq!(ids.len(), 1102);

    let loaded = load(&bytes);
    assert_eq!(loaded.sheets[0].comments, wb.sheets[0].comments);
    assert_eq!(loaded.sheets[1].comments, wb.sheets[1].comments);
    assert_eq!(note_at(&loaded, 0, "AK655").text(), "Ann:\nnote 1099");
}

// ---- files from other applications -----------------------------------------------

fn relationships(list: &[(&str, &str, &str)]) -> String {
    let mut x = format!("{DECL}<Relationships xmlns=\"{PKG_REL}\">");
    for (id, kind, target) in list {
        let kind = if kind.contains("://") {
            kind.to_string()
        } else {
            format!("{REL}/{kind}")
        };
        x.push_str(&format!(
            "<Relationship Id=\"{id}\" Type=\"{kind}\" Target=\"{target}\"/>"
        ));
    }
    x.push_str("</Relationships>");
    x
}

const THOMAS_ID: &str = "{C6A1E3F4-0B2D-4A5C-8E7F-123456789ABC}";
const ANN_ID: &str = "{0A0B0C0D-1E1F-4A2B-9C3D-4E5F60718293}";
const THREAD_B4: &str = "{8B1A6F0E-2C7D-4E53-9A41-0D5B7C3E9F12}";
const THREAD_D7: &str = "{11111111-2222-4333-8444-555555555555}";
const STAND_IN: &str = "[Threaded comment]\n\nYour version of Excel allows you to read this threaded comment; however, any edits to it will get removed if the file is opened in a newer version of Excel. Learn more: https://go.microsoft.com/fwlink/?linkid=870924\n\nComment:\n    ";

/// The notes of a sheet as Excel 365 writes them: two authors, runs in
/// several fonts, the stand-ins of two threads.
fn excel_comments() -> String {
    let plain =
        r#"<rPr><sz val="9"/><color indexed="81"/><rFont val="Tahoma"/><family val="2"/></rPr>"#;
    let bold = r#"<rPr><b/><sz val="9"/><color indexed="81"/><rFont val="Tahoma"/><family val="2"/></rPr>"#;
    format!(
        r#"{DECL}<comments xmlns="{MAIN}" xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" mc:Ignorable="xr" xmlns:xr="http://schemas.microsoft.com/office/spreadsheetml/2014/revision"><authors><author>Thomas Tastet</author><author>Ann Lee</author><author>tc={THREAD_B4}</author><author>tc={THREAD_D7}</author></authors><commentList><comment ref="A1" authorId="0" shapeId="0" xr:uid="{{00000000-0006-0000-0000-000001000000}}"><text><r>{bold}<t>Thomas Tastet:</t></r><r>{plain}<t xml:space="preserve">
A plain note</t></r></text></comment><comment ref="C2" authorId="1" shapeId="0" xr:uid="{{00000000-0006-0000-0000-000002000000}}"><text><r>{bold}<t>Ann Lee:</t></r><r><rPr><i/><u/><sz val="12"/><color rgb="FFFF0000"/><rFont val="Arial"/><family val="2"/></rPr><t xml:space="preserve">
red italic </t></r><r><rPr><b val="0"/><strike/><u val="double"/><vertAlign val="superscript"/><sz val="9"/><color theme="4" tint="-0.249977111117893"/><rFont val="Tahoma"/><charset val="1"/></rPr><t>struck_x000D_</t></r><r><rPr><sz val="9"/><color theme="1"/><rFont val="Tahoma"/></rPr><t xml:space="preserve"> black</t></r><r><t xml:space="preserve"> bare</t></r><rPh sb="0" eb="1"><t>phonetic</t></rPh><phoneticPr fontId="1"/></text></comment><comment ref="B4" authorId="2" shapeId="0" xr:uid="{THREAD_B4}"><text><t>{STAND_IN}Is this right?
Reply:
    Yes.
Reply:
    @Ann Lee thanks</t></text></comment><comment ref="D7" authorId="3" shapeId="0" xr:uid="{THREAD_D7}"><text><t>{STAND_IN}Resolved one</t></text></comment><comment ref="F9" authorId="1" shapeId="0" xr:uid="{{00000000-0006-0000-0000-000005000000}}"><text><r>{bold}<t>Ann Lee:</t></r><r>{plain}<t xml:space="preserve">
Shown, moved and resized</t></r></text></comment><comment ref="A12" authorId="0" shapeId="0"><text><r>{plain}<t>Hidden and moved</t></r></text></comment></commentList></comments>"#
    )
}

/// One shape of a VML drawing, laid out as Excel does.
fn excel_shape(id: u32, style: &str, anchor: &str, row: u32, col: u32, visible: bool) -> String {
    let visible = if visible { "\n   <x:Visible/>" } else { "" };
    format!(
        r##"<v:shape id="_x0000_s{id}" type="#_x0000_t202" style='position:absolute;
  {style}'
  fillcolor="infoBackground [80]" strokecolor="none [81]" o:insetmode="auto">
  <v:fill color2="infoBackground [80]"/>
  <v:shadow color="none [81]" obscured="t"/>
  <v:path o:connecttype="none"/>
  <v:textbox style='mso-direction-alt:auto'>
   <div style='text-align:left'></div>
  </v:textbox>
  <x:ClientData ObjectType="Note">
   <x:MoveWithCells/>
   <x:SizeWithCells/>
   <x:Anchor>
    {anchor}</x:Anchor>
   <x:AutoFill>False</x:AutoFill>
   <x:Row>{row}</x:Row>
   <x:Column>{col}</x:Column>{visible}
  </x:ClientData>
 </v:shape>"##
    )
}

/// The VML drawing of the same sheet: a box per note, one of them shown,
/// two moved, and a button, which is no note. The `style` of a shape and
/// its `Anchor` say the same thing in a file from Excel; here those of F9
/// disagree, and Excel goes by the anchor.
fn excel_vml() -> String {
    let hidden = "margin-left:59.25pt;margin-top:1.5pt;width:108pt;height:59.25pt;z-index:1;\n  visibility:hidden";
    let mut x = String::from(
        r##"<xml xmlns:v="urn:schemas-microsoft-com:vml"
 xmlns:o="urn:schemas-microsoft-com:office:office"
 xmlns:x="urn:schemas-microsoft-com:office:excel">
 <o:shapelayout v:ext="edit">
  <o:idmap v:ext="edit" data="1"/>
 </o:shapelayout><v:shapetype id="_x0000_t201" coordsize="21600,21600" o:spt="201"
  path="m,l,21600r21600,l21600,xe">
  <v:stroke joinstyle="miter"/>
  <v:path shadowok="f" o:extrusionok="f" strokeok="f" fillok="f" o:connecttype="rect"/>
  <o:lock v:ext="edit" shapetype="t"/>
 </v:shapetype><v:shape id="_x0000_s1031" type="#_x0000_t201" style='position:absolute;
  margin-left:48pt;margin-top:150pt;width:96pt;height:30pt;z-index:7;
  mso-wrap-style:tight' o:button="t" fillcolor="buttonFace [67]"
  strokecolor="windowText [64]" o:insetmode="auto">
  <v:fill color2="buttonFace [67]" o:detectmouseclick="t"/>
  <o:lock v:ext="edit" rotation="t"/>
  <v:textbox style='mso-direction-alt:auto' o:singleclick="f">
   <div style='text-align:center'><font face="Calibri" size="220" color="#000000">Button 1<br></font></div>
  </v:textbox>
  <x:ClientData ObjectType="Button">
   <x:Anchor>
    0, 0, 0, 0, 2, 0, 2, 0</x:Anchor>
   <x:PrintObject>False</x:PrintObject>
   <x:AutoFill>False</x:AutoFill>
   <x:FmlaMacro>[0]!Button1_Click</x:FmlaMacro>
   <x:TextHAlign>Center</x:TextHAlign>
   <x:TextVAlign>Center</x:TextVAlign>
   <x:Row>0</x:Row>
   <x:Column>0</x:Column>
   <x:Visible/>
  </x:ClientData>
 </v:shape><v:shapetype id="_x0000_t202" coordsize="21600,21600" o:spt="202"
  path="m,l,21600r21600,l21600,xe">
  <v:stroke joinstyle="miter"/>
  <v:path gradientshapeok="t" o:connecttype="rect"/>
 </v:shapetype>"##,
    );
    // Column B is 120 px wide and row 6 is 40 px tall in this sheet.
    x.push_str(&excel_shape(
        1025,
        hidden,
        "1, 15, 0, 2, 2, 39, 4, 1",
        0,
        0,
        false,
    ));
    x.push_str(&excel_shape(
        1026,
        hidden,
        "3, 15, 0, 10, 5, 31, 4, 9",
        1,
        2,
        false,
    ));
    x.push_str(&excel_shape(
        1027,
        hidden,
        "2, 15, 2, 10, 4, 31, 5, 29",
        3,
        1,
        false,
    ));
    x.push_str(&excel_shape(
        1028,
        hidden,
        "4, 15, 5, 30, 6, 31, 9, 9",
        6,
        3,
        false,
    ));
    x.push_str(&excel_shape(
        1029,
        "margin-left:330pt;margin-top:63.75pt;width:159pt;height:125.25pt;z-index:5",
        "6, 20, 3, 5, 9, 40, 10, 12",
        8,
        5,
        true,
    ));
    x.push_str(&excel_shape(
        1030,
        "margin-left:12pt;margin-top:255pt;width:60pt;height:30pt;z-index:6;\n  visibility:hidden",
        "0, 16, 16, 0, 1, 32, 18, 0",
        11,
        0,
        false,
    ));
    x.push_str("</xml>");
    x
}

fn excel_threads() -> String {
    format!(
        r#"{DECL}<ThreadedComments xmlns="{THREADS}" xmlns:x="{MAIN}"><threadedComment ref="B4" dT="2024-03-01T09:30:15.25" personId="{THOMAS_ID}" id="{THREAD_B4}"><text>Is this right?</text></threadedComment><threadedComment ref="B4" dT="2024-03-01T10:00:00.00" personId="{ANN_ID}" id="{{AAAAAAAA-0000-4000-8000-000000000001}}" parentId="{THREAD_B4}"><text>Yes.</text></threadedComment><threadedComment ref="B4" dT="2024-03-02T18:45:00.50" personId="{THOMAS_ID}" id="{{AAAAAAAA-0000-4000-8000-000000000002}}" parentId="{THREAD_B4}"><text>@Ann Lee thanks</text><mentions><mention mentionpersonId="{ANN_ID}" mentionId="{{BBBBBBBB-0000-4000-8000-000000000001}}" startIndex="0" length="8"/></mentions></threadedComment><threadedComment ref="D7" dT="2024-03-05T08:00:00.00" personId="{ANN_ID}" id="{THREAD_D7}" done="1"><text>Resolved one</text><extLst><ext uri="x"><text>not the text</text></ext></extLst></threadedComment></ThreadedComments>"#
    )
}

fn excel_persons() -> String {
    format!(
        r#"{DECL}<personList xmlns="{THREADS}" xmlns:x="{MAIN}"><person displayName="Thomas Tastet" id="{THOMAS_ID}" userId="S::thomas@example.com::9d0e7c1a-1b2c-4d3e-8f90-a1b2c3d4e5f6" providerId="AD"/><person displayName="Ann Lee" id="{ANN_ID}" userId="1234567890abcdef" providerId="Windows Live"/><person displayName="Nobody &amp; Co" id="{{DDDDDDDD-0000-4000-8000-000000000001}}" userId="nobody" providerId="None"/></personList>"#
    )
}

/// Notes from an application that writes neither boxes nor fonts, in a
/// part with prefixed elements. Excel opens such a sheet and shows none of
/// its notes.
const BARE_COMMENTS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<x:comments xmlns:x="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <x:authors>
    <x:author>Script</x:author>
  </x:authors>
  <x:commentList>
    <x:comment ref="B2" authorId="0">
      <x:text><x:t>Just text</x:t></x:text>
    </x:comment>
    <x:comment ref="$C$5" authorId="0">
      <x:text><x:r><x:t>Two </x:t></x:r><x:r><x:rPr><x:b/></x:rPr><x:t>runs</x:t></x:r></x:text>
    </x:comment>
    <x:comment ref="D9">
      <x:text/>
    </x:comment>
  </x:commentList>
</x:comments>"#;

/// The font of the fixture workbook, which text without a font of its own
/// is shown in.
fn workbook_font() -> Font {
    Font {
        name: Arc::from("Calibri"),
        ..Font::default()
    }
}

/// A workbook laid out like those of Excel 365: a sheet with notes and
/// threads, a sheet with the notes of a script. Written by hand; Excel for
/// Mac 16 opens it without complaint and shows the notes of the first sheet
/// where the tests below expect them. `replace` swaps parts (or removes
/// them, with `None`).
fn excel_package(replace: &[(&str, Option<&str>)]) -> Vec<u8> {
    let styles = format!(
        r#"{DECL}<styleSheet xmlns="{MAIN}"><fonts count="1"><font><sz val="11"/><color theme="1"/><name val="Calibri"/><family val="2"/><scheme val="minor"/></font></fonts><fills count="2"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill></fills><borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs></styleSheet>"#
    );
    let content_types = format!(
        r#"{DECL}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="vml" ContentType="application/vnd.openxmlformats-officedocument.vmlDrawing"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/worksheets/sheet2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/comments1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml"/><Override PartName="/xl/comments2.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.comments+xml"/><Override PartName="/xl/threadedComments/threadedComment1.xml" ContentType="application/vnd.ms-excel.threadedcomments+xml"/><Override PartName="/xl/persons/person.xml" ContentType="application/vnd.ms-excel.person+xml"/></Types>"#
    );
    let workbook = format!(
        r#"{DECL}<workbook xmlns="{MAIN}" xmlns:r="{REL}"><sheets><sheet name="Review" sheetId="1" r:id="rId1"/><sheet name="Scripted" sheetId="2" r:id="rId2"/></sheets></workbook>"#
    );
    let sheet1 = format!(
        r#"{DECL}<worksheet xmlns="{MAIN}" xmlns:r="{REL}"><dimension ref="A1:F12"/><sheetViews><sheetView tabSelected="1" workbookViewId="0"/></sheetViews><sheetFormatPr defaultRowHeight="15"/><cols><col min="2" max="2" width="17.140625" customWidth="1"/></cols><sheetData><row r="1"><c r="A1"><v>1</v></c></row><row r="4"><c r="B4"><v>2</v></c></row><row r="6" ht="30" customHeight="1"><c r="A6"><v>3</v></c></row></sheetData><pageMargins left="0.7" right="0.7" top="0.75" bottom="0.75" header="0.3" footer="0.3"/><legacyDrawing r:id="rId1"/></worksheet>"#
    );
    let sheet2 = format!(
        r#"{DECL}<worksheet xmlns="{MAIN}"><sheetData><row r="2"><c r="B2"><v>1</v></c></row></sheetData></worksheet>"#
    );
    let mut parts: Vec<(String, String)> = [
        ("[Content_Types].xml", content_types),
        (
            "_rels/.rels",
            relationships(&[("rId1", "officeDocument", "xl/workbook.xml")]),
        ),
        ("xl/workbook.xml", workbook),
        (
            "xl/_rels/workbook.xml.rels",
            relationships(&[
                ("rId1", "worksheet", "worksheets/sheet1.xml"),
                ("rId2", "worksheet", "worksheets/sheet2.xml"),
                (
                    "rId3",
                    "http://schemas.microsoft.com/office/2017/10/relationships/person",
                    "persons/person.xml",
                ),
                ("rId4", "styles", "styles.xml"),
            ]),
        ),
        ("xl/styles.xml", styles),
        ("xl/worksheets/sheet1.xml", sheet1),
        ("xl/worksheets/sheet2.xml", sheet2),
        (
            "xl/worksheets/_rels/sheet1.xml.rels",
            relationships(&[
                (
                    "rId3",
                    "http://schemas.microsoft.com/office/2017/10/relationships/threadedComment",
                    "../threadedComments/threadedComment1.xml",
                ),
                ("rId2", "comments", "../comments1.xml"),
                ("rId1", "vmlDrawing", "../drawings/vmlDrawing1.vml"),
            ]),
        ),
        (
            "xl/worksheets/_rels/sheet2.xml.rels",
            relationships(&[("rId1", "comments", "/xl/comments2.xml")]),
        ),
        ("xl/comments1.xml", excel_comments()),
        ("xl/comments2.xml", BARE_COMMENTS.to_string()),
        ("xl/drawings/vmlDrawing1.vml", excel_vml()),
        ("xl/threadedComments/threadedComment1.xml", excel_threads()),
        ("xl/persons/person.xml", excel_persons()),
    ]
    .into_iter()
    .map(|(name, text)| (name.to_string(), text))
    .collect();
    for (name, text) in replace {
        parts.retain(|(n, _)| n != name);
        if let Some(text) = text {
            parts.push((name.to_string(), text.to_string()));
        }
    }
    let parts: Vec<(&str, &str)> = parts
        .iter()
        .map(|(name, text)| (name.as_str(), text.as_str()))
        .collect();
    package(&parts)
}

#[test]
fn notes_and_threads_from_excel() {
    let bytes = excel_package(&[]);
    keep("excel-style-comments.xlsx", &bytes);
    let wb = load(&bytes);
    assert_eq!(cells(&wb, 0), ["A1", "C2", "B4", "D7", "F9", "A12"]);

    // A1: a note as Excel makes it, never moved.
    let note = note_at(&wb, 0, "A1");
    assert_eq!(note.author, "Thomas Tastet");
    assert_eq!(note.text(), "Thomas Tastet:\nA plain note");
    assert_eq!(note.runs.len(), 2);
    assert_eq!(note.runs[0].font, font(|f| f.bold = true));
    assert_eq!(note.runs[1].font, Note::font());
    assert!(!note.visible);
    assert_eq!(
        note.frame, None,
        "where Excel puts a new note is no place of its own"
    );
    assert_eq!(
        *note,
        Note::new("Thomas Tastet", "Thomas Tastet:\nA plain note")
    );

    // C2: another author, runs in several fonts; phonetic hints are not
    // text.
    let note = note_at(&wb, 0, "C2");
    assert_eq!(note.author, "Ann Lee");
    assert_eq!(note.text(), "Ann Lee:\nred italic struck\r black bare");
    let fonts: Vec<&Font> = note.runs.iter().map(|run| &run.font).collect();
    assert_eq!(*fonts[0], font(|f| f.bold = true));
    assert_eq!(
        *fonts[1],
        font(|f| {
            f.italic = true;
            f.underline = Underline::Single;
            f.size = 24;
            f.color = Some(Color(0xFF0000));
            f.name = Arc::from("Arial");
        })
    );
    assert!(!fonts[2].bold && fonts[2].strike);
    assert_eq!(fonts[2].underline, Underline::Double);
    assert_eq!(
        fonts[2].color,
        Some(Color(0x305496)),
        "accent 1, darker 25 %"
    );
    assert_eq!(
        *fonts[3],
        Note::font(),
        "the black of plain text is automatic"
    );
    // Excel shows text without properties in the font of the workbook.
    assert_eq!(*fonts[4], workbook_font());
    assert_eq!(note.frame, None);

    // F9: shown, moved and resized. From 20 px into column G to 40 px
    // into column J, from 5 px into row 4 to 12 px into row 11, with row 6
    // 40 px tall.
    let note = note_at(&wb, 0, "F9");
    assert!(note.visible);
    assert_eq!(
        note.frame,
        Some(NoteBox {
            anchor: (3, 6),
            offset: (20.0, 5.0),
            size: (212.0, 167.0)
        })
    );
    assert_eq!(
        wb.sheets[0].note_frame(8, 5),
        Some(Frame::new(460.0, 65.0, 212.0, 167.0))
    );
    // A12: hidden, moved inside column A.
    let note = note_at(&wb, 0, "A12");
    assert!(!note.visible);
    assert_eq!(note.text(), "Hidden and moved");
    assert_eq!(
        note.frame,
        Some(NoteBox {
            anchor: (16, 0),
            offset: (16.0, 0.0),
            size: (80.0, 40.0)
        })
    );

    // B4: a thread with two replies; the note beside it is not a note.
    let thread = thread_at(&wb, 0, "B4");
    let thomas = account(
        "Thomas Tastet",
        "S::thomas@example.com::9d0e7c1a-1b2c-4d3e-8f90-a1b2c3d4e5f6",
        "AD",
    );
    let ann = account("Ann Lee", "1234567890abcdef", "Windows Live");
    assert_eq!(thread.first.author, thomas);
    assert_eq!(thread.first.text, "Is this right?");
    assert!(
        (thread.first.time - (45352.0 + (9.0 * 3600.0 + 30.0 * 60.0 + 15.25) / 86400.0)).abs()
            < 1e-9
    );
    assert_eq!(thread.replies.len(), 2);
    assert_eq!(thread.replies[0].author, ann);
    assert_eq!(thread.replies[0].text, "Yes.");
    assert_eq!(thread.replies[0].time, 45352.0 + 10.0 / 24.0);
    assert_eq!(thread.replies[1].author, thomas);
    assert_eq!(
        thread.replies[1].text, "@Ann Lee thanks",
        "a mention stays as text"
    );
    assert!(!thread.resolved);
    // D7: resolved.
    let thread = thread_at(&wb, 0, "D7");
    assert!(thread.resolved && thread.replies.is_empty());
    assert_eq!(thread.first.text, "Resolved one");
    assert_eq!(thread.first.author, ann);
    assert_eq!(thread.first.time, 45356.0 + 8.0 / 24.0);

    // The other sheet: notes without boxes or fonts.
    assert_eq!(cells(&wb, 1), ["B2", "C5", "D9"]);
    assert_eq!(
        *note_at(&wb, 1, "B2"),
        Note {
            author: "Script".to_string(),
            runs: vec![run("Just text", workbook_font())],
            visible: false,
            frame: None,
        }
    );
    let note = note_at(&wb, 1, "C5");
    assert_eq!(note.text(), "Two runs");
    assert_eq!(note.runs[0].font, workbook_font());
    assert_eq!(
        note.runs[1].font,
        Font {
            bold: true,
            ..workbook_font()
        },
        "what a run does not say is as in the workbook"
    );
    let note = note_at(&wb, 1, "D9");
    assert!(note.author.is_empty() && note.runs.is_empty());

    // Written back, the file says the same.
    let again = load(&to_bytes(&wb));
    for (index, sheet) in wb.sheets.iter().enumerate() {
        assert_eq!(
            sheet.comments, again.sheets[index].comments,
            "{}",
            sheet.name
        );
    }
    let bytes = to_bytes(&wb);
    keep("excel-style-comments-saved.xlsx", &bytes);
    let comments = part(&bytes, "xl/comments1.xml");
    assert!(comments.contains("<comment ref=\"B4\" authorId=\"2\" shapeId=\"0\" xr:uid=\"{"));
    assert!(comments.contains(
        "Comment:\n    Is this right?\nReply:\n    Yes.\nReply:\n    @Ann Lee thanks</t>"
    ));
    let vml = part(&bytes, "xl/drawings/vmlDrawing1.vml");
    assert!(!vml.contains("Button"), "controls are not kept");
    assert!(vml.contains("<x:Anchor>6, 20, 3, 5, 9, 40, 10, 12</x:Anchor><x:AutoFill>False</x:AutoFill><x:Row>8</x:Row><x:Column>5</x:Column><x:Visible/>"), "{vml}");
    assert!(vml.contains("<x:Anchor>1, 15, 0, 2, 2, 39, 4, 1</x:Anchor>"));
    let persons = part(&bytes, "xl/persons/person.xml");
    assert_eq!(persons.matches("<person ").count(), 2, "{persons}");
    assert!(persons.contains("userId=\"1234567890abcdef\" providerId=\"Windows Live\"/>"));
}

#[test]
fn boxes_are_measured_on_the_sheet_of_the_file() {
    // Excel for Mac: wider columns and taller rows by default, which the
    // engine does not have. The box keeps the size it had there.
    let sheet = format!(
        r#"{DECL}<worksheet xmlns="{MAIN}" xmlns:r="{REL}"><sheetFormatPr baseColWidth="10" defaultRowHeight="16"/><sheetData><row r="1"><c r="A1"><v>1</v></c></row></sheetData><legacyDrawing r:id="rId1"/></worksheet>"#
    );
    let comments = format!(
        r#"{DECL}<comments xmlns="{MAIN}"><authors><author>A</author></authors><commentList><comment ref="C3" authorId="0" shapeId="0"><text><t>x</t></text></comment></commentList></comments>"#
    );
    let vml = format!(
        "<xml xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:o=\"urn:schemas-microsoft-com:office:office\" xmlns:x=\"urn:schemas-microsoft-com:office:excel\">{}</xml>",
        excel_shape(
            1025,
            "visibility:hidden",
            "5, 10, 4, 3, 7, 30, 8, 6",
            2,
            2,
            false
        )
    );
    let bytes = excel_package(&[
        ("xl/worksheets/sheet1.xml", Some(&sheet)),
        ("xl/comments1.xml", Some(&comments)),
        ("xl/drawings/vmlDrawing1.vml", Some(&vml)),
    ]);
    let wb = load(&bytes);
    let frame = note_at(&wb, 0, "C3").frame.expect("a box of its own");
    assert_eq!((frame.anchor, frame.offset), ((4, 5), (10.0, 3.0)));
    // Two columns of 80 px and 20 more; four rows of 16 points and 3 more.
    assert_eq!(frame.size.0, 180.0);
    assert!(
        (frame.size.1 - (4.0 * 16.0 * 96.0 / 72.0 + 3.0)).abs() < 0.01,
        "{frame:?}"
    );
}

// ---- damage ----------------------------------------------------------------------

#[test]
fn a_missing_part_costs_only_what_it_held() {
    // No boxes: the notes are all there, hidden, next to their cells.
    let wb = load(&excel_package(&[("xl/drawings/vmlDrawing1.vml", None)]));
    assert_eq!(cells(&wb, 0), ["A1", "C2", "B4", "D7", "F9", "A12"]);
    for cell in ["A1", "C2", "F9", "A12"] {
        let note = note_at(&wb, 0, cell);
        assert!(!note.visible && note.frame.is_none(), "{cell}");
    }
    assert_eq!(thread_at(&wb, 0, "B4").replies.len(), 2);

    // No notes: the threads are there.
    let wb = load(&excel_package(&[("xl/comments1.xml", None)]));
    assert_eq!(cells(&wb, 0), ["B4", "D7"]);
    assert!(thread_at(&wb, 0, "D7").resolved);
    assert_eq!(cells(&wb, 1), ["B2", "C5", "D9"]);

    // No threads (the file went through an application that knows only
    // notes): what stood in for them is all there is, as notes of nobody.
    let wb = load(&excel_package(&[(
        "xl/threadedComments/threadedComment1.xml",
        None,
    )]));
    assert_eq!(cells(&wb, 0), ["A1", "C2", "B4", "D7", "F9", "A12"]);
    let note = note_at(&wb, 0, "B4");
    assert_eq!(note.author, "");
    assert!(
        note.text().starts_with("[Threaded comment]")
            && note.text().ends_with("Reply:\n    @Ann Lee thanks")
    );
    assert_eq!(note.runs.len(), 1);
    assert_eq!(note.runs[0].font, workbook_font());
    assert_eq!(note_at(&wb, 0, "A1").author, "Thomas Tastet");
    // Such a note is written back as a note, not as the stand-in of a
    // thread that does not exist.
    let bytes = to_bytes(&wb);
    assert!(!part(&bytes, "xl/comments1.xml").contains("tc="));
    assert!(
        !part_names(&bytes)
            .iter()
            .any(|n| n.contains("threaded") || n.contains("person"))
    );

    // No people: the posts are there, by nobody.
    let wb = load(&excel_package(&[("xl/persons/person.xml", None)]));
    let thread = thread_at(&wb, 0, "B4");
    assert_eq!(thread.replies[1].text, "@Ann Lee thanks");
    assert_eq!(thread.first.author, account("", "", "None"));
    let bytes = to_bytes(&wb);
    assert_eq!(
        part(&bytes, "xl/persons/person.xml")
            .matches("<person ")
            .count(),
        1
    );
    assert_eq!(thread_at(&load(&bytes), 0, "B4"), thread);

    // No styles: text without a font is in the font of new workbooks.
    let wb = load(&excel_package(&[("xl/styles.xml", None)]));
    assert_eq!(
        note_at(&wb, 1, "B2").runs,
        [run("Just text", Font::default())]
    );
    assert_eq!(note_at(&wb, 0, "A1").runs[1].font, Note::font());

    // No relationships: nothing, and nothing else is lost.
    let wb = load(&excel_package(&[
        ("xl/worksheets/_rels/sheet1.xml.rels", None),
        ("xl/_rels/workbook.xml.rels", None),
    ]));
    assert!(wb.sheets[0].comments.is_empty());
    assert_eq!(wb.sheets.len(), 2);
    assert_eq!(
        common::value(&wb, 0, "B4"),
        opensheet_engine::Value::Number(2.0)
    );
}

#[test]
fn odd_parts_are_read_as_far_as_they_make_sense() {
    // Notes: cells that are no cells, authors that are not listed, a cell
    // named twice, text that is not there, text that is no text.
    let comments = format!(
        r#"{DECL}<comments xmlns="{MAIN}"><commentList><comment ref="ZZZZ1" authorId="0"><text><t>off the sheet</t></text></comment><comment ref="A0" authorId="0"><text><t>no row</t></text></comment><comment authorId="0"><text><t>nowhere</t></text></comment><comment ref="B2" authorId="7"><text><t>whose?</t></text></comment><comment ref="B2" authorId="0"><text><t>again</t></text></comment><comment ref="C3" authorId="-1"/><comment ref="D4" authorId="1"><text><r><rPr><sz val="abc"/><rFont val=""/><color rgb="nope"/><u val="none"/></rPr><t>odd font</t></r><r/><r><rPr/></r></text></comment><comment ref="E5" authorId="0"><text><t>kept</t></text><mc:AlternateContent xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006"><mc:Choice Requires="v2"><commentPr autoFill="0"><anchor><text><t>not a note</t></text></anchor></commentPr></mc:Choice><mc:Fallback/></mc:AlternateContent></comment><comment ref="XFD1048576" authorId="0"><text><t>the last cell</t></text></comment></commentList><authors><author>Late</author><author/></authors></comments>"#
    );
    // Boxes: anchors that are no anchors, cells that are no cells, shapes
    // that never end.
    let vml = format!(
        "<xml xmlns:v=\"urn:schemas-microsoft-com:vml\" xmlns:x=\"urn:schemas-microsoft-com:office:excel\">{}{}{}{}<v:shape><x:ClientData ObjectType=\"Note\"><x:Row>9999999</x:Row><x:Column>0</x:Column><x:Visible/></x:ClientData></v:shape><v:shape><x:ClientData ObjectType=\"Note\"><x:Row>1048575</x:Row><x:Column>16383</x:Column><x:Visible>False</x:Visible><x:Anchor>1,2,3",
        excel_shape(1, "", "9, 0, 9, 0, 2, 0, 2, 0", 1, 1, true),
        excel_shape(2, "", "1, 2, 3", 2, 2, true),
        excel_shape(
            3,
            "",
            "99999999, 0, 99999999, 0, 99999999, 50, 99999999, 50",
            3,
            3,
            false
        ),
        excel_shape(4, "", "0, 0, 0, 0, 5, 0, 5, 0", 1, 1, false),
    );
    let wb = load(&excel_package(&[
        ("xl/comments1.xml", Some(&comments)),
        ("xl/drawings/vmlDrawing1.vml", Some(&vml)),
        ("xl/threadedComments/threadedComment1.xml", None),
    ]));
    assert_eq!(cells(&wb, 0), ["B2", "C3", "D4", "E5", "XFD1048576"]);
    let note = note_at(&wb, 0, "B2");
    assert_eq!((note.author.as_str(), note.text().as_str()), ("", "whose?"));
    // Its first box has its corners the wrong way round: no box, but shown.
    assert!(note.visible && note.frame.is_none());
    let note = note_at(&wb, 0, "C3");
    assert!(note.runs.is_empty() && note.visible && note.frame.is_none());
    let note = note_at(&wb, 0, "D4");
    assert_eq!(note.author, "", "an author without a name");
    assert_eq!(note.runs, [run("odd font", workbook_font())]);
    // Anchored beyond the sheet: held in its last cell.
    assert_eq!(
        note.frame,
        Some(NoteBox {
            anchor: (1_048_575, 16_383),
            offset: (0.0, 0.0),
            size: (50.0, 20.0)
        })
    );
    // What later versions add to a comment is not its text.
    assert_eq!(note_at(&wb, 0, "E5").text(), "kept");
    let note = note_at(&wb, 0, "XFD1048576");
    assert_eq!(note.author, "Late", "authors listed after the notes");
    assert!(!note.visible && note.frame.is_none());
    // Whatever was read can be written and read again.
    let again = load(&to_bytes(&wb));
    assert_eq!(cells(&again, 0), cells(&wb, 0));
    assert_eq!(note_at(&again, 0, "D4").text(), "odd font");

    // Threads: replies before, without or to the wrong comment; two first
    // comments on a cell; cells that are no cells; times that are none.
    let threads = format!(
        r#"{DECL}<ThreadedComments xmlns="{THREADS}"><threadedComment ref="A1" personId="{ANN_ID}" id="{{R1}}" parentId="{{ROOT}}"><text>early reply</text></threadedComment><threadedComment ref="A1" dT="yesterday" personId="{{unknown}}" id="{{ROOT}}"><text>first</text></threadedComment><threadedComment ref="A1" dT="2024-03-01T10:00:00" personId="{THOMAS_ID}" id="{{r2}}" parentId="{{r1}}"><text>reply to a reply</text></threadedComment><threadedComment ref="A1" id="{{ROOT2}}" done="1"><text>second first</text></threadedComment><threadedComment ref="B5" parentId="{{GONE}}" id="{{R3}}"><text>orphan</text></threadedComment><threadedComment ref="B5" parentId="{{GONE}}"><text>orphan's sibling</text></threadedComment><threadedComment ref="nowhere" parentId="{{ROOT}}" id="{{R4}}"/><threadedComment ref="nowhere" id="{{R5}}"><text>lost</text></threadedComment><threadedComment parentId="{{NONE}}"><text>lost too</text></threadedComment><threadedComment ref="C9" dT="1899-12-31T00:00:00.00" id="{{OLD}}" done="true"><text>before time</text></threadedComment></ThreadedComments>"#
    );
    let wb = load(&excel_package(&[(
        "xl/threadedComments/threadedComment1.xml",
        Some(&threads),
    )]));
    let thread = thread_at(&wb, 0, "A1");
    assert_eq!(thread.first.text, "first");
    assert_eq!(thread.first.author, account("", "", "None"));
    assert_eq!(thread.first.time, 0.0);
    assert!(!thread.resolved);
    let replies: Vec<&str> = thread
        .replies
        .iter()
        .map(|post| post.text.as_str())
        .collect();
    assert_eq!(
        replies,
        ["second first", "early reply", "reply to a reply", ""]
    );
    assert_eq!(thread.replies[1].author.name, "Ann Lee");
    assert_eq!(thread.replies[2].time, 45352.0 + 10.0 / 24.0);
    let thread = thread_at(&wb, 0, "B5");
    assert_eq!(thread.first.text, "orphan");
    assert_eq!(thread.replies.len(), 1);
    assert!(thread_at(&wb, 0, "C9").resolved);
    // The notes of the cells with threads gave way, the others are there.
    assert_eq!(
        cells(&wb, 0),
        ["A1", "C2", "B4", "B5", "D7", "C9", "F9", "A12"]
    );
    assert_eq!(
        note_at(&wb, 0, "B4").author,
        "",
        "a stand-in whose thread is gone"
    );
    // A time no file can hold is left out, and nothing is lost by that.
    let bytes = to_bytes(&wb);
    let written = part(&bytes, "xl/threadedComments/threadedComment1.xml");
    assert!(
        written.contains("<threadedComment ref=\"A1\" personId=\"{"),
        "{written}"
    );
    assert!(
        written.contains("<threadedComment ref=\"A1\" dT=\"2024-03-01T10:00:00.00\" personId=\"{")
    );
    let again = load(&bytes);
    assert_eq!(again.sheets[0].comments, wb.sheets[0].comments);
}

#[test]
fn damaged_comment_parts_never_panic() {
    // Cut each part of notes and threads short, at many places in a
    // package of ours and at every character in one laid out like Excel's.
    let ours = to_bytes(&sample(Locale::en()));
    let names = part_names(&ours);
    let parts: Vec<(String, String)> = names.iter().map(|n| (n.clone(), part(&ours, n))).collect();
    let mut attempts = 0;
    for (victim, text) in parts.iter().filter(|(name, _)| {
        name.contains("comments2")
            || name.contains("vmlDrawing2")
            || name.contains("threadedComment1")
            || name.contains("person")
            || name.contains("sheet2.xml.rels")
    }) {
        for cut in (0..text.len())
            .step_by(7)
            .filter(|i| text.is_char_boundary(*i))
        {
            let damaged: Vec<(&str, &str)> = parts
                .iter()
                .map(|(n, t)| (n.as_str(), if n == victim { &t[..cut] } else { t.as_str() }))
                .collect();
            let wb = read_xlsx(Cursor::new(package(&damaged)), Locale::en())
                .expect("the rest of the workbook loads");
            assert_eq!(wb.sheets.len(), 4);
            // The other sheets do not suffer.
            assert_eq!(wb.sheets[0].comments.len(), 6, "{victim} cut at {cut}");
            assert_eq!(wb.sheets[2].comments.len(), 2, "{victim} cut at {cut}");
            attempts += 1;
        }
    }
    assert!(attempts > 1000, "{attempts}");

    let originals = [
        ("xl/comments1.xml", excel_comments()),
        ("xl/drawings/vmlDrawing1.vml", excel_vml()),
        ("xl/threadedComments/threadedComment1.xml", excel_threads()),
        ("xl/persons/person.xml", excel_persons()),
        ("xl/comments2.xml", BARE_COMMENTS.to_string()),
    ];
    for (victim, text) in &originals {
        for cut in (0..text.len()).filter(|i| text.is_char_boundary(*i)) {
            let wb = load(&excel_package(&[(victim, Some(&text[..cut]))]));
            assert_eq!(wb.sheets.len(), 2);
            if *victim != "xl/comments2.xml" {
                assert_eq!(wb.sheets[1].comments.len(), 3, "{victim} cut at {cut}");
            }
            // Cutting the boxes or the people costs no note and no thread.
            if victim.ends_with(".vml") || victim.contains("person") {
                assert_eq!(wb.sheets[0].comments.len(), 6, "{victim} cut at {cut}");
            }
            // Whatever is left can be saved.
            if cut % 97 == 0 {
                load(&to_bytes(&wb));
            }
        }
    }

    // Parts that are something else altogether.
    for junk in [
        "",
        "not xml at all",
        "<",
        "<comments><commentList><comment ref=\"A1\"><text><r><rPr><sz val=\"1e999\"/></rPr><t>x",
        "<xml><v:shape><x:ClientData ObjectType=\"Note\"><x:Anchor>1,1,1,1,1,1,1,1</x:Anchor><x:Row>-1</x:Row><x:Column>1e9</x:Column>",
        "<ThreadedComments><threadedComment ref=\"A1\" dT=\"9999-99-99T99:99:99\" id=\"\" parentId=\"\"><text>",
        "<personList><person/><person id=\"\"/><person id=\"{X}\" displayName=\"&#0;&bogus;\"/>",
        "\u{feff}<?xml version=\"1.0\"?><!DOCTYPE x [<!ENTITY a \"b\">]><comments/>",
    ] {
        for victim in originals.iter().map(|(name, _)| *name) {
            let wb = load(&excel_package(&[(victim, Some(junk))]));
            assert_eq!(wb.sheets.len(), 2, "{victim}: {junk}");
        }
    }
}
