//! The Sort dialog: levels (key, what is compared, order), "My data has
//! headers", its options (case, orientation) and the custom lists; and the
//! warning asked when one column of a block of data is selected.

use super::{
    APP_NAME, Cmd, SortPlan, SortTarget, State, cannot_apply, cells_kind, column_title, select_keeping_active,
    shown_colors, single_selection, sort_failed, sort_plan, straddles_filter,
};
use crate::app::ExcelApp;
use crate::features::{Command, Cx, Intent};
use crate::i18n::{self, t};
use crate::shell::{Alert, Modal};
use crate::theme::{self, c};
use crate::widgets::{
    self, Button, CONTROL_H, Checkbox, GroupBox, ListBox, ROW_H, RadioGroup, ScrollArea, Select, dialog, icon, label,
    ok_cancel, tooltip,
};
use gpui::{
    AnyElement, Context, FontWeight, MouseButton, SharedString, WeakEntity, Window, anchored, deferred, div, prelude::*,
    px, relative,
};
use opensheet_engine::filter::{ColumnKind, SortState};
use opensheet_engine::ops::{SortError, SortLevel, SortOn, SortSpec};
use opensheet_engine::refs::col_to_letters;
use opensheet_engine::{Color, Rect, Workbook};
use std::rc::Rc;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- the model

/// What a level compares ("Sort On").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Basis {
    Values,
    CellColor,
    FontColor,
}

/// A line of the dialog.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Level {
    /// The key column (row, left to right) as an offset in the sorted
    /// range; `None` until one is chosen.
    pub key: Option<u32>,
    pub basis: Basis,
    /// Z to A; for a colour, on bottom (on the right).
    pub descending: bool,
    /// The colour of a colour level (`None`: no fill, automatic).
    pub color: Option<Color>,
    /// Values only: the order of this list instead of the alphabet.
    pub custom_list: Option<Vec<String>>,
}

impl Level {
    pub(super) fn blank() -> Level {
        Level {
            key: None,
            basis: Basis::Values,
            descending: false,
            color: None,
            custom_list: None,
        }
    }

    fn from_engine(level: &SortLevel) -> Level {
        let (basis, color) = match level.on {
            SortOn::Values => (Basis::Values, None),
            SortOn::CellColor(color) => (Basis::CellColor, color),
            SortOn::FontColor(color) => (Basis::FontColor, color),
        };
        Level {
            key: Some(level.key),
            basis,
            descending: level.descending,
            color,
            custom_list: level.custom_list.clone(),
        }
    }

    fn to_engine(&self, key: u32) -> SortLevel {
        SortLevel {
            key,
            on: match self.basis {
                Basis::Values => SortOn::Values,
                Basis::CellColor => SortOn::CellColor(self.color),
                Basis::FontColor => SortOn::FontColor(self.color),
            },
            descending: self.descending,
            custom_list: self.custom_list.clone().filter(|_| self.basis == Basis::Values),
        }
    }
}

/// Why OK cannot sort yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Problem {
    /// A level has no key.
    NoKey,
    /// Two levels sort the values of the same key (its offset).
    Repeated(u32),
}

/// Everything the dialog sets.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Setup {
    pub target: SortTarget,
    pub levels: Vec<Level>,
    pub has_header: bool,
    pub case_sensitive: bool,
    pub left_to_right: bool,
}

impl Setup {
    /// How the dialog opens on a target: on the sort its filter remembers,
    /// on the last sort of these very cells, else on one empty level.
    pub(super) fn opening(wb: &Workbook, sheet: usize, target: SortTarget, last: Option<&Setup>) -> Setup {
        let mut setup = Setup {
            target,
            levels: vec![Level::blank()],
            has_header: true,
            case_sensitive: false,
            left_to_right: false,
        };
        match target {
            SortTarget::Filter { scope, .. } => {
                if let Some(sort) = wb.auto_filter(sheet, scope).and_then(|filter| filter.sort.as_ref()) {
                    setup.levels = sort.levels.iter().map(Level::from_engine).collect();
                    setup.case_sensitive = sort.case_sensitive;
                }
            }
            SortTarget::Range { has_header, .. } => match last {
                Some(last) if last.target.rect() == target.rect() => return last.clone(),
                _ => setup.has_header = has_header,
            },
        }
        setup
    }

    pub(super) fn rect(&self) -> Rect {
        self.target.rect()
    }

    /// A list sorted through its filter always has its header row.
    pub(super) fn header_is_fixed(&self) -> bool {
        matches!(self.target, SortTarget::Filter { .. })
    }

