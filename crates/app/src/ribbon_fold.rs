//! A ribbon that folds: when the window is too narrow for a tab, its
//! groups give way one after the other, as in Excel, and come back in the
//! reverse order when the window widens.
//!
//! Nothing is known in advance of how wide a group is in a language: the
//! tab is built, its groups are measured, and while they do not fit one
//! more *step* is taken and the tab is built again, all within the layout
//! of one frame (what is painted is the first build that fits).
//!
//! A step takes one group one stage further, as Excel for Mac does:
//!
//! 1. *stacked*: the large buttons that stand side by side in a row
//!    become small ones that keep their label, and the row stacks them
//!    in three rows (a large button alone in its group stays as it is,
//!    as Excel keeps Paste: the small buttons around it give way);
//! 2. *compact*: the same without the labels;
//! 3. *folded*: one button that opens the whole group as a drop-down.
//!
//! Which group gives way first is Excel's own order for each tab
//! ([`excel_order`], read from the scaling lists of Excel for Mac's
//! ribbon, `Excel.ribbonb`); a tab Excel does not have, and what Excel's
//! list leaves out, go stage by stage from the right. Only the order is
//! tabulated: a step that does not make its group narrower in the
//! language shown is skipped, and each step remembers the width the tab
//! needed before it, so that it is undone once the window offers that
//! much. Only when every step is taken and the tab still does not fit
//! does the ribbon scroll sideways.
//!
//! The tabs know nothing of this. `ribbon::group` asks here how the group
//! it is building is shown ([`next_group`]); the helpers that build large
//! buttons ask which form they take ([`large_form`]), `ribbon::row`
//! says which large buttons it holds ([`row_of_larges`]) and learns with
//! `ribbon::sep` whether its group is restacked, and the few groups
//! that shorten a gallery or drop the labels of small buttons by
//! themselves ask how far their group has given way ([`level`]). Groups
//! are told apart by their position in the tab, which is how the code of
//! a tab builds them: the content of a group, then the group.
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
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// How a group is shown. Each stage is narrower than the one before.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mode {
    #[default]
    Full,
    /// Large buttons are small ones with their label, in three rows.
    Stacked,
    /// The same without the labels.
    Compact,
    /// One button that opens the group as a drop-down.
    Folded,
}

/// The form a large button takes in the group being built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LargeForm {
    Large,
    /// One row: icon and label.
    Labeled,
    /// The icon alone (the tip keeps the name).
    Icon,
}

use Mode::{Compact as C, Folded as F, Stacked as S};

/// The order in which the groups of a tab give way in Excel for Mac:
/// (position of the group in our tab, the stage it goes to). Read from
/// the scaling list of each tab in Excel's `Excel.ribbonb`, where a group
/// goes from layout size 3 to 2, 1 and then 0 (one button); Excel's
/// groups are matched to ours by what they hold, and those Excel has and
/// we have not are left out.
fn excel_order(tab: &str) -> &'static [(usize, Mode)] {
    match tab {
        // Home: Clipboard 0, Font 1, Alignment 2, Number 3, Styles 4,
        // Cells 5, Editing 6. Styles stacks its three buttons first, the
        // labels beside Paste go, then those of Editing, Cells and
        // Alignment; Editing is the first to fold, Clipboard the last.
        "1" => &[(4, S), (0, S), (6, C), (5, C), (2, S), (4, C), (6, F), (5, F), (3, F), (2, F), (1, F), (0, F)],
        // Insert: Tables 0, Illustrations 1, Charts 2, PivotChart 3,
        // Sparklines 4, Filters 5, Links 6, Comments 7, Text 8, Symbols 9.
        "2" => &[(8, S), (9, S), (1, S), (2, S), (5, S), (8, C), (9, C), (1, C), (2, C), (8, F), (9, F), (1, F), (2, F), (0, F)],
        // Draw: Drawing Tools 0, Pens 1, Convert 2, Replay 3.
        "3" => &[(1, S), (0, S), (2, S), (1, C), (0, F), (2, F)],
        // Page Layout: Themes 0, Page Setup 1, Scale to Fit 2, Sheet
        // Options 3, Arrange 4. Excel for Mac has no Arrange group on
        // this tab: ours gives way first, and the rest follows Excel,
        // where Page Setup keeps its large buttons the longest.
        "4" => &[(4, S), (4, C), (4, F), (0, S), (0, C), (0, F), (3, F), (2, F), (1, S), (1, C)],
        // Formulas: Function Library 0, Defined Names 1, Formula
        // Auditing 2, Calculation 3.
        "5" => &[(3, S), (0, S), (2, S), (0, C), (1, F), (2, F), (0, F)],
        // Data: Get & Transform Data 0, Sort & Filter 1, Data Tools 2,
        // Forecast 3, Outline 4.
        "6" => &[(0, S), (4, S), (2, S), (4, C), (0, C), (2, C), (4, F), (1, S), (2, F), (1, F)],
        // Review: Proofing 0, Accessibility 1, Language 2, Comments 3,
        // Notes 4, Protect 5.
        "7" => &[(3, S), (4, S), (0, S), (3, C), (4, C), (5, S), (5, F), (1, F), (3, F), (4, F), (0, F)],
        // View: Workbook Views 0, Show 1, Zoom 2, Window 3, Macros 4.
        "8" => &[(4, F), (1, F), (0, S), (3, F), (0, F), (2, F)],
        // Automate: Scripting Tools 0, Office Scripts 1.
        "9" => &[(1, S), (1, C), (1, F), (0, F)],
        _ => &[],
    }
}

