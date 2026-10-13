//! A ribbon that folds: when the window is too narrow for a tab, its
//! groups give way one after the other, as in Excel, and come back in the
//! reverse order when the window widens.
//!
//! Nothing is known in advance of how wide a group is in a language: the
//! tab is built, its groups are measured, and while they do not fit one
//! more *step* is taken and the tab is built again, all within the layout
//! of one frame (what is painted is the first build that fits). The steps
//! of a tab are always the same, in this order:
//!
//! 1. the four compaction levels the tabs wrote by hand
//!    (`ExcelApp::ribbon_level`: Home drops labels, galleries shorten);
//! 2. then, group after group from the right: the group becomes *compact*
//!    (its large buttons become small ones without their label), then it
//!    is *folded* into one button that opens the whole group as a
//!    drop-down.
//!
//! A step that does not make its group narrower (a group of one button is
//! not helped by folding) is skipped. Each step remembers the width the
//! tab needed before it: it is undone once the window offers that much.
//! Only when every step is taken and the tab still does not fit does the
//! ribbon scroll sideways.
//!
//! The tabs know nothing of this. `ribbon::group` asks here how the group
//! it is building is shown ([`next_group`]); the helpers that build large
//! buttons ask whether their group is compact ([`compact`]). Groups are
//! told apart by their position in the tab, which is how the code of a
//! tab builds them: the content of a group, then the group.
//!
//! The drop-down of a folded group is the group itself, built as the tab
//! builds it (same commands, same state, same fields), handed over here
//! ([`next_group`] returns where to put it) and painted by [`panel_layer`]
//! over the rest of the window.

use crate::app::ExcelApp;
use crate::theme::{self, c};
use crate::view::icon;
use gpui::{
    AnyElement, AvailableSpace, Bounds, MouseButton, Pixels, WeakEntity, Window, canvas, div, point, prelude::*, px, size,
};
use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How a group is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Full,
    /// Large buttons are small and have lost their label.
    Compact,
    /// One button that opens the group as a drop-down.
    Folded,
}

/// The hand-written compaction levels of the tabs (`ribbon_level`).
pub const LEVELS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq)]
enum StepKind {
    Level,
    Compact(usize),
    Fold(usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Step {
    kind: StepKind,
    /// The width the tab needed before this step: what undoes it.
    need: f32,
    /// The step did not make the tab narrower: it is not applied.
    useless: bool,
}

/// The steps taken for one tab in one language, in order.
#[derive(Clone, Debug, Default, PartialEq)]
struct Plan {
    /// How many groups the tab had when the steps were taken.
    groups: usize,
    steps: Vec<Step>,
}

impl Plan {
    fn level(&self) -> u8 {
        self.steps.iter().filter(|step| step.kind == StepKind::Level).count() as u8
    }

    fn modes(&self) -> Vec<Mode> {
        let mut modes = vec![Mode::Full; self.groups];
        for step in self.steps.iter().filter(|step| !step.useless) {
            match step.kind {
                StepKind::Compact(group) if group < modes.len() => modes[group] = Mode::Compact,
                StepKind::Fold(group) if group < modes.len() => modes[group] = Mode::Folded,
                _ => {}
            }
        }
        modes
    }

    /// The step after those taken: the levels, then compact and fold for
    /// each group from the last to the first.
    fn next(&self) -> Option<StepKind> {
        let taken = self.steps.len();
        if taken < LEVELS {
            return Some(StepKind::Level);
        }
        let after = taken - LEVELS;
        let group = self.groups.checked_sub(after / 2 + 1)?;
        Some(if after % 2 == 0 { StepKind::Compact(group) } else { StepKind::Fold(group) })
    }
}

/// What the last layout of the ribbon came to (for tests, and for the
/// sideways scroll of the last resort).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fit {
    pub key: String,
    /// Width the window offers the groups.
    pub available: f32,
    /// Width the groups take as they are shown.
    pub total: f32,
    pub level: u8,
    pub modes: Vec<Mode>,
    /// How many times the tab was built to get there.
    pub builds: usize,
}

/// The tab being built.
struct Build {
    key: String,
    modes: Vec<Mode>,
    /// How many groups were built so far: the position of the group whose
    /// content is being built.
    index: usize,
    /// The first icon a button of that group asked for: the icon of the
    /// group once folded.
    icon: Option<&'static str>,
}

#[derive(Default)]
struct State {
    plans: HashMap<String, Plan>,
    build: Option<Build>,
    /// The folded group whose drop-down is open: tab and position.
    open: Option<(String, usize)>,
    /// The drop-down a press outside it just closed, so that the click of
    /// that press on its own button does not open it again.
    closed: Option<(usize, Instant)>,
    /// Where the button of the open group was painted.
    button: Option<Bounds<Pixels>>,
    /// The drop-down of the open group, built with the tab and waiting to
    /// be painted over the window.
    panel: Option<AnyElement>,
    app: Option<WeakEntity<ExcelApp>>,
    fit: Fit,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|state| f(&mut state.borrow_mut()))
}

