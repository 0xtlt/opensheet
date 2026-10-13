//! The folding ribbon in GPUI's headless window: every tab at several
//! widths and in several languages. What is checked is what the eye would
//! check: the tab fits, no control leaves its group or the ribbon, no two
//! controls overlap, and every command of the wide ribbon can still be
//! reached (on the ribbon, or in the drop-down of a folded group).

use super::*;
use crate::features::{self, Command};
use crate::i18n::{Lang, set_lang};
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use opensheet_engine::Rect;
use opensheet_engine::drawing::{Frame, ShapeTool};
use std::collections::{BTreeMap, BTreeSet};

#[test]
fn steps_follow_excel_then_go_from_the_right() {
    // A tab Excel has no list for: stage by stage, from the right.
    assert_eq!(
        sequence("shape-format/en", 3),
        vec![(2, S), (1, S), (0, S), (2, C), (1, C), (0, C), (2, F), (1, F), (0, F)]
    );
    // Home starts as Excel does: Styles stacks, the labels beside Paste
    // go, then those of Editing and Cells.
    let home = sequence("1/fr", 7);
    assert_eq!(home[..4], [(4, S), (0, S), (6, C), (5, C)]);
    // Editing is the first group to fold and Clipboard the last of
    // Excel's list; Styles, which Excel never folds, comes after.
    let folds: Vec<usize> = home.iter().filter(|(_, mode)| *mode == F).map(|(group, _)| *group).collect();
    assert_eq!(folds, vec![6, 5, 3, 2, 1, 0, 4]);
    for tab in 1..=10 {
        for groups in [1, 4, 7, 12] {
            let steps = sequence(&format!("{tab}/en"), groups);
            // Every group ends folded, and only ever goes further.
            let mut reached = vec![Mode::Full; groups];
            for (group, mode) in &steps {
                assert!(*mode > reached[*group], "tab {tab}: {steps:?}");
                reached[*group] = *mode;
            }
            assert!(reached.iter().all(|mode| *mode == F), "tab {tab}: {steps:?}");
        }
    }
}

#[test]
fn a_plan_applies_its_steps_and_leaves_the_useless_ones_out() {
    let step = |group, mode, useless| Step {
        group,
        mode,
        need: 1000.0,
        useless,
    };
    let mut plan = Plan {
        groups: 3,
        larges: vec![Larges::default(); 3],
        steps: vec![step(2, S, false), step(1, S, true), step(2, C, false), step(0, F, false)],
    };
    assert_eq!(plan.modes(), vec![Mode::Folded, Mode::Full, Mode::Compact]);
    // The next step is the one after those taken, useless or not.
    assert_eq!(plan.next("x/en"), Some((1, C)));
    plan.steps.truncate(1);
    assert_eq!(plan.modes(), vec![Mode::Full, Mode::Full, Mode::Stacked]);
}

fn open(cx: &mut TestAppContext) -> (Entity<ExcelApp>, &mut VisualTestContext) {
    cx.update(crate::bind_keys);
    let (app, cx) = cx.add_window_view(ExcelApp::new);
    cx.run_until_parked();
    (app, cx)
}

/// Lay the window out at a width and say how the ribbon fitted.
fn fit_at(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, width: f32) -> Fit {
    cx.simulate_resize(size(px(width), px(820.)));
    // Twice: the element is sized from the layout before.
    for _ in 0..2 {
        app.update(cx, |app, cx| {
            // Only what this layout paints is looked at.
            app.anchors.borrow_mut().clear();
            cx.notify();
        });
        cx.run_until_parked();
        cx.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::none());
    }
    last_fit()
}

fn show_tab(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, tab: usize) {
    app.update(cx, |app, cx| {
        app.context_tab = None;
        app.ribbon_tab = tab;
        cx.notify();
    });
}

const WIDTHS: [f32; 6] = [1700.0, 1390.0, 1200.0, 1000.0, 820.0, 640.0];

/// Where a control was painted: left, top, right, bottom.
type Place = (f32, f32, f32, f32);

/// The controls painted by the last layout, by their name.
fn controls(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> BTreeMap<&'static str, Place> {
    app.update(cx, |app, _| {
        app.anchors
            .borrow()
            .iter()
            .map(|(id, b)| {
                let (x, y) = (f32::from(b.origin.x), f32::from(b.origin.y));
                (*id, (x, y, x + f32::from(b.size.width), y + f32::from(b.size.height)))
            })
            .collect()
    })
}