/// Every step a tab can take, in order: Excel's for the tab, then what
/// is left, stage by stage from the right.
fn sequence(key: &str, groups: usize) -> Vec<(usize, Mode)> {
    let tab = key.split('/').next().unwrap_or(key);
    let mut steps: Vec<(usize, Mode)> = Vec::new();
    let mut reached = vec![Mode::Full; groups];
    let mut take = |group: usize, mode: Mode, steps: &mut Vec<(usize, Mode)>| {
        // A group only ever goes further.
        if group < groups && mode > reached[group] {
            reached[group] = mode;
            steps.push((group, mode));
        }
    };
    for (group, mode) in excel_order(tab) {
        take(*group, *mode, &mut steps);
    }
    for mode in [S, C, F] {
        for group in (0..groups).rev() {
            take(group, mode, &mut steps);
        }
    }
    steps
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Step {
    group: usize,
    mode: Mode,
    /// The width the tab needed before this step: what undoes it.
    need: f32,
    /// The step did not make the tab narrower: it is not applied.
    useless: bool,
}

/// What folding a group must gain, in pixels, to be done at all.
const MIN_FOLD_GAIN: f32 = 12.0;

/// Three stages a group, each built at most twice (once to try it, once
/// more without it when it gains nothing).
const MAX_BUILDS_PER_GROUP: usize = 6;

/// The large buttons of a group, numbered as the group builds them.
#[derive(Clone, Debug, Default, PartialEq)]
struct Larges {
    count: usize,
    /// Those a row holds together with at least another one: the ones
    /// that are restacked.
    in_rows: Vec<usize>,
}

/// The steps taken for one tab in one language, in order.
#[derive(Clone, Debug, Default, PartialEq)]
struct Plan {
    /// How many groups the tab had when the steps were taken.
    groups: usize,
    /// The large buttons each group builds.
    larges: Vec<Larges>,
    steps: Vec<Step>,
}

impl Plan {
    fn modes(&self) -> Vec<Mode> {
        let mut modes = vec![Mode::Full; self.groups];
        for step in self.steps.iter().filter(|step| !step.useless) {
            if let Some(mode) = modes.get_mut(step.group) {
                *mode = step.mode;
            }
        }
        modes
    }

    /// The step after those taken.
    fn next(&self, key: &str) -> Option<(usize, Mode)> {
        sequence(key, self.groups).get(self.steps.len()).copied()
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
    pub modes: Vec<Mode>,
    /// The width of each group as it is shown.
    pub widths: Vec<f32>,
    /// Where the first group starts in the window, and how high the
    /// groups are.
    pub origin: (f32, f32),
    pub height: f32,
    /// The names of the groups, as the tab gave them.
    pub labels: Vec<&'static str>,
    /// No step is left to take.
    pub exhausted: bool,
    /// How many times the tab was built to get there.
    pub builds: usize,
}

/// The tab being built.
struct Build {
    key: String,
    modes: Vec<Mode>,
    /// The large buttons of each group, as the build before found them.
    larges: Vec<Larges>,
    /// Those found in this build, group after group, and in the group
    /// whose content is being built.
    counted: Vec<Larges>,
    found: Larges,
    labels: Vec<&'static str>,
    /// The rows of large buttons of the group and its separators: what
    /// is told when the group restacks.
    rows: Vec<Rc<Cell<bool>>>,
    seps: Vec<Rc<Cell<bool>>>,
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

/// The form a large button takes in the group whose content is being
/// built, and its number in the group (for [`row_of_larges`]).
pub fn large_form() -> (LargeForm, usize) {
    with(|state| {
        let Some(build) = state.build.as_mut() else {
            return (LargeForm::Large, 0);
        };
        let number = build.found.count;
        build.found.count += 1;
        let known = build.larges.get(build.index);
        let in_row = known.is_some_and(|known| known.in_rows.contains(&number));
        let several = known.is_some_and(|known| known.count > 1);
        let form = match build.modes.get(build.index) {
            Some(Mode::Stacked) if in_row => LargeForm::Labeled,
            // Large buttons that no row holds lose their label where
            // they stand.
            Some(Mode::Compact) if in_row || several => LargeForm::Icon,
            _ => LargeForm::Large,
        };
        (form, number)
    })
}

/// How far the group being built has given way, for the groups that
/// shorten a gallery or drop the labels of their small buttons by
/// themselves: 0 in full, 2 stacked, 4 compact. (A folded group opens in
/// full.)
pub fn level() -> u8 {
    with(|state| {
        let mode = state.build.as_ref().and_then(|build| build.modes.get(build.index).copied());
        match mode {
            Some(Mode::Stacked) => 2,
            Some(Mode::Compact) => 4,
            _ => 0,
        }
    })
}

/// A row that holds several large buttons (their numbers) was built: the
/// flag that will tell it, once its group is known, to stack its
/// controls in three rows instead.
pub fn row_of_larges(numbers: &[usize]) -> Rc<Cell<bool>> {
    let flag = Rc::new(Cell::new(false));
    with(|state| {
        if let Some(build) = state.build.as_mut() {
            build.found.in_rows.extend_from_slice(numbers);
            build.rows.push(flag.clone());
        }
    });
    flag
}

/// A separator was built: the flag that hides it when its group is
/// restacked (a line between two controls has no place between rows).
pub fn sep_built() -> Rc<Cell<bool>> {
    let flag = Rc::new(Cell::new(false));
    with(|state| {
        if let Some(build) = state.build.as_mut() {
            build.seps.push(flag.clone());
        }
    });
    flag
}

/// A button of the group being built shows this icon.
pub fn saw_icon(name: &'static str) {
    // The placeholder of a command without an icon says nothing of a group.
    if name == "more" {
        return;
    }
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
pub fn next_group(label: &'static str) -> Slot {
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
        let mode = build.modes.get(index).copied().unwrap_or_default();
        // A group that gives way stacks its rows of large buttons in
        // three rows (they were built small, on what the build before
        // found: only then does it apply).
        let rows = std::mem::take(&mut build.rows);
        let seps = std::mem::take(&mut build.seps);
        let found = std::mem::take(&mut build.found);
        let as_built = build.larges.get(index) == Some(&found);
        if as_built && !rows.is_empty() && matches!(mode, Mode::Stacked | Mode::Compact) {
            for flag in rows.iter().chain(&seps) {
                flag.set(true);
            }
        }
        build.counted.push(found);
        build.labels.push(label);
        build.index += 1;
        let icon = build.icon.take().unwrap_or("more");
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
) -> (Vec<(AnyElement, Pixels)>, f32, Vec<Larges>, Vec<&'static str>) {
    with(|state| {
        state.panel = None;
        state.build = Some(Build {
            key: key.to_string(),
            modes: plan.modes(),
            larges: plan.larges.clone(),
            counted: Vec::new(),
            found: Larges::default(),
            labels: Vec::new(),
            rows: Vec::new(),
            seps: Vec::new(),
            index: 0,
            icon: None,
        });
    });
    let groups = app.update(cx, |app, cx| app.ribbon_groups(window, cx)).unwrap_or_default();
    let (counted, labels) = with(|state| state.build.take().map(|build| (build.counted, build.labels))).unwrap_or_default();
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
    (measured, total, counted, labels)
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
            let (groups, total, labels) = loop {
                let (groups, total, counted, labels) = build(&app, &key, &plan, height, window, cx);
                builds += 1;
                if groups.len() != plan.groups {
                    // The tab changed (a group came or went): start again.
                    plan = Plan {
                        groups: groups.len(),
                        larges: counted,
                        steps: Vec::new(),
                    };
                    trial = None;
                    if builds < 3 {
                        continue;
                    }
                } else if counted != plan.larges {
                    // What the groups hold changed: they were built on
                    // what they held before.
                    plan.larges = counted;
                    if !plan.steps.is_empty() && builds < 3 {
                        continue;
                    }
                }
                if let Some(before) = trial.take() {
                    let last = plan.steps.last_mut().expect("a step was just taken");
                    // Folding hides a group: it has to be worth it (a
                    // group of one button gains nothing to speak of).
                    let gain = if last.mode == Mode::Folded { MIN_FOLD_GAIN } else { 0.5 };
                    if total > before - gain {
                        last.useless = true;
                        // What was built shows the step: build without it.
                        continue;
                    }
                }
                let fits = total <= available + 0.5;
                // The builds are bounded whatever happens.
                if builds > MAX_BUILDS_PER_GROUP * plan.groups + 8 {
                    break (groups, total, labels);
                }
                if !fits {
                    match plan.next(&key) {
                        Some((group, mode)) => {
                            plan.steps.push(Step {
                                group,
                                mode,
                                need: total,
                                useless: false,
                            });
                            trial = Some(total);
                            took_step = true;
                            continue;
                        }
                        // Every step is taken: the ribbon scrolls.
                        None => break (groups, total, labels),
                    }
                }
                // There is room: the last step is undone when the window
                // offers what the tab needed before it.
                match plan.steps.last() {
                    Some(step) if !took_step && step.need <= available + 0.5 => {
                        plan.steps.pop();
                        continue;
                    }
                    _ => break (groups, total, labels),
                }
            };
            let modes = plan.modes();
            let exhausted = plan.next(&key).is_none();
            let widths = groups.iter().map(|(_, width)| f32::from(*width)).collect();
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
                    modes,
                    widths,
                    origin: (f32::from(bounds.origin.x), f32::from(bounds.origin.y)),
                    height: f32::from(height),
                    labels,
                    exhausted,
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
mod tests;
