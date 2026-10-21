//! The proofing tools driven in GPUI's headless window: commands, keys,
//! dialogs and panes, with stand-ins for what macOS provides. What these
//! tests cannot tell is how things look.

use super::accessibility::{self, AccessibilityUi, Action, Rule, Target};
use super::spelling::{Place, Spelling, SpellingDialog};
use super::system::{Sense, Services, Synonyms, TranslateError, Translation, fake};
use super::thesaurus::{self, ThesaurusPane, ThesaurusUi};
use super::translate::{self, State, TranslatePane, TranslateUi};
use super::*;
use crate::features::preferences::settings;
use crate::features::{self, drawings};
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, point, px};
use opensheet_engine::chart::{Chart, ChartKind, ChartTitle};
use opensheet_engine::drawing::{Frame, ShapeTool};
use opensheet_engine::hyperlink::Hyperlink;
use opensheet_engine::page::HfVariant;
use opensheet_engine::protect::SheetProtection;
use opensheet_engine::style::Color;
use opensheet_engine::table::TableOption;
use opensheet_engine::{Rect, Value, Workbook};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// A window on an empty workbook, with the key bindings of the application.
fn open(cx: &mut TestAppContext) -> (Entity<ExcelApp>, &mut VisualTestContext) {
    cx.update(crate::bind_keys);
    let (app, cx) = cx.add_window_view(ExcelApp::new);
    redraw(cx);
    (app, cx)
}

/// Draw the window as it is now (a pointer move far from everything).
fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::none());
}

/// Change the workbook as a command would, and draw the result.
fn edit(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, f: impl FnOnce(&mut Workbook)) {
    app.update(cx, |app, cx| {
        f(&mut app.wb);
        app.commit("setup", cx);
    });
    redraw(cx);
}

fn run(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cmd: Cmd) {
    app.update_in(cx, |app, window, cx| app.run_command(cmd, window, cx));
    redraw(cx);
}

fn select(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, row: u32, col: u32) {
    app.update(cx, |app, cx| app.select_cell(row, col, cx));
    redraw(cx);
}

fn text(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, row: u32, col: u32) -> String {
    app.update(cx, |app, _| app.wb.edit_text(0, row, col))
}