/// The controls on the ribbon itself (not in a drop-down, not elsewhere
/// in the window).
fn on_ribbon(fit: &Fit, all: &BTreeMap<&'static str, Place>) -> BTreeMap<&'static str, Place> {
    let (top, bottom) = (fit.origin.1 - 4.0, fit.origin.1 + fit.height + 4.0);
    all.iter().filter(|(_, place)| place.1 >= top && place.3 <= bottom).map(|(id, place)| (*id, *place)).collect()
}

/// The names of the controls in the drop-downs of the folded groups, each
/// opened in turn.
fn in_drop_downs(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, fit: &Fit, width: f32, what: &str) -> BTreeSet<&'static str> {
    let mut found = BTreeSet::new();
    let below = fit.origin.1 + fit.height;
    for (group, _) in fit.modes.iter().enumerate().filter(|(_, mode)| **mode == Mode::Folded) {
        toggle_group(group);
        let again = fit_at(app, cx, width);
        assert_eq!(open_group(), Some(group), "{what}: the group {group} opens");
        assert_eq!(again.modes, fit.modes, "{what}: opening a group changes nothing else");
        let inside: Vec<&'static str> = controls(app, cx).into_iter().filter(|(_, place)| place.1 > below).map(|(id, _)| id).collect();
        assert!(!inside.is_empty(), "{what}: the drop-down of group {group} ({}) is empty", fit.labels[group]);
        found.extend(inside);
        assert!(close_panel());
    }
    fit_at(app, cx, width);
    found
}

/// A control reports where its inside starts: past its own padding, up to
/// 7 pixels right of its edge for a combo box. Two neighbours can seem to
/// share that much.
const SHIFT: f32 = 8.0;

