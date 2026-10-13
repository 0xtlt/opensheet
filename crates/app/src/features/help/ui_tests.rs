//! The Help pane and the Feedback dialog driven in GPUI's headless
//! window. The share sheet is the stand-in of `share::system::fake`.

use super::feedback::{FeedbackDialog, Kind};
use super::*;
use crate::features::share::system::{Item, fake};
use crate::features::share::ui_tests::{open, redraw};
use gpui::{Modifiers, VisualTestContext};
use gpui::{KeyBindingContextPredicate, TestAppContext};

fn run(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cmd: Cmd) {
    app.update_in(cx, |app, window, cx| app.run_command(cmd, window, cx));
    redraw(cx);
}

fn pane(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Entity<HelpPane> {
    app.update(cx, |app, _| {
        let entry = app.panes.iter().find(|pane| pane.id == PANE).expect("the Help pane is open");
        entry.view.clone().downcast::<HelpPane>().ok().expect("a Help pane")
    })
}

fn click(cx: &mut VisualTestContext, name: &'static str) {
    let at = cx.debug_bounds(name).unwrap_or_else(|| panic!("{name} is not drawn")).center();
    cx.simulate_click(at, Modifiers::none());
    redraw(cx);
}

#[gpui::test]
fn help_opens_a_pane_over_the_documentation(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    app.update(cx, |app, _| assert!(!Cmd::Help.checked(app)));
    run(&app, cx, Cmd::Help);
    app.update(cx, |app, _| assert!(app.pane_open(PANE) && Cmd::Help.checked(app)));
    let help = pane(&app, cx);
    help.update(cx, |help, _| {
        let titles = help.titles();
        // The groups of the README, then the shortcuts.
        assert!(titles.contains(&"Interface") && titles.contains(&"Calcul"), "{titles:?}");
        assert_eq!(titles.last(), Some(&"Keyboard shortcuts"));
        assert_eq!(help.open_topic(), None);
    });
    // A click on a topic shows it; Back returns to the list.
    click(cx, "help-topic-0");
    let first = help.update(cx, |help, _| help.titles()[0].to_string());
    help.update(cx, |help, _| assert_eq!(help.open_topic(), Some(first.as_str())));
    click(cx, "help-back");
    help.update(cx, |help, _| assert_eq!(help.open_topic(), None));
    // The button of the ribbon closes the pane; the menu bar opens it.
    run(&app, cx, Cmd::Help);
    app.update(cx, |app, _| assert!(!app.pane_open(PANE)));
    cx.dispatch_action(OpenHelp);
    cx.dispatch_action(OpenHelp);
    app.update(cx, |app, _| assert!(app.pane_open(PANE)));
}

#[gpui::test]
fn the_search_field_finds_lines_of_the_documentation(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    run(&app, cx, Cmd::Help);
    let help = pane(&app, cx);
    help.update(cx, |help, cx| assert_eq!(help.found(cx), None));
    click(cx, "help-search");
    // Without accents or capitals, every word wanted.
    cx.simulate_input("recalcul dependances");
    redraw(cx);
    let found = help.update(cx, |help, cx| help.found(cx)).expect("a search");
    assert!(
        found.iter().any(|(topic, lines)| topic == "Calcul" && lines.iter().any(|line| line.contains("Recalcul par dépendances"))),
        "{found:?}"
    );
    // A shortcut is found by what it does or by its keys.
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("copy");
    let found = help.update(cx, |help, cx| help.found(cx)).unwrap();
    assert!(found.iter().any(|(topic, lines)| topic == "Keyboard shortcuts" && lines.contains(&"Copy".to_string())), "{found:?}");
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("⌘P");
    let found = help.update(cx, |help, cx| help.found(cx)).unwrap();
    assert_eq!(found, [("Keyboard shortcuts".to_string(), vec!["Print".to_string()])]);
    // Nothing found is said, not left blank.
    cx.simulate_keystrokes("cmd-a");
    cx.simulate_input("zzzzqq");
    redraw(cx);
    assert_eq!(help.update(cx, |help, cx| help.found(cx)), Some(Vec::new()));
    // Emptied, the list of topics is back.
    cx.simulate_keystrokes("cmd-a backspace");
    help.update(cx, |help, cx| assert_eq!(help.found(cx), None));
    // Typing in the pane never reached the sheet.
    app.update(cx, |app, _| assert!(!app.wb.can_undo() && !app.is_editing()));
}

#[gpui::test]
fn the_shortcuts_are_those_the_application_binds(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    run(&app, cx, Cmd::Help);
    let help = pane(&app, cx);
    let listed: Vec<(String, String)> = help.update(cx, |help, _| {
        help.topic("Keyboard shortcuts")
            .lines
            .iter()
            .map(|line| (line.keys.clone().expect("keys"), line.text.clone()))
            .collect()
    });
    let keys_of = |what: &str| listed.iter().find(|(_, text)| text == what).map(|(keys, _)| keys.as_str());
    assert_eq!(keys_of("Copy"), Some("⌘C"));
    assert_eq!(keys_of("Go To"), Some("⌃G, ⌘G, F5"));
    assert_eq!(keys_of("New Comment"), Some("⇧⌘M, ⇧⌘F2"));
    assert_eq!(keys_of("Select Cell Up"), Some("↑"));
    assert_eq!(keys_of("Move to the edge of the current data region"), Some("⌘↑, ⌘↓, ⌘←, ⌘→, ⌃↑, ⌃↓, ⌃←, ⌃→"));
    assert_eq!(keys_of("Preferences"), Some("⌘,"));
    // Every key bound on the sheet or on the window is in the list, once.
    let bound: Vec<String> = cx.update(|_, cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        keymap
            .bindings()
            .filter(|binding| match binding.predicate().as_deref() {
                None => true,
                Some(KeyBindingContextPredicate::Identifier(context)) => context.as_ref() == "Grid" || context.as_ref() == "Workbook",
                Some(_) => false,
            })
            .map(|binding| binding.keystrokes().iter().map(|stroke| shortcuts::keys(stroke.inner())).collect::<Vec<_>>().join(" "))
            .collect()
    });
    assert!(bound.len() > 100, "{} bindings", bound.len());
    let all: Vec<&str> = listed.iter().flat_map(|(keys, _)| keys.split(", ")).collect();
    for stroke in &bound {
        assert!(all.contains(&stroke.as_str()), "{stroke} is bound and not listed");
    }
    // The keys of text boxes are not: Backspace alone clears a cell here.
    assert_eq!(keys_of("Clear Contents"), Some("⌫, ⌦"));
    // No action is listed under a made-up name: each has its words.
    for (_, text) in &listed {
        assert!(!text.is_empty() && !text.contains("::"), "{text}");
    }
}