    /// How many keys there are to choose from.
    fn key_count(&self) -> u32 {
        if self.left_to_right { self.rect().rows() } else { self.rect().cols() }
    }

    /// What the engine is asked to do.
    pub(super) fn spec(&self) -> Result<SortSpec, Problem> {
        let mut levels: Vec<SortLevel> = Vec::new();
        for level in &self.levels {
            let key = level.key.filter(|key| *key < self.key_count()).ok_or(Problem::NoKey)?;
            let repeated = level.basis == Basis::Values && levels.iter().any(|l| l.key == key && l.on == SortOn::Values);
            if repeated {
                return Err(Problem::Repeated(key));
            }
            levels.push(level.to_engine(key));
        }
        Ok(SortSpec {
            levels,
            has_header: self.has_header && !self.left_to_right,
            case_sensitive: self.case_sensitive,
            left_to_right: self.left_to_right,
        })
    }

    /// Sort. A filtered list sorted by rows keeps the sort with its
    /// filter (the arrows on its buttons).
    pub(super) fn apply(&self, wb: &mut Workbook, sheet: usize, spec: SortSpec) -> Result<(), SortError> {
        match self.target {
            SortTarget::Filter { scope, .. } if !spec.left_to_right => {
                let state = SortState {
                    levels: spec.levels,
                    case_sensitive: spec.case_sensitive,
                };
                wb.sort_filter(sheet, scope, state)
            }
            _ => wb.sort_range(sheet, self.rect(), &spec),
        }
    }

    /// The cells a key is made of: the column under the header, or the
    /// whole row.
    fn key_cells(&self, key: u32) -> impl Iterator<Item = (u32, u32)> + use<> {
        let rect = self.rect();
        let (left_to_right, first) = (self.left_to_right, rect.r0 + (self.has_header && !self.left_to_right) as u32);
        let along = if left_to_right { rect.c0..=rect.c1 } else { first..=rect.r1 };
        along
            .take(KEY_SAMPLE)
            .map(move |i| if left_to_right { (rect.r0 + key, i) } else { (i, rect.c0 + key) })
    }
}

/// How many keys the lists of the dialog offer, and how many cells of a
/// key are looked at.
const KEY_LIMIT: usize = 10_000;
const KEY_SAMPLE: usize = 5_000;

/// The names of the keys, in order: the header of each column, "Column
/// B" without headers, "Row 3" when sorting left to right.
pub(super) fn key_names(wb: &Workbook, sheet: usize, setup: &Setup) -> Vec<String> {
    let rect = setup.rect();
    if setup.left_to_right {
        return (rect.r0..=rect.r1)
            .take(KEY_LIMIT)
            .map(|row| t("Row {}", "Ligne {}").replace("{}", &(row + 1).to_string()))
            .collect();
    }
    (rect.c0..=rect.c1)
        .take(KEY_LIMIT)
        .map(|col| {
            if setup.has_header {
                column_title(&wb.display_text(sheet, rect.r0, col).text, col)
            } else {
                t("Column {}", "Colonne {}").replace("{}", &col_to_letters(col))
            }
        })
        .collect()
}

/// What the dialog needs to know about a key: what it holds (the wording
/// of the orders) and the colours found in it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Facts {
    pub kind: ColumnKind,
    pub cell_colors: Vec<Option<Color>>,
    pub font_colors: Vec<Option<Color>>,
}

pub(super) fn facts(wb: &Workbook, sheet: usize, setup: &Setup, key: Option<u32>) -> Facts {
    let mut out = Facts {
        kind: ColumnKind::Text,
        cell_colors: Vec::new(),
        font_colors: Vec::new(),
    };
    let Some(key) = key else {
        return out;
    };
    out.kind = cells_kind(wb, sheet, setup.key_cells(key));
    for (row, col) in setup.key_cells(key) {
        let (fill, font) = shown_colors(wb, sheet, row, col);
        if !out.cell_colors.contains(&fill) {
            out.cell_colors.push(fill);
        }
        if !out.font_colors.contains(&font) {
            out.font_colors.push(font);
        }
    }
    out
}

/// The two plain orders of a key, by what it holds.
pub(super) fn orders(kind: ColumnKind) -> [&'static str; 2] {
    match kind {
        ColumnKind::Text => [t("A to Z", "De A à Z"), t("Z to A", "De Z à A")],
        ColumnKind::Number => [
            t("Smallest to Largest", "Du plus petit au plus grand"),
            t("Largest to Smallest", "Du plus grand au plus petit"),
        ],
        ColumnKind::Date => [
            t("Oldest to Newest", "Du plus ancien au plus récent"),
            t("Newest to Oldest", "Du plus récent au plus ancien"),
        ],
    }
}