/// What the last layout of the ribbon came to.
#[cfg(test)]
pub fn last_fit() -> Fit {
    with(|state| state.fit.clone())
}

/// The position of the folded group whose drop-down is open.
#[cfg(test)]
pub fn open_group() -> Option<usize> {
    with(|state| state.open.as_ref().map(|(_, group)| *group))
}

/// Close the drop-down of a folded group; false when none was open.
pub fn close_panel() -> bool {
    with(|state| {
        state.panel = None;
        state.open.take().is_some()
    })
}

/// Open or close the drop-down of the folded group at a position of the
/// tab shown (what a click on its button does).
pub fn toggle_group(group: usize) {
    with(|state| {
        let key = state.fit.key.clone();
        let just_closed = state.closed.take().is_some_and(|(closed, at)| closed == group && at.elapsed() < Duration::from_millis(400));
        state.open = match &state.open {
            Some((_, open)) if *open == group => None,
            _ if just_closed => None,
            _ => Some((key, group)),
        };
    });
}

// ------------------------------------------------- asked while a tab is built

/// True while the content of a compact group is being built: its large
/// buttons are made small, without their label.
pub fn compact() -> bool {
    with(|state| state.build.as_ref().is_some_and(|build| build.modes.get(build.index) == Some(&Mode::Compact)))
}

/// A button of the group being built shows this icon.
pub fn saw_icon(name: &'static str) {
    with(|state| {
        if let Some(build) = state.build.as_mut() {
            build.icon.get_or_insert(name);
        }
    });
}

/// How `ribbon::group` shows the group it was just handed.
pub struct Slot {
    pub mode: Mode,
    /// Position of the group in its tab.
    pub index: usize,
    pub icon: &'static str,
    /// Folded, with its drop-down open: the group is wanted for it
    /// ([`show_panel`]).
    pub open: bool,
}

/// The content of a group is built: say how the group is shown, and move
/// on to the next one.
pub fn next_group() -> Slot {
    with(|state| {
        let open = state.open.clone();
        let Some(build) = state.build.as_mut() else {
            // Not the ribbon (a test building a group by hand).
            return Slot {
                mode: Mode::Full,
                index: 0,
                icon: "more",
                open: false,
            };
        };
        let index = build.index;
        build.index += 1;
        let icon = build.icon.take().unwrap_or("more");
        let mode = build.modes.get(index).copied().unwrap_or_default();
        Slot {
            mode,
            index,
            icon,
            open: mode == Mode::Folded && open.is_some_and(|(key, group)| key == build.key && group == index),
        }
    })
}

/// Hand over the drop-down of the open folded group: the group as the tab
/// built it.
pub fn show_panel(group: impl IntoElement) {
    let panel = div()
        .id("ribbon-fold-panel")
        .occlude()
        .h(px(theme::RIBBON_H - 8.0))
        .flex()
        .bg(c(theme::PANEL_BG))
        .border_1()
        .border_color(c(theme::BORDER_STRONG))
        .rounded(px(8.))
        .shadow_lg()
        .py(px(3.))
        .text_size(px(12.))
        .text_color(c(theme::TEXT))
        .child(group)
        // A press anywhere else closes it, as it closes a menu.
        .on_mouse_down_out(|_, window, _| {
            with(|state| {
                if let Some((_, group)) = state.open.take() {
                    state.closed = Some((group, Instant::now()));
                }
            });
            window.refresh();
        })
        // So does a click on one of its buttons, once the command ran:
        // unless the click opened a menu or went into a field.
        .on_mouse_up(MouseButton::Left, |_, window, cx| {
            window.defer(cx, |window, cx| {
                let app = with(|state| state.app.clone()).and_then(|app| app.upgrade());
                let stays = app.is_some_and(|app| {
                    let app = app.read(cx);
                    app.popup.is_some() || !app.grid_focus.is_focused(window)
                });
                if !stays && close_panel() {
                    window.refresh();
                }
            });
        })
        .into_any_element();
    with(|state| state.panel = Some(panel));
}