fn dialog_view(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Entity<FeedbackDialog> {
    app.update(cx, |app, _| {
        let view = app.front_modal_view().expect("a dialog is open");
        view.downcast::<FeedbackDialog>().ok().expect("the Feedback dialog")
    })
}

#[gpui::test]
fn feedback_goes_to_the_share_sheet_and_nowhere_else(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    run(&app, cx, Cmd::Feedback);
    redraw(cx);
    let dialog = dialog_view(&app, cx);
    dialog.update(cx, |dialog, cx| {
        assert_eq!(dialog.kind, Kind::Like);
        assert_eq!(dialog.hint(cx), "Tell us what you like");
    });
    // The dialog opens with the keyboard in the box.
    assert!(dialog.update_in(cx, |dialog, window, cx| dialog.text_focused(window, cx)));
    // Nothing written: Send does nothing, by the button or by ⌘Return.
    click(cx, "feedback-send");
    click(cx, "feedback-text");
    cx.simulate_keystrokes("cmd-enter");
    assert!(fake::taken().is_empty());
    // Return is a line break in the box.
    cx.simulate_input("Charts are great");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("and fast.");
    dialog.update(cx, |dialog, cx| assert_eq!(dialog.draft(cx), "Charts are great\nand fast."));
    // A click on the third choice.
    let kinds = cx.debug_bounds("feedback-kind").expect("the choices");
    cx.simulate_click(gpui::point(kinds.left() + px(8.), kinds.top() + kinds.size.height * (5.0 / 6.0)), Modifiers::none());
    redraw(cx);
    dialog.update(cx, |dialog, cx| {
        assert_eq!(dialog.kind, Kind::Idea);
        // The hint follows the choice; what was written stays.
        assert_eq!(dialog.hint(cx), "Tell us your idea");
        assert_eq!(dialog.draft(cx), "Charts are great\nand fast.");
    });
    redraw(cx);
    let send = cx.debug_bounds("feedback-send").expect("the Send button");
    click(cx, "feedback-send");
    let mut shared = fake::taken();
    assert_eq!(shared.len(), 1, "one share sheet");
    let (items, under) = shared.remove(0);
    // The sheet hangs under the Send button and holds the text, what it
    // is about and the two versions; no file, no recipient.
    assert_eq!(under, send);
    let expected = format!(
        "I have a suggestion\n\nCharts are great\nand fast.\n\nOpenSheet {} — macOS 0.0",
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(items, [Item::Text(expected)]);
    // The dialog stays, with the text: the user may leave the sheet
    // without sending.
    app.update(cx, |app, _| assert!(app.modal_open()));
    dialog.update(cx, |dialog, cx| assert_eq!(dialog.draft(cx), "Charts are great\nand fast."));
    // Send is the default button: Return outside the box presses it, and
    // ⌘Return does from inside the box.
    cx.simulate_keystrokes("enter");
    assert_eq!(fake::taken().len(), 1);
    click(cx, "feedback-text");
    cx.simulate_keystrokes("cmd-enter");
    assert_eq!(fake::taken().len(), 1);
    // A share sheet that fails is reported.
    fake::fail(true);
    cx.simulate_keystrokes("cmd-enter");
    fake::fail(false);
    let (title, message) = app.update(cx, |app, cx| app.front_alert(cx)).expect("a message");
    assert_eq!(title, "Send Feedback");
    assert!(message.starts_with("Something went wrong"), "{message}");
    cx.simulate_keystrokes("enter");
    // Escape in the box closes the dialog; the workbook was never touched.
    redraw(cx);
    click(cx, "feedback-text");
    cx.simulate_keystrokes("escape");
    app.update(cx, |app, _| assert!(!app.modal_open() && !app.wb.can_undo()));
    // From the menu bar.
    cx.dispatch_action(SendFeedback);
    app.update(cx, |app, _| assert!(app.modal_open()));
    redraw(cx);
    click(cx, "feedback-close");
    app.update(cx, |app, _| assert!(!app.modal_open()));
}

/// Click a button of the ribbon by the id it was given.
pub(crate) fn click_ribbon(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, tab: usize, id: &'static str) {
    app.update(cx, |app, cx| {
        app.ribbon_tab = tab;
        cx.notify();
    });
    redraw(cx);
    let button = app.update(cx, |app, _| app.anchors.borrow().get(id).copied()).unwrap_or_else(|| panic!("{id} is not on the tab"));
    cx.simulate_click(button.center(), Modifiers::none());
    redraw(cx);
}

#[gpui::test]
fn the_help_tab_runs_its_two_commands(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    click_ribbon(&app, cx, 10, "hp-help");
    app.update(cx, |app, _| assert!(app.pane_open(PANE)));
    pane(&app, cx);
    click_ribbon(&app, cx, 10, "hp-feedback");
    dialog_view(&app, cx);
    cx.simulate_keystrokes("escape");
    // A second click on Help puts the pane away.
    click_ribbon(&app, cx, 10, "hp-help");
    app.update(cx, |app, _| assert!(!app.pane_open(PANE) && !app.modal_open()));
}