/// A custom list as the dialog names it: its items, in order or reversed.
pub(super) fn list_name(list: &[String], reversed: bool) -> String {
    let mut items: Vec<&str> = list.iter().map(String::as_str).collect();
    if reversed {
        items.reverse();
    }
    items.join(", ")
}

// ----------------------------------------------------------------- opening

/// Data > Sort and "Custom Sort...": the dialog on what the selection
/// stands for.
pub(super) fn open(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some(selection) = single_selection(app, cx) else {
        return;
    };
    match sort_plan(&app.wb, app.sheet, selection, app.active()) {
        SortPlan::Nothing => cannot_apply(app, cx),
        SortPlan::StraddlesFilter => straddles_filter(app, cx),
        SortPlan::Sort(target) => open_on(app, target, window, cx),
        SortPlan::Ask { selection, expanded } => ask_to_expand(
            app,
            Asking::Sort,
            selection,
            expanded,
            |app, rect, window, cx| {
                let has_header = app.wb.guess_header_row(app.sheet, rect);
                open_on(app, SortTarget::Range { rect, has_header }, window, cx);
            },
            window,
            cx,
        ),
    }
}

/// Open the dialog on a given target.
pub(super) fn open_on(app: &mut ExcelApp, target: SortTarget, window: &mut Window, cx: &mut Cx) {
    if !app.may(Intent::Sort(target.rect()), cx) {
        return;
    }
    let sheet = app.sheet;
    let last = app.ext::<State>().and_then(|state| state.last_sort.get(&sheet));
    let setup = Setup::opening(&app.wb, sheet, target, last);
    let names = names_of(&app.wb, sheet, &setup);
    let facts = setup.levels.iter().map(|level| facts(&app.wb, sheet, &setup, level.key)).collect();
    let lists = app.wb.custom_lists();
    // The rows about to be sorted show as the selection, as in Excel.
    let rect = target.rect();
    let first = (rect.r0 + setup.has_header as u32).min(rect.r1);
    select_keeping_active(app, Rect::new(first, rect.c0, rect.r1, rect.c1), cx);
    app.open_modal(window, cx, move |modal, _, _| SortDialog {
        modal,
        sheet,
        setup,
        names,
        facts,
        lists,
        selected: 0,
        colors_open: None,
        colors_closed: None,
    });
}

fn names_of(wb: &Workbook, sheet: usize, setup: &Setup) -> Vec<SharedString> {
    key_names(wb, sheet, setup).into_iter().map(SharedString::from).collect()
}

/// What OK does once the dialog is closed.
fn run(app: &mut ExcelApp, sheet: usize, setup: Setup, spec: SortSpec, cx: &mut Cx) {
    if app.sheet != sheet || spec.levels.is_empty() || !app.may(Intent::Sort(setup.rect()), cx) {
        return;
    }
    match setup.apply(&mut app.wb, sheet, spec) {
        Ok(()) => {
            app.commit(Cmd::Sort.label(), cx);
            app.ext_mut::<State>().last_sort.insert(sheet, setup);
        }
        Err(err) => {
            app.wb.rollback();
            sort_failed(app, err, cx);
        }
    }
}

// --------------------------------------------------------------- the dialog

const LEVEL_H: f32 = 32.0;
const LABEL_W: f32 = 64.0;
const KEY_W: f32 = 186.0;
const BASIS_W: f32 = 186.0;
const ORDER_W: f32 = 280.0;
const FIELD_BORDER: u32 = 0xABABAB;

struct SortDialog {
    modal: Modal,
    sheet: usize,
    setup: Setup,
    /// The keys to choose from, as `key_names` gives them.
    names: Vec<SharedString>,
    /// One per level.
    facts: Vec<Facts>,
    /// The custom lists "Custom List..." offers.
    lists: Vec<Vec<String>>,
    /// The level the buttons act on.
    selected: usize,
    /// The level whose list of colours is open.
    colors_open: Option<usize>,
    /// When a press outside closed that list: the click that follows on
    /// its own button must not open it again.
    colors_closed: Option<Instant>,
}