/// What the eye would check of the ribbon as the last layout painted it.
fn check_geometry(fit: &Fit, shown: &BTreeMap<&'static str, Place>, what: &str) {
    // Each group has its stretch of the ribbon, one after the other.
    let mut starts = vec![fit.origin.0];
    for width in &fit.widths {
        starts.push(starts.last().unwrap() + width);
    }
    // The controls stand over the name of their group, inside the ribbon
    // (three rows of buttons are 76 high in the 72 over the name, whose
    // text starts lower still).
    let (top, bottom) = (fit.origin.1 - 4.5, fit.origin.1 + fit.height - 16.0 + 4.5);
    for (id, place) in shown {
        assert!(place.1 >= top && place.3 <= bottom, "{what}: {id} is cut off or over the name of its group ({place:?}, room {top}..{bottom})");
        let group = starts.windows(2).position(|x| place.0 >= x[0] - 0.5 && place.2 <= x[1] + 0.5);
        assert!(group.is_some(), "{what}: {id} is not inside a group ({place:?}, groups start at {starts:?})");
        assert!(fit.modes[group.unwrap()] != Mode::Folded, "{what}: {id} shows in a folded group");
    }
    // No two controls share room (a control may hold another one whole:
    // the field of a combo box and its arrow).
    let places: Vec<(&&'static str, &Place)> = shown.iter().collect();
    for (index, (a_id, a)) in places.iter().enumerate() {
        for (b_id, b) in &places[index + 1..] {
            let (w, h) = (a.2.min(b.2) - a.0.max(b.0), a.3.min(b.3) - a.1.max(b.1));
            if w <= SHIFT || h <= 1.0 {
                continue;
            }
            let holds = |outer: &Place, inner: &Place| outer.0 <= inner.0 + 0.5 && outer.1 <= inner.1 + 0.5 && outer.2 >= inner.2 - 0.5 && outer.3 >= inner.3 - 0.5;
            assert!(holds(a, b) || holds(b, a), "{what}: {a_id} {a:?} and {b_id} {b:?} overlap");
        }
    }
}

/// The tab shown, at every width: it fits, it is sound, nothing is lost,
/// and it is whole again when there is room.
fn check_tab(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, what: &str) {
    let wide = fit_at(app, cx, 3200.0);
    assert!(wide.modes.iter().all(|mode| *mode == Mode::Full), "{what}: {:?}", wide.modes);
    let everything = on_ribbon(&wide, &controls(app, cx));
    check_geometry(&wide, &everything, &format!("{what} at 3200"));
    let mut least = wide.total;
    for width in WIDTHS {
        let what = format!("{what} at {width}");
        let fit = fit_at(app, cx, width);
        // In a very narrow window a tab may not fit even folded: then
        // every group has given what it can, and the ribbon scrolls.
        assert!(
            fit.total <= fit.available + 0.5 || (width < 1000.0 && fit.exhausted),
            "{what}: {} wide for {} ({:?})",
            fit.total,
            fit.available,
            fit.modes
        );
        assert!(fit.builds <= MAX_BUILDS_PER_GROUP * fit.modes.len() + 9, "{what}: {} builds", fit.builds);
        // A narrower window never makes the tab wider.
        assert!(fit.total <= least + 0.5, "{what}: {} after {least}", fit.total);
        least = fit.total;
        let shown = on_ribbon(&fit, &controls(app, cx));
        check_geometry(&fit, &shown, &what);
        // Every control of the wide ribbon is on the ribbon or in the
        // drop-down of a folded group.
        let folded = in_drop_downs(app, cx, &fit, width, &what);
        let lost: Vec<&&'static str> = everything.keys().filter(|id| !shown.contains_key(*id) && !folded.contains(*id)).collect();
        assert!(lost.is_empty(), "{what}: {lost:?} cannot be reached ({:?})", fit.modes);
    }
    // A second layout at the same width builds the tab once.
    assert_eq!(fit_at(app, cx, 640.0).builds, 1, "{what}");
    // Widening undoes every step.
    let wide = fit_at(app, cx, 3200.0);
    assert!(wide.modes.iter().all(|mode| *mode == Mode::Full), "{what}: {:?}", wide.modes);
    assert_eq!(on_ribbon(&wide, &controls(app, cx)), everything, "{what}: the wide ribbon is as it was");
}

#[gpui::test]
fn every_tab_is_sound_at_every_width_in_every_language(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    for lang in Lang::ALL {
        set_lang(lang);
        for tab in 1..=10 {
            show_tab(&app, cx, tab);
            check_tab(&app, cx, &format!("tab {tab} in {}", lang.code()));
        }
    }
    set_lang(Lang::En);
}

#[gpui::test]
fn contextual_tabs_are_sound_too(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    // A table (Table Design) and a shape (Shape Format).
    let shape = app.update(cx, |app, cx| {
        app.wb.set_cell_text(0, 0, 0, "Name").unwrap();
        app.wb.set_cell_text(0, 1, 0, "a").unwrap();
        app.wb.add_table(0, Rect::new(0, 0, 1, 0), true).unwrap();
        let shape = app.wb.add_shape(0, ShapeTool::Rectangle, Frame::new(300.0, 100.0, 120.0, 80.0)).unwrap();
        app.commit("setup", cx);
        app.select_cell(1, 0, cx);
        shape
    });
    let mut seen = 0;
    for lang in [Lang::Fr, Lang::De, Lang::En] {
        set_lang(lang);
        for object in [None, Some(shape)] {
            app.update(cx, |app, cx| {
                let id = object.map(|id| features::object_id(&features::drawings::Drawings, id));
                app.select_object(id, cx);
            });
            let tabs = app.update(cx, |app, _| features::context_tabs(app));
            assert!(!tabs.is_empty());
            for (_, tab) in tabs {
                app.update(cx, |app, cx| {
                    app.context_tab = Some(tab.id);
                    cx.notify();
                });
                check_tab(&app, cx, &format!("{} in {}", tab.id, lang.code()));
                seen += 1;
            }
        }
    }
    assert!(seen >= 6);
    set_lang(Lang::En);
}

/// The narrowest of the widths tried at which a group of the tab shown is
/// in a mode.
fn width_where(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, group: usize, mode: Mode) -> Option<f32> {
    let mut width = 1800.0;
    while width > 300.0 {
        if fit_at(app, cx, width).modes.get(group) == Some(&mode) {
            return Some(width);
        }
        width -= 20.0;
    }
    None
}

#[gpui::test]
fn large_buttons_restack_in_three_rows_before_they_lose_their_labels(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    set_lang(Lang::En);
    // Review ▸ Comments: five large buttons in a row.
    show_tab(&app, cx, 7);
    let comments = 3;
    let buttons = ["rv-comment", "rv-comment-del", "rv-comment-prev", "rv-comment-next", "rv-comment-show"];
    let places = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| -> Vec<Place> {
        let all = controls(app, cx);
        buttons.iter().map(|id| *all.get(id).unwrap_or_else(|| panic!("{id} is painted"))).collect()
    };
    let wide = fit_at(&app, cx, 3200.0);
    let full = places(&app, cx);
    // In full: one row of buttons 68 high.
    assert!(full.windows(2).all(|pair| pair[0].1 == pair[1].1 && pair[0].2 <= pair[1].0 + 0.5), "{full:?}");
    assert!(full.iter().all(|place| place.3 - place.1 > 60.0), "{full:?}");
    // Stacked: the first three one under the other, the last two in a
    // second column; each one row high and wide enough for its label.
    let width = width_where(&app, cx, comments, Mode::Stacked).expect("the group stacks before it goes compact");
    let fit = fit_at(&app, cx, width);
    let stacked = places(&app, cx);
    assert!(stacked.iter().all(|place| (place.3 - place.1 - 24.0).abs() < 0.5), "{stacked:?}");
    assert!(stacked[0].0 == stacked[1].0 && stacked[1].0 == stacked[2].0, "{stacked:?}");
    assert!(stacked[0].1 < stacked[1].1 && stacked[1].1 < stacked[2].1, "{stacked:?}");
    assert!(stacked[3].0 > stacked[0].0 && stacked[3].0 == stacked[4].0, "{stacked:?}");
    assert!(stacked[3].1 == stacked[0].1 && stacked[4].1 == stacked[1].1, "{stacked:?}");
    assert!(stacked.iter().all(|place| place.2 - place.0 > 60.0), "the labels are kept: {stacked:?}");
    assert!(fit.widths[comments] < wide.widths[comments] - 20.0, "{fit:?}");
    // Compact: the same three rows, the icons alone.
    let width = width_where(&app, cx, comments, Mode::Compact).expect("the group goes compact before it folds");
    let narrow = fit_at(&app, cx, width);
    let compact = places(&app, cx);
    assert!(compact.iter().all(|place| place.2 - place.0 < 34.0 && (place.3 - place.1 - 24.0).abs() < 0.5), "{compact:?}");
    assert!(compact[0].0 == compact[2].0 && compact[3].0 > compact[0].0 && compact[3].1 == compact[0].1, "{compact:?}");
    assert!(narrow.widths[comments] < fit.widths[comments] - 20.0, "{narrow:?}");
    // A group with a single large button keeps it (Accessibility; and
    // Clipboard keeps Paste while the labels beside it go).
    assert!(narrow.modes[1] == Mode::Full || narrow.widths[1] == wide.widths[1], "{narrow:?}");
    show_tab(&app, cx, 1);
    let wide = fit_at(&app, cx, 3200.0);
    let paste = controls(&app, cx)["rb-paste"];
    let cut = controls(&app, cx)["rb-cut"];
    let width = width_where(&app, cx, 0, Mode::Stacked).expect("Clipboard gives way");
    let fit = fit_at(&app, cx, width);
    let all = controls(&app, cx);
    assert_eq!((all["rb-paste"].2 - all["rb-paste"].0, all["rb-paste"].3 - all["rb-paste"].1), (paste.2 - paste.0, paste.3 - paste.1));
    // (Cut is as wide as Copy under it, which keeps its arrow.)
    assert!(all["rb-cut"].2 - all["rb-cut"].0 < cut.2 - cut.0 - 20.0, "Cut has lost its label: {:?} after {cut:?}", all["rb-cut"]);
    assert!(fit.widths[0] < wide.widths[0] - 40.0);
}

#[gpui::test]
fn groups_give_way_in_the_order_of_excel(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    set_lang(Lang::En);
    // Home: Styles (4) stacks first; Editing (6) folds first, while
    // Clipboard (0) and Font (1), on the left, are the last to fold.
    show_tab(&app, cx, 1);
    let mut width = fit_at(&app, cx, 3200.0).total + 24.0;
    let mut first_change = None;
    let mut first_fold = None;
    while width > 400.0 && first_fold.is_none() {
        let fit = fit_at(&app, cx, width);
        if first_change.is_none() {
            first_change = fit.modes.iter().position(|mode| *mode != Mode::Full);
        }
        first_fold = fit.modes.iter().position(|mode| *mode == Mode::Folded);
        width -= 10.0;
    }
    assert_eq!((first_change, first_fold), (Some(4), Some(6)));
    // View: Excel folds Macros and Show before it touches the views; a
    // group of one button (Macros, Window) is not worth folding.
    show_tab(&app, cx, 8);
    let wide = fit_at(&app, cx, 3200.0);
    let fit = fit_at(&app, cx, wide.total - 30.0);
    assert_eq!(fit.modes, vec![Mode::Full, Mode::Folded, Mode::Full, Mode::Full, Mode::Full], "{:?}", fit.labels);
}

#[gpui::test]
fn german_formulas_fold_at_1390_and_unfold(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    set_lang(Lang::De);
    show_tab(&app, cx, 5);
    let narrow = fit_at(&app, cx, 1390.0);
    assert!(narrow.total <= narrow.available + 0.5, "{narrow:?}");
    // It took steps to get there.
    assert!(narrow.modes.iter().any(|mode| *mode != Mode::Full), "{narrow:?}");
    // Narrower still: more steps, never fewer.
    let further = |fit: &Fit| fit.modes.iter().map(|mode| *mode as usize).sum::<usize>();
    let narrower = fit_at(&app, cx, 700.0);
    assert!(further(&narrower) > further(&narrow), "{narrower:?}");
    // Back at 1390 it looks as it did, and wide it is whole.
    let again = fit_at(&app, cx, 1390.0);
    assert_eq!(again.modes, narrow.modes);
    let wide = fit_at(&app, cx, 3200.0);
    assert!(wide.modes.iter().all(|mode| *mode == Mode::Full), "{wide:?}");
    set_lang(Lang::En);
}

#[gpui::test]
fn a_folded_group_opens_and_runs_its_commands(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    set_lang(Lang::En);
    show_tab(&app, cx, 3);
    // The Draw tab in a window too narrow for its gallery of pens.
    let fit = fit_at(&app, cx, 330.0);
    let pens = 1;
    assert_eq!(fit.modes.get(pens), Some(&Mode::Folded), "{fit:?}");
    let below_ribbon = crate::theme::TITLEBAR_H + crate::theme::TABS_H + crate::theme::RIBBON_H - 10.0;
    let pen = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| app.update(cx, |app, _| app.anchors.borrow().get("ink-pen-1").copied());
    let drawing = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| app.update(cx, |app, _| !features::ink::Cmd::Select.checked(app));
    // A click on the button of the group opens the group under it.
    assert_eq!(open_group(), None);
    toggle_group(pens);
    fit_at(&app, cx, 330.0);
    assert_eq!(open_group(), Some(pens));
    let button = pen(&app, cx).expect("the pen was painted");
    assert!(f32::from(button.origin.y) > below_ribbon, "{button:?}");
    // The pen in the drop-down is the pen of the ribbon: a click takes
    // it in hand, and the drop-down closes.
    assert!(!drawing(&app, cx));
    cx.simulate_click(button.center(), Modifiers::none());
    cx.run_until_parked();
    assert!(drawing(&app, cx));
    assert_eq!(open_group(), None);
    // Open again; Escape closes it (and only it: the pen stays in hand).
    toggle_group(pens);
    fit_at(&app, cx, 330.0);
    assert_eq!(open_group(), Some(pens));
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert_eq!(open_group(), None);
    assert!(drawing(&app, cx));
    // A press elsewhere closes it too.
    toggle_group(pens);
    fit_at(&app, cx, 330.0);
    assert_eq!(open_group(), Some(pens));
    cx.simulate_mouse_down(point(px(300.), px(700.)), gpui::MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(point(px(300.), px(700.)), gpui::MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    assert_eq!(open_group(), None);
    // Widening restores the group itself on the ribbon.
    let wide = fit_at(&app, cx, 1400.0);
    assert_eq!(wide.modes.get(pens), Some(&Mode::Full));
    let button = pen(&app, cx).unwrap();
    assert!(f32::from(button.origin.y) < below_ribbon, "{button:?}");
}