fn modals(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> usize {
    app.update(cx, |app, _| app.modals.len())
}

fn undo_label(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Option<String> {
    app.update(cx, |app, _| app.wb.undo_label().map(str::to_string))
}

// ----------------------------------------------------------------- spelling

/// A checker that knows a few English words.
fn checker() -> Rc<fake::Words> {
    Rc::new(fake::Words::knowing(
        &[
            "the", "cat", "sat", "end", "a", "note", "on", "box", "of", "sales", "page", "budget",
            "total", "ada", "good", "first", "ten", "that", "and",
        ],
        &[
            ("catt", &["cat", "cart"]),
            ("teh", &["the", "ten"]),
            ("Teh", &["The"]),
            ("salles", &["sales"]),
        ],
    ))
}

fn use_checker(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Rc<fake::Words> {
    let words = checker();
    let shared = words.clone();
    app.update(cx, |app, _| {
        app.ext_mut::<Services>().spelling = Some(shared)
    });
    words
}

/// The word the Spelling dialog shows, where it is and its suggestions.
fn issue(
    app: &Entity<ExcelApp>,
    cx: &mut VisualTestContext,
) -> Option<(String, Place, Vec<String>)> {
    app.update(cx, |app, _| {
        let issue = app.ext::<Spelling>()?.session.as_ref()?.current.clone()?;
        Some((issue.word, issue.place, issue.suggestions))
    })
}

fn word(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Option<String> {
    issue(app, cx).map(|(word, ..)| word)
}

fn dialog(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Entity<SpellingDialog> {
    app.update(cx, |app, _| {
        app.ext::<Spelling>()
            .and_then(|spelling| spelling.dialog.clone())
            .expect("the Spelling dialog is open")
            .0
    })
}

/// Press a button of the Spelling dialog.
fn press(
    app: &Entity<ExcelApp>,
    cx: &mut VisualTestContext,
    f: impl FnOnce(&mut SpellingDialog, &mut gpui::Window, &mut gpui::Context<SpellingDialog>),
) {
    let dialog = dialog(app, cx);
    dialog.update_in(cx, f);
    redraw(cx);
}

#[gpui::test]
fn f7_checks_the_sheet_word_by_word(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let words = use_checker(&app, cx);
    edit(&app, cx, |wb| {
        wb.set_cell_text(0, 0, 0, "The catt sat").unwrap();
        // Formulas and numbers are not read, nor words in capitals or
        // with digits.
        wb.set_cell_text(0, 1, 0, "=\"zzz\"&1").unwrap();
        wb.set_cell_text(0, 2, 0, "12").unwrap();
        wb.set_cell_text(0, 3, 0, "NASA Q3 abc1").unwrap();
        wb.set_cell_text(0, 4, 1, "teh end teh").unwrap();
        wb.set_cell_text(0, 5, 0, "Teh cat, teh end").unwrap();
        wb.set_cell_text(0, 6, 0, "blorp").unwrap();
        wb.set_cell_text(0, 7, 0, "zork and zork").unwrap();
        wb.set_cell_text(0, 8, 0, "zork frobnicate").unwrap();
    });
    cx.simulate_keystrokes("f7");
    redraw(cx);
    assert_eq!(modals(&app, cx), 1);
    let (found, place, suggestions) = issue(&app, cx).expect("a first word");
    assert_eq!(
        (found.as_str(), place, suggestions),
        (
            "catt",
            Place::Cell(0, 0),
            vec!["cat".to_string(), "cart".to_string()]
        )
    );
    // Return is Change, with the first suggestion.
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(text(&app, cx, 0, 0), "The cat sat");
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Spelling"));
    // The next word; its cell is the active one.
    assert_eq!(word(&app, cx).as_deref(), Some("teh"));
    assert_eq!(app.update(cx, |app, _| app.active()), (4, 1));
    // Change All, with the second suggestion: every "teh" left, as one
    // step. "Teh" is another word.
    press(&app, cx, |dialog, window, cx| {
        dialog.select_suggestion(1);
        dialog.change(true, window, cx);
    });
    assert_eq!(text(&app, cx, 4, 1), "ten end ten");
    assert_eq!(text(&app, cx, 5, 0), "Teh cat, ten end");
    assert_eq!(word(&app, cx).as_deref(), Some("Teh"));
    // A correction typed over the word.
    press(&app, cx, |dialog, window, cx| {
        dialog.type_word("That", cx);
        dialog.change(false, window, cx);
    });
    assert_eq!(text(&app, cx, 5, 0), "That cat, ten end");
    assert_eq!(word(&app, cx).as_deref(), Some("blorp"));
    press(&app, cx, |dialog, window, cx| dialog.add(window, cx));
    assert_eq!(*words.learned.borrow(), vec!["blorp".to_string()]);
    // Ignore All passes over the word here and in the cells after.
    assert_eq!(word(&app, cx).as_deref(), Some("zork"));
    press(&app, cx, |dialog, window, cx| dialog.ignore_all(window, cx));
    assert_eq!(word(&app, cx).as_deref(), Some("frobnicate"));
    // Without a suggestion nor a correction, Change has nothing to put.
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(word(&app, cx).as_deref(), Some("frobnicate"));
    press(&app, cx, |dialog, window, cx| {
        dialog.ignore_once(window, cx)
    });
    // The end: the dialog goes and Excel says so.
    assert_eq!(modals(&app, cx), 1);
    assert!(app.update(cx, |app, _| {
        app.ext::<Spelling>()
            .is_none_or(|spelling| spelling.session.is_none())
    }));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(modals(&app, cx), 0);
    // Each change is a step of its own.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&app, cx, 5, 0), "Teh cat, ten end");
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(
        (text(&app, cx, 4, 1), text(&app, cx, 5, 0)),
        ("teh end teh".to_string(), "Teh cat, teh end".to_string())
    );
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&app, cx, 0, 0), "The catt sat");
}

#[gpui::test]
fn a_sheet_without_mistakes_is_said_to_be_good(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    use_checker(&app, cx);
    edit(&app, cx, |wb| {
        wb.set_cell_text(0, 0, 0, "The cat sat").unwrap()
    });
    run(&app, cx, Cmd::Spelling);
    // No dialog: only the message.
    assert_eq!(modals(&app, cx), 1);
    assert!(app.update(cx, |app, _| {
        app.ext::<Spelling>()
            .is_none_or(|spelling| spelling.dialog.is_none())
    }));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(modals(&app, cx), 0);
}

#[gpui::test]
fn the_check_goes_on_from_the_top_when_asked(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    use_checker(&app, cx);
    edit(&app, cx, |wb| {
        wb.set_cell_text(0, 0, 0, "catt first").unwrap();
        wb.set_cell_text(0, 3, 0, "teh end").unwrap();
    });
    // From A3 the check finds A4, then asks about the cells before.
    select(&app, cx, 2, 0);
    run(&app, cx, Cmd::Spelling);
    assert_eq!(word(&app, cx).as_deref(), Some("teh"));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(text(&app, cx, 3, 0), "the end");
    // The question, over the dialog; "No" ends the check.
    assert_eq!(modals(&app, cx), 2);
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!(text(&app, cx, 0, 0), "catt first");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(modals(&app, cx), 0);
    // Again, answering "Yes": the first cell is read.
    edit(&app, cx, |wb| wb.set_cell_text(0, 3, 0, "teh end").unwrap());
    select(&app, cx, 2, 0);
    run(&app, cx, Cmd::Spelling);
    press(&app, cx, |dialog, window, cx| {
        dialog.ignore_once(window, cx)
    });
    assert_eq!(modals(&app, cx), 2);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(
        (modals(&app, cx), word(&app, cx)),
        (1, Some("catt".to_string()))
    );
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(text(&app, cx, 0, 0), "cat first");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(modals(&app, cx), 0);
}

#[gpui::test]
fn a_selection_is_all_that_is_checked(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    use_checker(&app, cx);
    edit(&app, cx, |wb| {
        for (row, value) in ["catt", "teh", "catt", "teh"].into_iter().enumerate() {
            wb.set_cell_text(0, row as u32, 0, value).unwrap();
        }
        wb.set_note(0, 0, 2, "Ada", "Ada:\nteh note");
    });
    app.update(cx, |app, cx| app.select_rect(Rect::new(1, 0, 2, 0), cx));
    run(&app, cx, Cmd::Spelling);
    assert_eq!(word(&app, cx).as_deref(), Some("teh"));
    // The selection stays; the active cell walks through it.
    assert_eq!(
        app.update(cx, |app, _| (app.sel().current(), app.active())),
        (Rect::new(1, 0, 2, 0), (1, 0))
    );
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(word(&app, cx).as_deref(), Some("catt"));
    assert_eq!(app.update(cx, |app, _| app.active()), (2, 0));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    // Neither the other cells, nor the note, nor a question.
    assert!(word(&app, cx).is_none());
    assert_eq!(modals(&app, cx), 1);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    let column: Vec<String> = (0..4).map(|row| text(&app, cx, row, 0)).collect();
    assert_eq!(column, vec!["catt", "the", "cat", "teh"]);
}

#[gpui::test]
fn notes_comments_shapes_titles_and_headers_are_checked(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    use_checker(&app, cx);
    let mut shape = 0;
    let mut chart = 0;
    edit(&app, cx, |wb| {
        wb.set_cell_text(0, 0, 0, "Good").unwrap();
        // The name that opens a note is not a word to check.
        wb.set_note(0, 1, 1, "Zorglub", "Zorglub:\nA catt note");
        let author = opensheet_engine::comment::Person::new("Ada");
        wb.add_comment(0, 2, 2, &author, 45000.0, "teh first");
        wb.add_reply(0, 2, 2, &author, 45000.5, "on teh box");
        shape = wb
            .add_shape(
                0,
                ShapeTool::Rectangle,
                Frame::new(300.0, 40.0, 120.0, 60.0),
            )
            .expect("a shape");
        wb.update_drawing(0, shape, |drawing| {
            drawing.as_shape_mut().unwrap().text = "Box of catt".to_string()
        });
        let titled = Chart {
            title: ChartTitle::Text("Total salles".to_string()),
            ..Chart::new(ChartKind::Column)
        };
        chart = wb.add_chart(0, titled).expect("a chart");
        wb.snapshot();
        let both = &mut wb.sheets[0].page.header_footer;
        both.odd_header = "&L&\"Arial,Bold\"&14Budget of teh cat&RPage &P".to_string();
        both.odd_footer = "&Ccatt".to_string();
    });
    run(&app, cx, Cmd::Spelling);
    let mut seen = Vec::new();
    while let Some((found, place, _)) = issue(&app, cx) {
        seen.push((found, place));
        cx.simulate_keystrokes("enter");
        redraw(cx);
    }
    assert_eq!(
        seen,
        vec![
            ("catt".to_string(), Place::Note(1, 1)),
            ("teh".to_string(), Place::Comment(2, 2, None)),
            ("teh".to_string(), Place::Comment(2, 2, Some(0))),
            ("catt".to_string(), Place::Shape(shape)),
            ("salles".to_string(), Place::ChartTitle(chart)),
            ("teh".to_string(), Place::Header(HfVariant::Odd, false)),
            ("catt".to_string(), Place::Header(HfVariant::Odd, true)),
        ]
    );
    app.update(cx, |app, _| {
        let sheet = &app.wb.sheets[0];
        assert_eq!(
            sheet.comments[&(1, 1)].as_note().unwrap().text(),
            "Zorglub:\nA cat note"
        );
        let thread = sheet.comments[&(2, 2)].as_thread().unwrap();
        assert_eq!(
            (thread.first.text.as_str(), thread.replies[0].text.as_str()),
            ("the first", "on the box")
        );
        assert_eq!(
            app.wb.drawing(0, shape).unwrap().as_shape().unwrap().text,
            "Box of cat"
        );
        assert_eq!(
            app.wb.chart(0, chart).unwrap().title,
            ChartTitle::Text("Total sales".to_string())
        );
        // The codes of the header are where they were.
        assert_eq!(
            sheet.page.header_footer.odd_header,
            "&L&\"Arial,Bold\"&14Budget of the cat&RPage &P"
        );
        assert_eq!(sheet.page.header_footer.odd_footer, "&Ccat");
    });
    // The message of the end, then one undo per change.
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(modals(&app, cx), 0);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(
        app.update(cx, |app, _| app.wb.sheets[0]
            .page
            .header_footer
            .odd_footer
            .clone()),
        "&Ccatt"
    );
}

#[gpui::test]
fn undo_last_takes_answers_back(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let words = use_checker(&app, cx);
    edit(&app, cx, |wb| {
        wb.set_cell_text(0, 0, 0, "catt").unwrap();
        wb.set_cell_text(0, 1, 0, "teh blorp").unwrap();
        wb.set_cell_text(0, 2, 0, "teh zork").unwrap();
    });
    run(&app, cx, Cmd::Spelling);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(
        (text(&app, cx, 0, 0), word(&app, cx)),
        ("cat".to_string(), Some("teh".to_string()))
    );
    // The change is taken back and its word shown again.
    press(&app, cx, |dialog, window, cx| dialog.undo_last(window, cx));
    assert_eq!(
        (text(&app, cx, 0, 0), word(&app, cx)),
        ("catt".to_string(), Some("catt".to_string()))
    );
    press(&app, cx, |dialog, window, cx| {
        dialog.ignore_once(window, cx)
    });
    // Change All, Add and Ignore All, then back through the three.
    press(&app, cx, |dialog, window, cx| {
        dialog.change(true, window, cx)
    });
    assert_eq!(
        (text(&app, cx, 1, 0), text(&app, cx, 2, 0)),
        ("the blorp".to_string(), "the zork".to_string())
    );
    press(&app, cx, |dialog, window, cx| dialog.add(window, cx));
    assert_eq!(word(&app, cx).as_deref(), Some("zork"));
    press(&app, cx, |dialog, window, cx| dialog.ignore_all(window, cx));
    assert_eq!(modals(&app, cx), 1, "the check is over");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    // Once more, to walk back before the end.
    // (A new check starts with nothing ignored; the stand-in is told.)
    words.ignored.borrow_mut().clear();
    edit(&app, cx, |wb| wb.set_cell_text(0, 3, 0, "catt").unwrap());
    select(&app, cx, 0, 0);
    run(&app, cx, Cmd::Spelling);
    assert_eq!(word(&app, cx).as_deref(), Some("catt"));
    press(&app, cx, |dialog, window, cx| dialog.ignore_all(window, cx));
    // "catt" of A4 is passed over too: the next word is "zork".
    assert_eq!(word(&app, cx).as_deref(), Some("zork"));
    press(&app, cx, |dialog, window, cx| dialog.add(window, cx));
    assert!(words.learned.borrow().contains(&"zork".to_string()));
    assert_eq!(modals(&app, cx), 1);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(modals(&app, cx), 0);
}

#[gpui::test]
fn undo_last_unlearns_and_unignores(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let words = use_checker(&app, cx);
    edit(&app, cx, |wb| {
        wb.set_cell_text(0, 0, 0, "blorp").unwrap();
        wb.set_cell_text(0, 1, 0, "zork").unwrap();
        wb.set_cell_text(0, 2, 0, "teh").unwrap();
    });
    run(&app, cx, Cmd::Spelling);
    press(&app, cx, |dialog, window, cx| dialog.add(window, cx));
    press(&app, cx, |dialog, window, cx| dialog.ignore_all(window, cx));
    assert_eq!(word(&app, cx).as_deref(), Some("teh"));
    assert_eq!(
        (words.learned.borrow().len(), words.ignored.borrow().len()),
        (1, 1)
    );
    press(&app, cx, |dialog, window, cx| dialog.undo_last(window, cx));
    assert_eq!(
        (word(&app, cx).as_deref(), words.ignored.borrow().len()),
        (Some("zork"), 0)
    );
    press(&app, cx, |dialog, window, cx| dialog.undo_last(window, cx));
    assert_eq!(
        (word(&app, cx).as_deref(), words.learned.borrow().len()),
        (Some("blorp"), 0)
    );
    // Nothing more to take back; Escape gives the check up without a word.
    press(&app, cx, |dialog, window, cx| dialog.undo_last(window, cx));
    assert_eq!(word(&app, cx).as_deref(), Some("blorp"));
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!(modals(&app, cx), 0);
}

#[gpui::test]
fn a_protected_sheet_refuses_corrections(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    use_checker(&app, cx);
    edit(&app, cx, |wb| {
        wb.set_cell_text(0, 0, 0, "catt").unwrap();
        wb.set_cell_text(0, 1, 0, "catt").unwrap();
        // B is unlocked: it may still be corrected.
        wb.set_cell_text(0, 2, 1, "catt").unwrap();
        wb.update_style(0, Rect::new(0, 1, 9, 1), &|style| style.locked = false);
        wb.protect_sheet(0, SheetProtection::default(), "");
    });
    run(&app, cx, Cmd::Spelling);
    assert_eq!(word(&app, cx).as_deref(), Some("catt"));
    // Change: Excel's message for a protected sheet, and nothing changes.
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(
        (modals(&app, cx), text(&app, cx, 0, 0)),
        (2, "catt".to_string())
    );
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(
        (modals(&app, cx), word(&app, cx)),
        (1, Some("catt".to_string()))
    );
    press(&app, cx, |dialog, window, cx| {
        dialog.ignore_once(window, cx)
    });
    press(&app, cx, |dialog, window, cx| {
        dialog.ignore_once(window, cx)
    });
    // The unlocked cell: Change All changes it, and only it.
    assert_eq!(
        issue(&app, cx).map(|(_, place, _)| place),
        Some(Place::Cell(2, 1))
    );
    press(&app, cx, |dialog, window, cx| {
        dialog.change(true, window, cx)
    });
    let cells = [
        text(&app, cx, 0, 0),
        text(&app, cx, 1, 0),
        text(&app, cx, 2, 1),
    ];
    assert_eq!(cells, ["catt", "catt", "cat"].map(str::to_string));
}

#[gpui::test]
fn the_dictionary_language_is_chosen_and_kept(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    use_checker(&app, cx);
    edit(&app, cx, |wb| wb.set_cell_text(0, 0, 0, "catt").unwrap());
    run(&app, cx, Cmd::Spelling);
    // The language of the interface to begin with.
    let language = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| {
        app.update(cx, |app, _| {
            app.ext::<Spelling>()
                .unwrap()
                .session
                .as_ref()
                .unwrap()
                .language
                .clone()
        })
    };
    assert_eq!(language(&app, cx), "en");
    press(&app, cx, |dialog, window, cx| {
        dialog.set_language(1, window, cx)
    });
    assert_eq!(
        (language(&app, cx), word(&app, cx)),
        ("fr".to_string(), Some("catt".to_string()))
    );
    assert_eq!(settings::get().spelling_language, "fr");
    cx.simulate_keystrokes("escape");
    redraw(cx);
    // The next check starts in it.
    run(&app, cx, Cmd::Spelling);
    assert_eq!(language(&app, cx), "fr");
}

#[gpui::test]
fn a_cell_being_typed_is_stored_before_the_check(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    use_checker(&app, cx);
    cx.simulate_input("catt");
    cx.simulate_keystrokes("f7");
    redraw(cx);
    assert_eq!(
        (text(&app, cx, 0, 0), word(&app, cx)),
        ("catt".to_string(), Some("catt".to_string()))
    );
    // A text that would read as a number keeps its apostrophe.
    cx.simulate_keystrokes("escape");
    edit(&app, cx, |wb| wb.set_cell_text(0, 0, 0, "'catt").unwrap());
    run(&app, cx, Cmd::Spelling);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(
        app.update(cx, |app, _| app.wb.value_at(0, 0, 0)),
        Value::text("cat")
    );
}

// ---------------------------------------------------------------- thesaurus

fn book() -> fake::Book {
    let sense = |pos: &str, synonyms: &[&str], antonyms: &[&str]| Sense {
        part_of_speech: pos.to_string(),
        example: String::new(),
        synonyms: synonyms.iter().map(|word| word.to_string()).collect(),
        antonyms: antonyms.iter().map(|word| word.to_string()).collect(),
    };
    fake::Book(
        [
            (
                "happy".to_string(),
                vec![
                    sense("adjective", &["cheerful", "merry"], &["sad"]),
                    sense("adjective", &["willing"], &[]),
                ],
            ),
            (
                "cheerful".to_string(),
                vec![sense("adjective", &["happy", "jolly"], &[])],
            ),
        ]
        .into_iter()
        .collect(),
    )
}

fn thesaurus_pane(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Entity<ThesaurusPane> {
    app.update(cx, |app, _| {
        app.ext::<ThesaurusUi>()
            .and_then(|ui| ui.pane.clone())
            .expect("the Thesaurus pane")
    })
}

#[gpui::test]
fn shift_f7_lists_the_synonyms_of_the_cell(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    app.update(cx, |app, _| {
        app.ext_mut::<Services>().thesaurus = Some(Rc::new(book()))
    });
    edit(&app, cx, |wb| wb.set_cell_text(0, 0, 0, "Happy").unwrap());
    cx.simulate_keystrokes("shift-f7");
    redraw(cx);
    assert!(app.update(cx, |app, _| app.pane_open(thesaurus::PANE)));
    let pane = thesaurus_pane(&app, cx);
    let headings = pane.update(cx, |pane, _| match &pane.result {
        Some(Synonyms::Found { senses, .. }) => {
            senses.iter().map(thesaurus::heading).collect::<Vec<_>>()
        }
        other => panic!("{other:?}"),
    });
    assert_eq!(headings, vec!["cheerful (adj.)", "willing (adj.)"]);
    // A press on a synonym chooses it; Insert puts it in the cell.
    let merry = cx
        .debug_bounds("thesaurus-word-1")
        .expect("the second synonym is drawn")
        .center();
    cx.simulate_click(merry, Modifiers::none());
    redraw(cx);
    assert_eq!(
        pane.update(cx, |pane, _| pane.chosen.clone()).as_deref(),
        Some("merry")
    );
    pane.update(cx, |pane, cx| pane.copy_chosen(cx));
    assert_eq!(
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("merry")
    );
    pane.update(cx, |pane, cx| pane.insert_chosen(cx));
    redraw(cx);
    assert_eq!(
        (text(&app, cx, 0, 0), undo_label(&app, cx)),
        ("merry".to_string(), Some("Thesaurus".to_string()))
    );
    app.update(cx, |app, cx| app.undo(cx));
    assert_eq!(text(&app, cx, 0, 0), "Happy");
    // Two presses look the synonym up; Back returns.
    let cheerful = cx
        .debug_bounds("thesaurus-word-0")
        .expect("the first synonym")
        .center();
    cx.simulate_mouse_down(cheerful, gpui::MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(cheerful, gpui::MouseButton::Left, Modifiers::none());
    cx.simulate_event(gpui::MouseDownEvent {
        button: gpui::MouseButton::Left,
        position: cheerful,
        modifiers: Modifiers::none(),
        click_count: 2,
        first_mouse: false,
    });
    redraw(cx);
    assert_eq!(pane.update(cx, |pane, _| pane.word.clone()), "cheerful");
    pane.update(cx, |pane, cx| pane.back(cx));
    assert_eq!(pane.update(cx, |pane, _| pane.word.clone()), "Happy");
    // A word the thesaurus does not hold, typed in the search field.
    pane.update(cx, |pane, cx| pane.look_up("heureux", cx));
    redraw(cx);
    assert_eq!(
        pane.update(cx, |pane, _| pane.result.clone()),
        Some(Synonyms::NotFound)
    );
    // The command again, on another cell: the same pane, the new word.
    edit(&app, cx, |wb| {
        wb.set_cell_text(0, 1, 0, "cheerful.").unwrap()
    });
    select(&app, cx, 1, 0);
    run(&app, cx, Cmd::Thesaurus);
    assert_eq!(pane.update(cx, |pane, _| pane.word.clone()), "cheerful");
}

#[gpui::test]
fn the_thesaurus_says_when_the_mac_has_none(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.set_cell_text(0, 0, 0, "happy").unwrap();
        wb.protect_sheet(0, SheetProtection::default(), "");
    });
    run(&app, cx, Cmd::Thesaurus);
    let pane = thesaurus_pane(&app, cx);
    assert_eq!(
        pane.update(cx, |pane, _| pane.result.clone()),
        Some(Synonyms::NoThesaurus)
    );
    // A protected sheet refuses the word Insert would put.
    app.update(cx, |app, cx| assert!(!thesaurus::insert(app, "merry", cx)));
    redraw(cx);
    assert_eq!(
        (modals(&app, cx), text(&app, cx, 0, 0)),
        (1, "happy".to_string())
    );
}

// ---------------------------------------------------------------- translate

fn translator_pane(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Entity<TranslatePane> {
    app.update(cx, |app, _| {
        app.ext::<TranslateUi>()
            .and_then(|ui| ui.pane.clone())
            .expect("the Translator pane")
    })
}

fn translated(text: &str, source: &str) -> State {
    State::Done(Translation {
        text: text.to_string(),
        source: source.to_string(),
    })
}

#[gpui::test]
fn the_translator_translates_the_selected_cell(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    app.update(cx, |app, _| {
        app.ext_mut::<Services>().translator = Some(Arc::new(fake::Echo(None)))
    });
    edit(&app, cx, |wb| {
        wb.set_cell_text(0, 0, 0, "Good morning").unwrap();
        wb.set_cell_text(0, 1, 0, "Thank you").unwrap();
    });
    run(&app, cx, Cmd::Translate);
    assert!(app.update(cx, |app, _| app.pane_open(translate::PANE)));
    let pane = translator_pane(&app, cx);
    // From: detected; To: the language of the interface.
    let state = |cx: &mut VisualTestContext| {
        pane.update(cx, |pane, _| {
            (pane.from.clone(), pane.to.clone(), pane.state.clone())
        })
    };
    assert_eq!(
        state(cx),
        (
            None,
            "en".to_string(),
            translated("[en] GOOD MORNING", "en")
        )
    );
    pane.update(cx, |pane, cx| pane.set_to("fr".to_string(), cx));
    redraw(cx);
    assert_eq!(state(cx).2, translated("[fr] GOOD MORNING", "en"));
    pane.update(cx, |pane, cx| pane.set_from(Some("de".to_string()), cx));
    redraw(cx);
    assert_eq!(state(cx).2, translated("[fr] GOOD MORNING", "de"));
    assert_eq!(
        (settings::get().translate_from, settings::get().translate_to),
        ("de".to_string(), "fr".to_string())
    );
    // The pane follows the selection.
    select(&app, cx, 1, 0);
    assert_eq!(
        pane.update(cx, |pane, cx| pane.source_text(cx)),
        "Thank you"
    );
    assert_eq!(state(cx).2, translated("[fr] THANK YOU", "de"));
    // Insert puts the translation in the cell, as one step, and the pane
    // does not translate it in its turn.
    pane.update(cx, |pane, cx| pane.insert(cx));
    redraw(cx);
    assert_eq!(
        (text(&app, cx, 1, 0), undo_label(&app, cx)),
        ("[fr] THANK YOU".to_string(), Some("Translate".to_string()))
    );
    assert_eq!(
        pane.update(cx, |pane, cx| pane.source_text(cx)),
        "Thank you"
    );
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(text(&app, cx, 1, 0), "Thank you");
    // An empty cell: nothing to translate.
    select(&app, cx, 5, 5);
    assert_eq!(state(cx).2, State::Empty);
    // A text typed in the box is translated after a pause.
    pane.update(cx, |pane, cx| pane.follow("typed".to_string(), cx));
    redraw(cx);
    pane.update(cx, |pane, cx| {
        pane.translate_after(Duration::from_millis(500), cx)
    });
    cx.run_until_parked();
    assert_eq!(state(cx).2, State::Working);
    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    assert_eq!(state(cx).2, translated("[fr] TYPED", "de"));
}

#[gpui::test]
fn the_translator_opens_and_explains_when_macos_cannot_translate(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    edit(&app, cx, |wb| {
        wb.set_cell_text(0, 0, 0, "Good morning").unwrap()
    });
    // What a window gets in a test: no helper next to the application.
    run(&app, cx, Cmd::Translate);
    let pane = translator_pane(&app, cx);
    assert_eq!(
        pane.update(cx, |pane, _| pane.state.clone()),
        State::Failed(TranslateError::NoHelper)
    );
    assert!(
        cx.debug_bounds("translation").is_some(),
        "the pane is drawn all the same"
    );
    // Languages that are not downloaded: the pane says where to get them.
    app.update(cx, |app, cx| {
        app.close_pane(translate::PANE, cx);
        app.ext_mut::<Services>().translator =
            Some(Arc::new(fake::Echo(Some(TranslateError::NotInstalled))));
    });
    run(&app, cx, Cmd::Translate);
    let pane = translator_pane(&app, cx);
    assert_eq!(
        pane.update(cx, |pane, _| pane.state.clone()),
        State::Failed(TranslateError::NotInstalled)
    );
    pane.update(cx, |pane, cx| pane.insert(cx));
    assert_eq!(text(&app, cx, 0, 0), "Good morning");
    assert!(translate::where_languages_are().contains("System Settings"));
    for error in [
        TranslateError::NoHelper,
        TranslateError::UnsupportedSystem,
        TranslateError::NotInstalled,
        TranslateError::UnsupportedPair,
        TranslateError::UnknownLanguage,
        TranslateError::Failed("busy".to_string()),
    ] {
        assert!(!translate::explain(&error).is_empty());
    }
    assert!(translate::explain(&TranslateError::Failed("busy".to_string())).ends_with("busy"));
    // A protected sheet refuses Insert.
    app.update(cx, |app, cx| {
        app.wb.protect_sheet(0, SheetProtection::default(), "");
        app.commit("protect", cx);
        assert!(!translate::insert(app, "Bonjour", cx));
    });
}

// ------------------------------------------------------------ accessibility

/// A workbook that breaks every rule once.
fn troubled(wb: &mut Workbook) -> (u32, u32) {
    // A1:B4, a table with a blank row, whose header row is then hidden.
    wb.set_cell_text(0, 0, 0, "Name").unwrap();
    wb.set_cell_text(0, 0, 1, "Qty").unwrap();
    wb.set_cell_text(0, 1, 0, "a").unwrap();
    wb.set_cell_text(0, 1, 1, "1").unwrap();
    wb.set_cell_text(0, 3, 0, "c").unwrap();
    wb.set_cell_text(0, 3, 1, "3").unwrap();
    let table = wb.add_table(0, Rect::new(0, 0, 3, 1), true).unwrap();
    wb.set_table_option(0, table, TableOption::HeaderRow, false)
        .unwrap();
    // Yellow on white, twice side by side.
    wb.set_cell_text(0, 5, 0, "faint").unwrap();
    wb.set_cell_text(0, 5, 1, "faint").unwrap();
    wb.update_style(0, Rect::new(5, 0, 5, 1), &|style| {
        style.font.color = Some(Color(0xFFFF00))
    });
    // Merged cells.
    wb.merge(0, Rect::new(7, 0, 7, 2));
    // Negative numbers in red alone.
    wb.set_cell_text(0, 9, 0, "-5").unwrap();
    wb.update_style(0, Rect::new(9, 0, 9, 0), &|style| {
        style.num_fmt = Arc::from("0;[Red]0")
    });
    // A link that shows as its address, and one that reads well.
    wb.set_cell_text(0, 11, 0, "https://example.org/a").unwrap();
    wb.set_cell_text(0, 12, 0, "Our prices").unwrap();
    let link = |row: u32| Hyperlink {
        range: Rect::cell(row, 0),
        address: "https://example.org/a".to_string(),
        location: String::new(),
        tooltip: String::new(),
    };
    wb.set_hyperlink(0, link(11));
    wb.set_hyperlink(0, link(12));
    // A shape without text nor description, and one that has a text.
    let bare = wb
        .add_shape(0, ShapeTool::Ellipse, Frame::new(400.0, 40.0, 80.0, 60.0))
        .expect("a shape");
    let labelled = wb
        .add_shape(
            0,
            ShapeTool::Rectangle,
            Frame::new(400.0, 140.0, 80.0, 60.0),
        )
        .expect("a shape");
    wb.update_drawing(0, labelled, |drawing| {
        drawing.as_shape_mut().unwrap().text = "Total".to_string()
    });
    (bare, labelled)
}

fn rules(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Vec<(Rule, String)> {
    app.update(cx, |app, _| {
        accessibility::findings(&app.wb)
            .iter()
            .map(|finding| (finding.rule, finding.label.clone()))
            .collect()
    })
}

fn status(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> String {
    app.update(cx, |app, _| {
        let items = features::status_items(app);
        items
            .iter()
            .map(|item| item.text.to_string())
            .find(|text| text.starts_with("Accessibility"))
            .expect("the indicator")
    })
}

fn accessibility_pane(
    app: &Entity<ExcelApp>,
    cx: &mut VisualTestContext,
) -> Entity<accessibility::AccessibilityPane> {
    app.update(cx, |app, _| {
        app.ext::<AccessibilityUi>()
            .and_then(|ui| ui.pane.clone())
            .expect("the Accessibility pane")
    })
}

/// Select the result of a rule in the pane.
fn choose(
    app: &Entity<ExcelApp>,
    cx: &mut VisualTestContext,
    rule: Rule,
) -> accessibility::Finding {
    let pane = accessibility_pane(app, cx);
    let finding = pane
        .update(cx, |pane, cx| {
            pane.listed(cx)
                .iter()
                .find(|finding| finding.rule == rule)
                .cloned()
        })
        .expect("a result of the rule");
    let chosen = finding.clone();
    pane.update_in(cx, |pane, window, cx| pane.select(chosen, window, cx));
    redraw(cx);
    finding
}

fn act(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, action: Action) {
    let pane = accessibility_pane(app, cx);
    pane.update_in(cx, |pane, window, cx| pane.run(action, window, cx));
    redraw(cx);
}

#[gpui::test]
fn the_checker_lists_what_excel_lists(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    // An empty workbook is good to go, whatever its sheet is called.
    assert_eq!(
        (rules(&app, cx), status(&app, cx)),
        (Vec::new(), "Accessibility: Good to go".to_string())
    );
    edit(&app, cx, |wb| {
        troubled(wb);
    });
    assert_eq!(status(&app, cx), "Accessibility: Investigate");
    let expected = [
        (Rule::MissingAltText, "Oval 1 (Sheet1)"),
        (Rule::MissingTableHeader, "Table1 (Sheet1)"),
        (Rule::Contrast, "A6:B6 (Sheet1)"),
        (Rule::MergedCells, "A8:C8 (Sheet1)"),
        (Rule::RedNegative, "A10 (Sheet1)"),
        (Rule::BlankTableRows, "A3:B3 (Sheet1)"),
        (Rule::UnclearLink, "A12 (Sheet1)"),
        (Rule::DefaultSheetName, "Sheet1"),
    ];
    let found = rules(&app, cx);
    let found: Vec<(Rule, &str)> = found
        .iter()
        .map(|(rule, label)| (*rule, label.as_str()))
        .collect();
    assert_eq!(found, expected);
    // Every rule has Excel's words, in every language of the interface.
    for lang in crate::i18n::Lang::ALL {
        crate::i18n::set_lang(lang);
        for (rule, _) in expected {
            assert!(!rule.title().is_empty() && !rule.why().is_empty() && !rule.steps().is_empty());
            assert!(
                rule.actions()
                    .iter()
                    .all(|action| !action.label().is_empty())
            );
        }
    }
    crate::i18n::set_lang(crate::i18n::Lang::En);
}

#[gpui::test]
fn the_accessibility_pane_shows_and_fixes(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let mut bare = 0;
    edit(&app, cx, |wb| bare = troubled(wb).0);
    // The indicator of the status bar opens the pane.
    app.update_in(cx, |app, window, cx| {
        let item = features::status_items(app)
            .into_iter()
            .find(|item| item.text.starts_with("Accessibility"))
            .unwrap();
        (item.on_click.expect("the indicator answers a click"))(app, window, cx);
    });
    redraw(cx);
    assert!(app.update(cx, |app, _| app.pane_open(accessibility::PANE)));
    let pane = accessibility_pane(&app, cx);
    assert_eq!(pane.update(cx, |pane, cx| pane.listed(cx).len()), 8);
    // A press on a result selects what it is about.
    let first = cx
        .debug_bounds("accessibility-result-0")
        .expect("the first result is drawn")
        .center();
    cx.simulate_click(first, Modifiers::none());
    redraw(cx);
    assert_eq!(
        pane.update(cx, |pane, _| pane
            .selected
            .as_ref()
            .map(|finding| finding.target)),
        Some(Target::Object(0, bare))
    );
    let selected = app.update(cx, |app, _| app.selected_objects());
    assert_eq!(
        selected,
        vec![features::object_id(&drawings::Drawings, bare)]
    );
    // Add a description: the Alt Text pane, on that object.
    act(&app, cx, Action::AddDescription);
    assert!(app.update(cx, |app, _| {
        app.panes
            .iter()
            .any(|pane| pane.title.as_ref() == "Alt Text")
    }));
    // Mark as decorative: the result goes, in one step that undoes.
    choose(&app, cx, Rule::MissingAltText);
    act(&app, cx, Action::MarkDecorative);
    assert!(app.update(cx, |app, _| app.wb.drawing(0, bare).unwrap().decorative));
    assert_eq!(pane.update(cx, |pane, cx| pane.listed(cx).len()), 7);
    assert_eq!(pane.update(cx, |pane, _| pane.selected.clone()), None);
    app.update(cx, |app, cx| app.undo(cx));
    assert_eq!(rules(&app, cx).len(), 8);
    act(&app, cx, Action::MarkDecorative);
    assert_eq!(
        rules(&app, cx).len(),
        8,
        "nothing is selected: nothing is done"
    );
    choose(&app, cx, Rule::MissingAltText);
    act(&app, cx, Action::MarkDecorative);
    // Merged cells: selected, then unmerged.
    choose(&app, cx, Rule::MergedCells);
    assert_eq!(
        app.update(cx, |app, _| app.sel().current()),
        Rect::new(7, 0, 7, 2)
    );
    act(&app, cx, Action::Unmerge);
    assert!(app.update(cx, |app, _| app.wb.sheets[0].merges.is_empty()));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Unmerge Cells"));
    // The table gets its header row.
    choose(&app, cx, Rule::MissingTableHeader);
    act(&app, cx, Action::UseFirstRowAsHeader);
    assert!(app.update(cx, |app, _| app.wb.sheets[0].tables[0].header_row));
    // Contrast and number format lead to Format Cells, the link to its dialog.
    for (rule, action) in [
        (Rule::Contrast, Action::FormatCellFont),
        (Rule::RedNegative, Action::FormatCellNumber),
        (Rule::UnclearLink, Action::EditLink),
    ] {
        let finding = choose(&app, cx, rule);
        let Target::Cells(_, rect) = finding.target else {
            panic!("{finding:?}");
        };
        assert_eq!(app.update(cx, |app, _| app.sel().current()), rect);
        act(&app, cx, action);
        assert_eq!(modals(&app, cx), 1, "{rule:?}");
        cx.simulate_keystrokes("escape");
        redraw(cx);
        assert_eq!(modals(&app, cx), 0);
    }
    // The pane follows the workbook: a better colour and the result goes.
    edit(&app, cx, |wb| {
        wb.update_style(0, Rect::new(5, 0, 5, 1), &|style| style.font.color = None)
    });
    assert!(!pane.update(cx, |pane, cx| {
        pane.listed(cx)
            .iter()
            .any(|finding| finding.rule == Rule::Contrast)
    }));
    // Rename sheet: the tab is ready for a name.
    choose(&app, cx, Rule::DefaultSheetName);
    act(&app, cx, Action::RenameSheet);
    assert_eq!(app.update(cx, |app, _| app.renaming_sheet), Some(0));
    cx.simulate_input("Budget");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(
        app.update(cx, |app, _| app.wb.sheets[0].name.clone()),
        "Budget"
    );
    let left: Vec<Rule> = rules(&app, cx).into_iter().map(|(rule, _)| rule).collect();
    assert_eq!(
        left,
        vec![Rule::RedNegative, Rule::BlankTableRows, Rule::UnclearLink]
    );
    assert_eq!(status(&app, cx), "Accessibility: Investigate");
}

#[gpui::test]
fn the_ribbon_command_opens_the_pane_and_protection_holds(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let mut bare = 0;
    edit(&app, cx, |wb| {
        bare = troubled(wb).0;
        wb.add_sheet(1);
        wb.protect_sheet(0, SheetProtection::default(), "");
    });
    app.update_in(cx, |app, window, cx| app.switch_sheet(1, window, cx));
    run(&app, cx, Cmd::CheckAccessibility);
    assert!(app.update(cx, |app, _| app.pane_open(accessibility::PANE)));
    // A result of another sheet brings that sheet to the front.
    choose(&app, cx, Rule::MergedCells);
    assert_eq!(app.update(cx, |app, _| app.sheet), 0);
    // The sheet is protected: neither unmerged nor marked.
    act(&app, cx, Action::Unmerge);
    assert_eq!(
        (
            modals(&app, cx),
            app.update(cx, |app, _| app.wb.sheets[0].merges.len())
        ),
        (1, 1)
    );
    cx.simulate_keystrokes("enter");
    redraw(cx);
    choose(&app, cx, Rule::MissingAltText);
    act(&app, cx, Action::MarkDecorative);
    assert_eq!(modals(&app, cx), 1);
    assert!(!app.update(cx, |app, _| app.wb.drawing(0, bare).unwrap().decorative));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    // The drop-down of the ribbon button holds the command and Alt Text.
    run(&app, cx, Cmd::AccessibilityMenu);
    assert!(app.update(cx, |app, _| app.popup.is_some()));
}