impl SortDialog {
    /// Look the keys up again after the header row or the orientation
    /// changed.
    fn refresh_keys(&mut self, cx: &mut Context<Self>) {
        let (sheet, setup) = (self.sheet, &self.setup);
        let read = self.modal.read_app(cx, |app| {
            let facts: Vec<Facts> = setup.levels.iter().map(|level| facts(&app.wb, sheet, setup, level.key)).collect();
            (names_of(&app.wb, sheet, setup), facts)
        });
        if let Some((names, facts)) = read {
            self.names = names;
            self.facts = facts;
        }
        cx.notify();
    }

    fn refresh_level(&mut self, index: usize, cx: &mut Context<Self>) {
        let (sheet, setup) = (self.sheet, &self.setup);
        let key = setup.levels[index].key;
        if let Some(found) = self.modal.read_app(cx, |app| facts(&app.wb, sheet, setup, key)) {
            self.facts[index] = found;
        }
        // A colour level starts on the first colour of its key.
        self.reset_color(index);
        cx.notify();
    }

    fn reset_color(&mut self, index: usize) {
        let level = &mut self.setup.levels[index];
        let colors = match level.basis {
            Basis::Values => return,
            Basis::CellColor => &self.facts[index].cell_colors,
            Basis::FontColor => &self.facts[index].font_colors,
        };
        level.color = colors.iter().flatten().next().copied();
    }

    fn add_level(&mut self, cx: &mut Context<Self>) {
        self.setup.levels.push(Level::blank());
        self.facts.push(facts_of_nothing());
        self.selected = self.setup.levels.len() - 1;
        cx.notify();
    }

    fn delete_level(&mut self, cx: &mut Context<Self>) {
        if self.selected < self.setup.levels.len() {
            self.setup.levels.remove(self.selected);
            self.facts.remove(self.selected);
            self.selected = self.selected.min(self.setup.levels.len().saturating_sub(1));
            cx.notify();
        }
    }

    fn copy_level(&mut self, cx: &mut Context<Self>) {
        if let Some(level) = self.setup.levels.get(self.selected).cloned() {
            let fact = self.facts[self.selected].clone();
            self.selected += 1;
            self.setup.levels.insert(self.selected, level);
            self.facts.insert(self.selected, fact);
            cx.notify();
        }
    }

    fn move_level(&mut self, down: bool, cx: &mut Context<Self>) {
        let from = self.selected;
        let to = if down { from + 1 } else { from.wrapping_sub(1) };
        if from < self.setup.levels.len() && to < self.setup.levels.len() {
            self.setup.levels.swap(from, to);
            self.facts.swap(from, to);
            self.selected = to;
            cx.notify();
        }
    }