/// The button a folded group becomes: its icon over its name and a caret.
pub fn folded_button(label: &'static str, slot: &Slot) -> gpui::Stateful<gpui::Div> {
    let (line1, line2) = crate::ribbon::split_label(label);
    let index = slot.index;
    let open = slot.open;
    div()
        .id(("ribbon-fold", index))
        .relative()
        .h(px(68.))
        .min_w(px(44.))
        .px(px(5.))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(2.))
        .pt(px(4.))
        .rounded(px(4.))
        .text_color(c(theme::TEXT))
        .when(open, |d| d.bg(c(theme::TOGGLED)))
        .hover(|s| s.bg(c(theme::HOVER)))
        .active(|s| s.bg(c(theme::PRESSED)))
        .child(icon(slot.icon, 26.0, theme::TEXT))
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .text_size(px(11.))
                .line_height(px(13.))
                .whitespace_nowrap()
                .child(line1.to_string())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(2.))
                        .child(line2.to_string())
                        .child(icon("chevron-down", 8.0, theme::TEXT_MUTED)),
                ),
        )
        // Where the drop-down hangs from.
        .when(open, |d| {
            d.child(
                canvas(|bounds, _, _| with(|state| state.button = Some(bounds)), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
        })
        .on_click(move |_, window, _| {
            toggle_group(index);
            window.refresh();
        })
}

// ------------------------------------------------------------------ fitting

/// Build the tab as a plan says and measure its groups.
fn build(
    app: &WeakEntity<ExcelApp>,
    key: &str,
    plan: &Plan,
    height: Pixels,
    window: &mut Window,
    cx: &mut gpui::App,
) -> (Vec<(AnyElement, Pixels)>, f32) {
    with(|state| {
        state.panel = None;
        state.build = Some(Build {
            key: key.to_string(),
            modes: plan.modes(),
            index: 0,
            icon: None,
        });
    });
    let level = plan.level();
    let groups = app.update(cx, |app, cx| app.ribbon_groups(level, window, cx)).unwrap_or_default();
    with(|state| state.build = None);
    let space = size(AvailableSpace::MinContent, AvailableSpace::Definite(height));
    let mut total = 0.0;
    let measured = groups
        .into_iter()
        .map(|group| {
            // The groups fill the height of the ribbon.
            let mut element = div().h(height).flex().flex_none().child(group).into_any_element();
            let width = element.layout_as_root(space, window, cx).width;
            total += f32::from(width);
            (element, width)
        })
        .collect();
    (measured, total)
}

/// The groups of the tab shown, fitted to `available` pixels: an element
/// that takes the width and the height it is given.
pub fn fitted(app: WeakEntity<ExcelApp>, key: String, available: f32) -> impl IntoElement {
    canvas(
        move |bounds: Bounds<Pixels>, window, cx| {
            let height = bounds.size.height;
            let mut plan = with(|state| {
                state.app = Some(app.clone());
                // Another tab: its drop-down goes with it.
                if state.open.as_ref().is_some_and(|(open, _)| *open != key) {
                    state.open = None;
                }
                state.plans.get(&key).cloned().unwrap_or_default()
            });
            // A step just taken, with the width before it: if it gains
            // nothing it is marked and left out.
            let mut trial: Option<f32> = None;
            let mut took_step = false;
            let mut builds = 0;
            let (groups, total) = loop {
                let (groups, total) = build(&app, &key, &plan, height, window, cx);
                builds += 1;
                if groups.len() != plan.groups {
                    // The tab changed (a group came or went): start again.
                    plan = Plan {
                        groups: groups.len(),
                        steps: Vec::new(),
                    };
                    trial = None;
                    if builds < 3 {
                        continue;
                    }
                }
                if let Some(before) = trial.take() {
                    let last = plan.steps.last_mut().expect("a step was just taken");
                    if last.kind != StepKind::Level && total >= before - 0.5 {
                        last.useless = true;
                        // What was built shows the step: build without it.
                        continue;
                    }
                }
                let fits = total <= available + 0.5;
                // The builds are bounded whatever happens.
                if builds > 2 * plan.groups + LEVELS + 8 {
                    break (groups, total);
                }
                if !fits {
                    match plan.next() {
                        Some(kind) => {
                            plan.steps.push(Step {
                                kind,
                                need: total,
                                useless: false,
                            });
                            trial = Some(total);
                            took_step = true;
                            continue;
                        }
                        // Every step is taken: the ribbon scrolls.
                        None => break (groups, total),
                    }
                }
                // There is room: the last step is undone when the window
                // offers what the tab needed before it.
                match plan.steps.last() {
                    Some(step) if !took_step && step.need <= available + 0.5 => {
                        plan.steps.pop();
                        continue;
                    }
                    _ => break (groups, total),
                }
            };
            let modes = plan.modes();
            let level = plan.level();
            // The element was sized for the last layout: when the width to
            // scroll changes, the window is laid out again.
            if (total > available + 0.5 || last_fit_width(available) > available) && (last_fit_width(available) - total.max(available)).abs() > 0.5 {
                window.refresh();
            }
            with(|state| {
                // A group that is no longer folded has no drop-down.
                if state.open.as_ref().is_some_and(|(_, group)| modes.get(*group) != Some(&Mode::Folded)) {
                    state.open = None;
                    state.panel = None;
                }
                state.fit = Fit {
                    key: key.clone(),
                    available,
                    total,
                    level,
                    modes,
                    builds,
                };
                state.plans.insert(key.clone(), plan);
            });
            let mut x = bounds.origin.x;
            let mut placed = Vec::with_capacity(groups.len());
            for (mut element, width) in groups {
                element.prepaint_at(point(x, bounds.origin.y), window, cx);
                x += width;
                placed.push(element);
            }
            placed
        },
        |_, groups: Vec<AnyElement>, window, cx| {
            for mut group in groups {
                group.paint(window, cx);
            }
        },
    )
    .h_full()
    .flex_none()
    // Wide enough to scroll when the tab does not fit even folded.
    .w(px(last_fit_width(available)))
}

/// The width the ribbon's content takes: the window's, or more when the
/// last layout could not fit the tab.
fn last_fit_width(available: f32) -> f32 {
    with(|state| if state.fit.total > available + 0.5 { state.fit.total } else { available })
}

/// The drop-down of the open folded group, painted over the window: put
/// it after everything it must cover.
pub fn panel_layer() -> impl IntoElement {
    canvas(
        |_, window, cx| {
            let (panel, button) = with(|state| (state.panel.take(), state.button));
            let (mut panel, button) = (panel?, button?);
            let space = size(AvailableSpace::MinContent, AvailableSpace::MinContent);
            let width = panel.layout_as_root(space, window, cx).width;
            // Under its button, and inside the window.
            let room = window.viewport_size().width - width - px(6.);
            let x = button.origin.x.min(room).max(px(6.));
            let y = button.origin.y + button.size.height + px(6.);
            panel.prepaint_at(point(x, y), window, cx);
            Some(panel)
        },
        |_, panel: Option<AnyElement>, window, cx| {
            if let Some(mut panel) = panel {
                panel.paint(window, cx);
            }
        },
    )
    .absolute()
    .size_0()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_come_in_the_same_order() {
        let mut plan = Plan {
            groups: 3,
            steps: Vec::new(),
        };
        let mut order = Vec::new();
        while let Some(kind) = plan.next() {
            order.push(kind);
            plan.steps.push(Step {
                kind,
                need: 1000.0 - order.len() as f32,
                useless: false,
            });
        }
        use StepKind::*;
        assert_eq!(
            order,
            vec![Level, Level, Level, Level, Compact(2), Fold(2), Compact(1), Fold(1), Compact(0), Fold(0)]
        );
        assert_eq!(plan.level(), 4);
        assert_eq!(plan.modes(), vec![Mode::Folded; 3]);
        // Undone in the reverse order; a useless step is not applied.
        plan.steps.truncate(7);
        plan.steps[5].useless = true;
        assert_eq!(plan.modes(), vec![Mode::Full, Mode::Compact, Mode::Compact]);
        plan.steps.truncate(2);
        assert_eq!((plan.level(), plan.modes()), (2, vec![Mode::Full; 3]));
    }

    use crate::features::{self, Command};
    use crate::i18n::{Lang, set_lang};
    use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
    use opensheet_engine::Rect;
    use opensheet_engine::drawing::{Frame, ShapeTool};

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
            app.update(cx, |_, cx| cx.notify());
            cx.run_until_parked();
            cx.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::none());
        }
        last_fit()
    }

    const WIDTHS: [f32; 6] = [1700.0, 1390.0, 1200.0, 1000.0, 820.0, 640.0];

    /// The tab shown fits at every width, and is whole when there is room.
    fn check_tab(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, what: &str) {
        for width in WIDTHS {
            let fit = fit_at(app, cx, width);
            let fits = fit.total <= fit.available + 0.5;
            // In a very narrow window a tab may not fit even folded: then
            // every group has given what it can, and the ribbon scrolls.
            let exhausted = width < 1000.0 && fit.level as usize == LEVELS && fit.modes.iter().all(|mode| *mode != Mode::Full);
            assert!(
                fits || exhausted,
                "{what} at {width}: {} wide for {} ({:?})",
                fit.total,
                fit.available,
                fit.modes
            );
            assert!(fit.builds <= 2 * fit.modes.len() + LEVELS + 8, "{what} at {width}: {} builds", fit.builds);
        }
        // A second layout at the same width builds the tab once.
        assert_eq!(fit_at(app, cx, 640.0).builds, 1, "{what}");
        // Widening undoes every step.
        let wide = fit_at(app, cx, 3200.0);
        assert_eq!(wide.level, 0, "{what}");
        assert!(wide.modes.iter().all(|mode| *mode == Mode::Full), "{what}: {:?}", wide.modes);
    }

    #[gpui::test]
    fn every_tab_fits_in_every_language(cx: &mut TestAppContext) {
        let (app, cx) = open(cx);
        for lang in Lang::ALL {
            set_lang(lang);
            for tab in 1..=10 {
                app.update(cx, |app, cx| {
                    app.ribbon_tab = tab;
                    cx.notify();
                });
                check_tab(&app, cx, &format!("tab {tab} in {}", lang.code()));
            }
        }
        set_lang(Lang::En);
    }

    #[gpui::test]
    fn contextual_tabs_fit_too(cx: &mut TestAppContext) {
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

    #[gpui::test]
    fn german_formulas_fold_at_1390_and_unfold(cx: &mut TestAppContext) {
        let (app, cx) = open(cx);
        set_lang(Lang::De);
        app.update(cx, |app, cx| {
            app.ribbon_tab = 5;
            cx.notify();
        });
        let narrow = fit_at(&app, cx, 1390.0);
        assert!(narrow.total <= narrow.available + 0.5, "{narrow:?}");
        // It took steps to get there, and the groups on the left are whole.
        assert!(narrow.level > 0 || narrow.modes.iter().any(|mode| *mode != Mode::Full), "{narrow:?}");
        assert_eq!(narrow.modes.first(), Some(&Mode::Full), "{narrow:?}");
        // Narrower still: more steps, never fewer.
        let folded = |fit: &Fit| fit.modes.iter().filter(|mode| **mode == Mode::Folded).count();
        let narrower = fit_at(&app, cx, 900.0);
        assert!(folded(&narrower) >= folded(&narrow) && folded(&narrower) > 0, "{narrower:?}");
        // Back at 1390 it looks as it did, and wide it is whole.
        let again = fit_at(&app, cx, 1390.0);
        assert_eq!((again.level, &again.modes), (narrow.level, &narrow.modes));
        let wide = fit_at(&app, cx, 3200.0);
        assert!(wide.level == 0 && wide.modes.iter().all(|mode| *mode == Mode::Full), "{wide:?}");
        set_lang(Lang::En);
    }

    #[gpui::test]
    fn a_folded_group_opens_and_runs_its_commands(cx: &mut TestAppContext) {
        let (app, cx) = open(cx);
        set_lang(Lang::En);
        app.update(cx, |app, cx| {
            app.ribbon_tab = 3;
            cx.notify();
        });
        // The Draw tab in a window too narrow for its gallery of pens.
        let fit = fit_at(&app, cx, 330.0);
        let pens = 1;
        assert_eq!(fit.modes.get(pens), Some(&Mode::Folded), "{fit:?}");
        // A group of one button is not folded: that would not be narrower.
        assert_eq!(fit.modes.get(2), Some(&Mode::Compact), "{fit:?}");
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
}