    fn open_options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let parent = cx.weak_entity();
        let (case_sensitive, left_to_right) = (self.setup.case_sensitive, self.setup.left_to_right);
        self.modal.update_app(cx, |app, cx| {
            app.open_modal(window, cx, move |modal, _, _| OptionsDialog {
                modal,
                parent,
                case_sensitive,
                left_to_right,
            });
        });
    }

    /// The Sort Options dialog was closed with OK.
    fn set_options(&mut self, case_sensitive: bool, left_to_right: bool, cx: &mut Context<Self>) {
        self.setup.case_sensitive = case_sensitive;
        if self.setup.left_to_right != left_to_right {
            self.setup.left_to_right = left_to_right;
            // Columns are not rows: the keys are to be chosen again.
            for level in &mut self.setup.levels {
                level.key = None;
            }
            self.refresh_keys(cx);
        }
        cx.notify();
    }

    fn open_lists(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let parent = cx.weak_entity();
        let lists = self.lists.clone();
        let current = self.setup.levels[index].custom_list.as_ref();
        let selected = current.and_then(|list| lists.iter().position(|l| l == list));
        self.modal.update_app(cx, |app, cx| {
            app.open_modal(window, cx, move |modal, _, _| ListsDialog {
                modal,
                parent,
                level: index,
                lists,
                selected,
            });
        });
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let spec = match self.setup.spec() {
            Ok(spec) => spec,
            Err(problem) => {
                let message = match problem {
                    Problem::NoKey => t(
                        "The sort reference is not valid. Make sure that it's within the data you want to sort, and the first Sort By box isn't the same or blank.",
                        "Référence de tri non valide. Vérifiez qu'elle se trouve bien parmi les données à trier et que la zone Trier par n'est pas identique ou vide.",
                    )
                    .to_string(),
                    Problem::Repeated(key) => {
                        let name = self.names.get(key as usize).map(|name| name.to_string()).unwrap_or_default();
                        t(
                            "{} is being sorted by values more than once. Delete the duplicate sort criteria and try again.",
                            "{} est trié plusieurs fois par valeurs. Supprimez le critère de tri en double, puis réessayez.",
                        )
                        .replace("{}", &name)
                    }
                };
                self.modal
                    .update_app(cx, |app, cx| app.alert(Alert::new(APP_NAME, message).warning(), cx));
                return;
            }
        };
        let (sheet, setup) = (self.sheet, self.setup.clone());
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| run(app, sheet, setup, spec, cx));
    }

    /// The box of a colour: a bar of it, or the name of "no colour".
    fn color_face(color: Option<Color>, basis: Basis) -> AnyElement {
        match color {
            Some(Color(rgb)) => div()
                .w(px(84.))
                .h(px(12.))
                .bg(c(rgb))
                .border_1()
                .border_color(c(theme::BORDER_STRONG))
                .into_any_element(),
            None => div()
                .truncate()
                .child(match basis {
                    Basis::FontColor => t("Automatic", "Automatique"),
                    _ => t("No Cell Color", "Aucune couleur de cellule"),
                })
                .into_any_element(),
        }
    }

    /// The list of the colours of a key, for a colour level.
    fn color_select(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let level = &self.setup.levels[index];
        let basis = level.basis;
        let found = match basis {
            Basis::FontColor => &self.facts[index].font_colors,
            _ => &self.facts[index].cell_colors,
        };
        // The colours, then "no colour", which is always offered.
        let mut colors: Vec<Option<Color>> = found.iter().flatten().map(|color| Some(*color)).collect();
        colors.push(None);
        let open = self.colors_open == Some(index);
        let button = div()
            .id(("sort-color", index))
            .h(px(CONTROL_H))
            .w(px(132.))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(4.))
            .pl(px(6.))
            .pr(px(4.))
            .bg(c(0xFFFFFF))
            .border_1()
            .border_color(c(FIELD_BORDER))
            .rounded(px(3.))
            .child(SortDialog::color_face(level.color, basis))
            .child(icon("chevron-down", 10.0, theme::TEXT_MUTED))
            .on_click(cx.listener(move |this, _, _, cx| {
                let just_closed = this.colors_closed.take().is_some_and(|at| at.elapsed() < Duration::from_millis(250));
                if !just_closed {
                    this.colors_open = if open { None } else { Some(index) };
                    cx.notify();
                }
            }));
        let panel = open.then(|| {
            let options: Vec<AnyElement> = colors
                .iter()
                .enumerate()
                .map(|(at, color)| {
                    let color = *color;
                    div()
                        .id(("sort-color-option", at))
                        .h(px(ROW_H + 2.0))
                        .px(px(6.))
                        .flex()
                        .items_center()
                        .when(color == level.color, |d| d.bg(c(widgets::SELECTED_BG)))
                        .when(color != level.color, |d| d.hover(|s| s.bg(c(theme::HOVER))))
                        .child(SortDialog::color_face(color, basis))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.setup.levels[index].color = color;
                            this.colors_open = None;
                            cx.notify();
                        }))
                        .into_any_element()
                })
                .collect();
            div().absolute().left_0().top(relative(1.)).child(
                deferred(
                    anchored().snap_to_window_with_margin(px(6.)).child(
                        div()
                            .id("sort-colors")
                            .occlude()
                            .mt(px(1.))
                            .w(px(132.))
                            .py(px(2.))
                            .bg(c(0xFFFFFF))
                            .border_1()
                            .border_color(c(theme::BORDER_STRONG))
                            .rounded(px(3.))
                            .shadow_md()
                            .text_size(px(12.))
                            .text_color(c(theme::TEXT))
                            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                this.colors_open = None;
                                this.colors_closed = Some(Instant::now());
                                cx.notify();
                            }))
                            .children(options),
                    ),
                )
                .with_priority(3),
            )
        });
        div().relative().flex_none().child(button).children(panel).into_any_element()
    }

    /// The "Order" cell of a level.
    fn order(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let level = &self.setup.levels[index];
        if level.basis != Basis::Values {
            let sides: Vec<SharedString> = if self.setup.left_to_right {
                vec![t("On Left", "À gauche").into(), t("On Right", "À droite").into()]
            } else {
                vec![t("On Top", "En haut").into(), t("On Bottom", "En bas").into()]
            };
            return div()
                .flex()
                .gap(px(6.))
                .child(self.color_select(index, cx))
                .child(
                    Select::new(("sort-side", index), sides, Some(level.descending as usize))
                        .width(ORDER_W - 138.0)
                        .on_change(cx.listener(move |this, at: &usize, _, cx| {
                            this.setup.levels[index].descending = *at == 1;
                            cx.notify();
                        })),
                )
                .into_any_element();
        }
        // The two plain orders, the custom list in use (both ways), then
        // the entry that opens the lists.
        let mut options: Vec<SharedString> = orders(self.facts[index].kind).map(SharedString::from).to_vec();
        if let Some(list) = &level.custom_list {
            options.push(list_name(list, false).into());
            options.push(list_name(list, true).into());
        }
        options.push(t("Custom List…", "Liste personnalisée…").into());
        let custom = level.custom_list.is_some();
        let lists_at = options.len() - 1;
        let selected = level.descending as usize + if custom { 2 } else { 0 };
        Select::new(("sort-order", index), options, Some(selected))
            .width(ORDER_W)
            .on_change(cx.listener(move |this, at: &usize, window, cx| {
                if *at == lists_at {
                    this.open_lists(index, window, cx);
                    return;
                }
                let level = &mut this.setup.levels[index];
                level.descending = *at % 2 == 1;
                if *at < 2 {
                    level.custom_list = None;
                }
                cx.notify();
            }))
            .into_any_element()
    }

    fn level_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let level = &self.setup.levels[index];
        let on = index == self.selected;
        let bases: Vec<SharedString> = vec![
            t("Cell Values", "Valeurs de cellule").into(),
            t("Cell Color", "Couleur de cellule").into(),
            t("Font Color", "Couleur de police").into(),
        ];
        let basis = match level.basis {
            Basis::Values => 0,
            Basis::CellColor => 1,
            Basis::FontColor => 2,
        };
        div()
            .id(("sort-level", index))
            .h(px(LEVEL_H))
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(8.))
            .when(on, |d| d.bg(c(widgets::SELECTED_BG)))
            // A press anywhere on the line makes it the current level.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    if this.selected != index {
                        this.selected = index;
                        cx.notify();
                    }
                }),
            )
            .child(
                label(if index == 0 { t("Sort by", "Trier par") } else { t("Then by", "Puis par") }).w(px(LABEL_W)),
            )
            .child(
                Select::new(("sort-key", index), self.names.clone(), level.key.map(|key| key as usize))
                    .width(KEY_W)
                    .on_change(cx.listener(move |this, at: &usize, _, cx| {
                        this.setup.levels[index].key = Some(*at as u32);
                        this.refresh_level(index, cx);
                    })),
            )
            .child(
                Select::new(("sort-basis", index), bases, Some(basis))
                    .width(BASIS_W)
                    .on_change(cx.listener(move |this, at: &usize, _, cx| {
                        let level = &mut this.setup.levels[index];
                        level.basis = [Basis::Values, Basis::CellColor, Basis::FontColor][(*at).min(2)];
                        level.descending = false;
                        this.reset_color(index);
                        cx.notify();
                    })),
            )
            .child(self.order(index, cx))
            .into_any_element()
    }
}

fn facts_of_nothing() -> Facts {
    Facts {
        kind: ColumnKind::Text,
        cell_colors: Vec::new(),
        font_colors: Vec::new(),
    }
}

impl Render for SortDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.selected = self.selected.min(self.setup.levels.len().saturating_sub(1));
        let none = self.setup.levels.is_empty();
        let arrow = |name: &'static str, tip: &'static str, down: bool, cx: &mut Context<Self>| {
            div()
                .id(name)
                .size(px(CONTROL_H))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .border_1()
                .border_color(c(FIELD_BORDER))
                .bg(c(0xFFFFFF))
                .hover(|s| s.bg(c(theme::HOVER)))
                .active(|s| s.bg(c(theme::PRESSED)))
                .child(icon(name, 11.0, theme::TEXT))
                .tooltip(tooltip(tip))
                .on_click(cx.listener(move |this, _, _, cx| this.move_level(down, cx)))
        };
        let heading = |text: &'static str, width: f32| {
            div()
                .w(px(width))
                .flex_none()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(c(theme::TEXT_MUTED))
                .child(text)
        };
        let levels: Vec<AnyElement> = (0..self.setup.levels.len()).map(|index| self.level_row(index, cx)).collect();
        let key_heading = if self.setup.left_to_right {
            t("Row", "Ligne")
        } else {
            // Not the "Column" of column charts.
            i18n::t_ctx("sort", "Column", "Colonne")
        };
        let fixed = self.setup.header_is_fixed() || self.setup.left_to_right;
        dialog(&self.modal, i18n::t_ctx("dialog", "Sort", "Tri"))
            .width(LABEL_W + KEY_W + BASIS_W + ORDER_W + 78.0)
            .on_ok(cx.listener(Self::ok))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        Button::new("sort-add", t("Add Level", "Ajouter un niveau"))
                            .on_click(cx.listener(|this, _, _, cx| this.add_level(cx))),
                    )
                    .child(
                        Button::new("sort-delete", t("Delete Level", "Supprimer un niveau"))
                            .disabled(none)
                            .on_click(cx.listener(|this, _, _, cx| this.delete_level(cx))),
                    )
                    .child(
                        Button::new("sort-copy", t("Copy Level", "Copier un niveau"))
                            .disabled(none)
                            .on_click(cx.listener(|this, _, _, cx| this.copy_level(cx))),
                    )
                    .child(arrow("chevron-up", t("Move Up", "Monter"), false, cx))
                    .child(arrow("chevron-down", t("Move Down", "Descendre"), true, cx))
                    .child(
                        Button::new("sort-options", t("Options…", "Options…"))
                            .on_click(cx.listener(|this, _, window, cx| this.open_options(window, cx))),
                    )
                    .child(div().flex_1())
                    .child(
                        Checkbox::new(
                            "sort-headers",
                            t("My data has headers", "Mes données ont des en-têtes"),
                            self.setup.has_header && !self.setup.left_to_right,
                        )
                        .disabled(fixed)
                        .on_change(cx.listener(|this, on: &bool, _, cx| {
                            this.setup.has_header = *on;
                            this.refresh_keys(cx);
                        })),
                    ),
            )
            .child(
                div()
                    .bg(c(0xFFFFFF))
                    .border_1()
                    .border_color(c(FIELD_BORDER))
                    .rounded(px(3.))
                    .overflow_hidden()
                    .child(
                        div()
                            .h(px(24.))
                            .px(px(8.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .bg(c(theme::CHROME_BG))
                            .border_b_1()
                            .border_color(c(theme::BORDER))
                            .child(heading(key_heading, LABEL_W + 8.0 + KEY_W))
                            .child(heading(t("Sort On", "Trier sur"), BASIS_W))
                            .child(heading(t("Order", "Ordre"), ORDER_W)),
                    )
                    .child(ScrollArea::new("sort-levels", LEVEL_H * 5.0).children(levels)),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// ------------------------------------------------------------- sort options

struct OptionsDialog {
    modal: Modal,
    parent: WeakEntity<SortDialog>,
    case_sensitive: bool,
    left_to_right: bool,
}

impl OptionsDialog {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let (case_sensitive, left_to_right) = (self.case_sensitive, self.left_to_right);
        self.modal.close(window, cx);
        self.parent
            .update(cx, |parent, cx| parent.set_options(case_sensitive, left_to_right, cx))
            .ok();
    }
}

impl Render for OptionsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let orientations: Vec<SharedString> = vec![
            t("Sort top to bottom", "Du haut vers le bas").into(),
            t("Sort left to right", "De la gauche vers la droite").into(),
        ];
        dialog(&self.modal, t("Sort Options", "Options de tri"))
            .width(280.0)
            .on_ok(cx.listener(Self::ok))
            .child(
                Checkbox::new("sort-case", t("Case sensitive", "Respecter la casse"), self.case_sensitive).on_change(
                    cx.listener(|this, on: &bool, _, cx| {
                        this.case_sensitive = *on;
                        cx.notify();
                    }),
                ),
            )
            .child(
                GroupBox::new(t("Orientation", "Orientation")).child(
                    RadioGroup::new("sort-orientation", orientations, Some(self.left_to_right as usize)).on_change(
                        cx.listener(|this, at: &usize, _, cx| {
                            this.left_to_right = *at == 1;
                            cx.notify();
                        }),
                    ),
                ),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// ------------------------------------------------------------- custom lists

struct ListsDialog {
    modal: Modal,
    parent: WeakEntity<SortDialog>,
    /// The level of the Sort dialog the list is for.
    level: usize,
    lists: Vec<Vec<String>>,
    selected: Option<usize>,
}

impl ListsDialog {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let (level, list) = (self.level, self.selected.and_then(|at| self.lists.get(at).cloned()));
        self.modal.close(window, cx);
        let Some(list) = list else {
            return;
        };
        self.parent
            .update(cx, |parent, cx| {
                if let Some(level) = parent.setup.levels.get_mut(level) {
                    level.custom_list = Some(list);
                    level.descending = false;
                }
                cx.notify();
            })
            .ok();
    }
}

impl Render for ListsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let names: Vec<SharedString> = self.lists.iter().map(|list| list_name(list, false).into()).collect();
        let entries: Vec<SharedString> = self
            .selected
            .and_then(|at| self.lists.get(at))
            .map(|list| list.iter().map(|item| SharedString::from(item.clone())).collect())
            .unwrap_or_default();
        let column = |title: &'static str, list: ListBox| div().flex().flex_col().gap(px(4.)).child(label(title)).child(list);
        dialog(&self.modal, t("Custom Lists", "Listes personnalisées"))
            .width(480.0)
            .on_ok(cx.listener(Self::ok))
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(column(
                        t("Custom lists:", "Listes personnalisées\u{a0}:"),
                        ListBox::new("lists", names, self.selected)
                            .width(270.0)
                            .height(ROW_H * 10.0)
                            .on_change(cx.listener(|this, at: &usize, _, cx| {
                                this.selected = Some(*at);
                                cx.notify();
                            }))
                            .on_activate(cx.listener(|this, at: &usize, window, cx| {
                                this.selected = Some(*at);
                                this.ok(&(), window, cx);
                            })),
                    ))
                    .child(column(
                        t("List entries:", "Entrées de la liste\u{a0}:"),
                        ListBox::new("list-entries", entries, None).width(170.0).height(ROW_H * 10.0),
                    )),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// ---------------------------------------------------------- "sort warning"

/// What is about to be done to a selection that has data beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Asking {
    Sort,
    Duplicates,
}

struct ExpandDialog {
    modal: Modal,
    asking: Asking,
    expand: bool,
    selection: Rect,
    expanded: Rect,
    then: Rc<dyn Fn(&mut ExcelApp, Rect, &mut Window, &mut Cx)>,
}

/// Ask whether a command goes on with the selected cells alone or with the
/// whole block of data around them; `then` runs with the cells chosen.
pub(super) fn ask_to_expand(
    app: &mut ExcelApp,
    asking: Asking,
    selection: Rect,
    expanded: Rect,
    then: impl Fn(&mut ExcelApp, Rect, &mut Window, &mut Cx) + 'static,
    window: &mut Window,
    cx: &mut Cx,
) {
    let then = Rc::new(then);
    app.open_modal(window, cx, move |modal, _, _| ExpandDialog {
        modal,
        asking,
        expand: true,
        selection,
        expanded,
        then,
    });
}

impl ExpandDialog {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let (expand, then) = (self.expand, self.then.clone());
        let rect = if expand { self.expanded } else { self.selection };
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            if expand {
                select_keeping_active(app, rect, cx);
            }
            then(app, rect, window, cx);
        });
    }
}

impl Render for ExpandDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (title, message, action) = match self.asking {
            Asking::Sort => (
                t("Sort Warning", "Attention"),
                t(
                    "Data outside your current selection won't be sorted.",
                    "Les données en dehors de votre sélection actuelle ne seront pas triées.",
                ),
                t("Sort…", "Trier…"),
            ),
            Asking::Duplicates => (
                t("Remove Duplicates Warning", "Supprimer les avertissements en double"),
                t(
                    "OpenSheet found data next to your selection. Because you have not selected this data, it will not be removed.",
                    "OpenSheet a trouvé des données près de votre sélection. Étant donné qu'elles ne sont pas sélectionnées, elles ne seront pas supprimées.",
                ),
                t("Remove Duplicates…", "Supprimer les doublons…"),
            ),
        };
        let choices: Vec<SharedString> = vec![
            t("Expand the selection", "Étendre la sélection").into(),
            t("Continue with the current selection", "Continuer avec la sélection en cours").into(),
        ];
        let modal = self.modal.clone();
        dialog(&self.modal, title)
            .width(420.0)
            .on_ok(cx.listener(Self::ok))
            .child(div().line_height(px(18.)).child(message))
            .child(label(t("What do you want to do?", "Que voulez-vous faire\u{a0}?")))
            .child(
                div().pl(px(12.)).child(
                    RadioGroup::new("expand-choice", choices, Some(if self.expand { 0 } else { 1 })).on_change(
                        cx.listener(|this, at: &usize, _, cx| {
                            this.expand = *at == 0;
                            cx.notify();
                        }),
                    ),
                ),
            )
            .footer(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(Button::new("expand-go", action).primary().on_click(cx.listener(Self::ok)))
                    .child(
                        Button::new("expand-cancel", t("Cancel", "Annuler"))
                            .on_click(move |_, window, cx| modal.close(window, cx)),
                    ),
            )
    }
}
