//! Go To (the list of names and the Reference box) and Go To Special (the
//! cells of a kind, selected together).

use super::links;
use crate::app::ExcelApp;
use crate::features::{Cx, names};
use crate::grid::Selection;
use crate::i18n::{t, t_ctx};
use crate::shell::{Alert, Modal, RefStyle};
use crate::text_input::TextInput;
use crate::theme::{self, c};
use crate::widgets::{self, Button, Checkbox, GroupBox, ListBox, RadioGroup, TextField, dialog, label, ok_cancel};
use gpui::{App, Context, ElementId, Entity, KeyDownEvent, SharedString, Window, div, prelude::*, px};
use opensheet_engine::{Content, Rect, Reference, Value, Workbook};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

/// The places Go To was last asked to leave, most recent first.
#[derive(Default)]
struct Visited {
    places: Vec<String>,
}

/// How many places the Go To list remembers, as Excel does.
const REMEMBERED: usize = 4;

fn none_found(app: &mut ExcelApp, cx: &mut Cx) {
    app.alert(
        Alert::new("OpenSheet", t("No cells were found.", "Pas de cellules correspondantes.")).warning(),
        cx,
    );
}

// --------------------------------------------------------------------- Go To

pub(super) fn open(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let mut items = app.ext::<Visited>().map(|visited| visited.places.clone()).unwrap_or_default();
    items.extend(names::visible_names(&app.wb, app.sheet));
    app.open_modal(window, cx, move |modal, _, cx| GoTo {
        reference: widgets::input(cx, ""),
        modal,
        items,
        selected: None,
    });
}

/// Select what `text` stands for, remembering where the selection was.
/// Returns false, after saying so, when it stands for nothing.
fn go_to(app: &mut ExcelApp, text: &str, cx: &mut Cx) -> bool {
    let (row, col) = app.active();
    let Some(areas) = app.wb.reference_areas(app.sheet, row, col, text) else {
        app.alert(Alert::new("OpenSheet", names::invalid_reference()).warning(), cx);
        return false;
    };
    let style = RefStyle {
        absolute: true,
        sheet: true,
        equals: false,
    };
    let from = style.format(app.sel().current(), &app.wb.sheets[app.sheet].name);
    if !names::select_areas(app, &areas, cx) {
        app.alert(Alert::new("OpenSheet", names::invalid_reference()).warning(), cx);
        return false;
    }
    remember(&mut app.ext_mut::<Visited>().places, from);
    true
}

/// Put a place at the head of the list of those visited.
fn remember(places: &mut Vec<String>, place: String) {
    places.retain(|known| *known != place);
    places.insert(0, place);
    places.truncate(REMEMBERED);
}

struct GoTo {
    modal: Modal,
    /// The places visited, then the names.
    items: Vec<String>,
    selected: Option<usize>,
    reference: Entity<TextInput>,
}

impl GoTo {
    fn pick(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = Some(index);
        if let Some(item) = self.items.get(index).cloned() {
            self.reference.update(cx, |input, cx| {
                input.set_text(item, cx);
                input.select_all_text(cx);
            });
        }
        cx.notify();
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let text = self.reference.read(cx).text().trim().to_string();
        if text.is_empty() {
            return;
        }
        if self.modal.update_app(cx, |app, cx| go_to(app, &text, cx)) == Some(true) {
            self.modal.close(window, cx);
        }
    }

    fn special(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| open_special(app, window, cx));
    }
}

impl Render for GoTo {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let items: Vec<SharedString> = self.items.iter().map(|item| SharedString::from(item.clone())).collect();
        dialog(&self.modal, t("Go To", "Atteindre"))
            .width(320.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(t("Go to:", "Atteindre :")))
            .child(
                div().flex().child(
                    ListBox::new("places", items, self.selected)
                        .height(160.0)
                        .on_change(cx.listener(|this, index: &usize, _, cx| this.pick(*index, cx)))
                        .on_activate(cx.listener(|this, index: &usize, window, cx| {
                            this.pick(*index, cx);
                            this.ok(&(), window, cx);
                        })),
                ),
            )
            .child(label(t("Reference:", "Référence :")))
            .child(div().flex().child(TextField::new(&self.reference)))
            .footer(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        Button::new("special", t("Special...", "Cellules..."))
                            .on_click(cx.listener(|this, _, window, cx| this.special(window, cx))),
                    )
                    .child(div().flex_1())
                    .child(ok_cancel(&self.modal, cx.listener(Self::ok))),
            )
    }
}

// ------------------------------------------------------------- what to find

/// The kinds of values Constants and Formulas keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Kinds {
    pub numbers: bool,
    pub text: bool,
    pub logicals: bool,
    pub errors: bool,
}

impl Kinds {
    pub const ALL: Kinds = Kinds {
        numbers: true,
        text: true,
        logicals: true,
        errors: true,
    };

    fn admits(&self, value: &Value) -> bool {
        match value {
            Value::Number(_) => self.numbers,
            Value::Text(_) => self.text,
            Value::Bool(_) => self.logicals,
            Value::Error(_) => self.errors,
            // A formula giving nothing shows 0: a number.
            Value::Empty | Value::Array(_) => self.numbers,
        }
    }
}

/// The choices of Go To Special.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Special {
    Notes,
    Constants(Kinds),
    Formulas(Kinds),
    Blanks,
    CurrentRegion,
    CurrentArray,
    /// The charts, pictures and shapes: not cells (see `select_special`).
    Objects,
    RowDifferences,
    ColumnDifferences,
    Precedents { all_levels: bool },
    Dependents { all_levels: bool },
    LastCell,
    VisibleCells,
    /// `same`: only those formatted like the active cell.
    ConditionalFormats { same: bool },
    DataValidation { same: bool },
}

/// Stack runs of cells (row, first column, last column), sorted by row,
/// into rectangles: runs covering the same columns of consecutive rows
/// make one.
fn stack_runs(runs: Vec<(u32, u32, u32)>) -> Vec<Rect> {
    let mut out: Vec<Rect> = Vec::new();
    // Rectangle still growing for each column span.
    let mut open: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    for (row, c0, c1) in runs {
        match open.get(&(c0, c1)).copied() {
            Some(index) if out[index].r1 + 1 == row => out[index].r1 = row,
            _ => {
                open.insert((c0, c1), out.len());
                out.push(Rect::new(row, c0, row, c1));
            }
        }
    }
    out
}

/// The fewest rectangles of whole runs covering a set of cells.
pub(crate) fn coalesce(cells: &BTreeSet<(u32, u32)>) -> Vec<Rect> {
    let mut runs: Vec<(u32, u32, u32)> = Vec::new();
    for (row, col) in cells {
        match runs.last_mut() {
            Some((r, _, c1)) if *r == *row && *c1 + 1 == *col => *c1 = *col,
            _ => runs.push((*row, *col, *col)),
        }
    }
    stack_runs(runs)
}

/// Whether two cells hold the same thing: the same constant, or formulas
/// that read the same once copied from one cell to the other.
fn same_content(wb: &Workbook, sheet: usize, a: (u32, u32), b: (u32, u32)) -> bool {
    let content = |cell: (u32, u32)| wb.cell(sheet, cell.0, cell.1).map(|c| &c.content);
    match (content(a), content(b)) {
        (None | Some(Content::Empty), None | Some(Content::Empty)) => true,
        (Some(Content::Value(x)), Some(Content::Value(y))) => x == y,
        (Some(Content::Formula(x)), Some(Content::Formula(y))) => {
            x.rebased(-(a.0 as i64), -(a.1 as i64)) == y.rebased(-(b.0 as i64), -(b.1 as i64))
        }
        _ => false,
    }
}

/// The cells Go To Special selects on `sheet` for the selection it is
/// asked from; empty when there are none. A selection of one cell means
/// the whole sheet for the choices that search.
pub(crate) fn special_cells(wb: &Workbook, sheet: usize, selection: &Selection, what: Special) -> Vec<Rect> {
    let Some(sh) = wb.sheets.get(sheet) else {
        return Vec::new();
    };
    let (row, col) = selection.active;
    let used = sh.used_rect();
    let one_cell = match selection.ranges.as_slice() {
        [range] => range.is_single() || sh.merge_at(row, col) == Some(*range),
        _ => false,
    };
    // Where to search, never beyond the cells in use.
    let area: Vec<Rect> = match (used, one_cell) {
        (None, _) => Vec::new(),
        (Some(used), true) => vec![used],
        (Some(used), false) => selection.ranges.iter().filter_map(|range| range.intersection(&used)).collect(),
    };
    let within = |row: u32, col: u32| area.iter().any(|range| range.contains(row, col));
    let cells_where = |keep: &dyn Fn(&opensheet_engine::Cell) -> bool| -> Vec<Rect> {
        let found: BTreeSet<(u32, u32)> = sh
            .cells
            .iter()
            .filter(|((row, col), cell)| within(*row, *col) && keep(cell))
            .map(|(at, _)| *at)
            .collect();
        coalesce(&found)
    };
    // The ranges of the rules (conditional formats, validation) that
    // count: all of them, or those covering the active cell.
    let rule_ranges = |rules: Vec<&Vec<Rect>>, same: bool| -> Vec<Rect> {
        let mut out: Vec<Rect> = Vec::new();
        for ranges in rules {
            if same && !ranges.iter().any(|range| range.contains(row, col)) {
                continue;
            }
            for range in ranges {
                if !out.contains(range) {
                    out.push(*range);
                }
            }
        }
        out
    };
    // Cells reached by following formulas from the selection, one level
    // or all of them; `step` gives the neighbours of a rectangle.
    let follow = |all_levels: bool, step: &dyn Fn(Rect) -> Vec<Rect>| -> Vec<Rect> {
        let mut found: Vec<Rect> = Vec::new();
        let mut frontier: Vec<Rect> = selection.ranges.iter().map(|range| wb.clamp_to_used(sheet, *range)).collect();
        // Chains of formulas end; the bound is for circular ones.
        for _ in 0..10_000 {
            let mut next: Vec<Rect> = Vec::new();
            for from in &frontier {
                for reached in step(*from) {
                    if !found.contains(&reached) {
                        found.push(reached);
                        next.push(reached);
                    }
                }
            }
            if !all_levels || next.is_empty() {
                break;
            }
            frontier = next;
        }
        found
    };

    match what {
        Special::Notes => {
            let found: BTreeSet<(u32, u32)> = sh.comments.keys().filter(|(row, col)| within(*row, *col)).copied().collect();
            coalesce(&found)
        }
        Special::Constants(kinds) => {
            cells_where(&|cell| matches!(&cell.content, Content::Value(value) if !value.is_empty() && kinds.admits(value)))
        }
        Special::Formulas(kinds) => {
            cells_where(&|cell| matches!(cell.content, Content::Formula(_)) && kinds.admits(&cell.value))
        }
        Special::Blanks => {
            let mut runs: Vec<(u32, u32, u32)> = Vec::new();
            let mut seen: Vec<Rect> = Vec::new();
            for range in &area {
                // Ranges of a multiple selection may overlap.
                if seen.iter().any(|done| done.intersects(range)) {
                    continue;
                }
                seen.push(*range);
                for row in range.r0..=range.r1 {
                    let mut start = range.c0;
                    for col in range.c0..=range.c1 {
                        if !sh.is_blank(row, col) {
                            if col > start {
                                runs.push((row, start, col - 1));
                            }
                            start = col + 1;
                        }
                    }
                    if start <= range.c1 {
                        runs.push((row, start, range.c1));
                    }
                }
            }
            runs.sort_unstable();
            stack_runs(runs)
        }
        Special::CurrentRegion => vec![wb.current_region(sheet, row, col)],
        Special::CurrentArray => sh
            .spill_at(row, col)
            .map(|spill| {
                Rect::new(
                    spill.row,
                    spill.col,
                    spill.row + spill.array.rows as u32 - 1,
                    spill.col + spill.array.cols as u32 - 1,
                )
            })
            .into_iter()
            .collect(),
        Special::Objects => Vec::new(),
        Special::RowDifferences | Special::ColumnDifferences => {
            let by_row = what == Special::RowDifferences;
            let mut found: BTreeSet<(u32, u32)> = BTreeSet::new();
            if !one_cell {
                for range in &area {
                    // Each row (or column) is compared with its cell in the
                    // column (or row) of the active cell.
                    let pivot_col = if col >= range.c0 && col <= range.c1 { col } else { range.c0 };
                    let pivot_row = if row >= range.r0 && row <= range.r1 { row } else { range.r0 };
                    for r in range.r0..=range.r1 {
                        for c in range.c0..=range.c1 {
                            let model = if by_row { (r, pivot_col) } else { (pivot_row, c) };
                            if !same_content(wb, sheet, model, (r, c)) {
                                found.insert((r, c));
                            }
                        }
                    }
                }
            }
            coalesce(&found)
        }
        Special::Precedents { all_levels } => follow(all_levels, &|from| {
            let mut out = Vec::new();
            for ((r, c), cell) in sh.cells.range((from.r0, 0)..=(from.r1, u32::MAX)) {
                if *c < from.c0 || *c > from.c1 || !matches!(cell.content, Content::Formula(_)) {
                    continue;
                }
                for reference in links::precedents(wb, sheet, *r, *c) {
                    if reference.sheet == sheet {
                        out.push(wb.clamp_to_used(sheet, reference.rect));
                    }
                }
            }
            out
        }),
        Special::Dependents { all_levels } => follow(all_levels, &|from| {
            links::dependents(wb, Reference::new(sheet, from))
                .into_iter()
                .filter(|(on, _, _)| *on == sheet)
                .map(|(_, r, c)| Rect::cell(r, c))
                .collect()
        }),
        Special::LastCell => used.map(|used| Rect::cell(used.r1, used.c1)).into_iter().collect(),
        Special::VisibleCells => {
            let hidden_rows = sh.rows_not_shown();
            let mut out: Vec<Rect> = Vec::new();
            for range in &selection.ranges {
                let range = wb.clamp_to_used(sheet, *range);
                // The stretches of rows, then of columns, that show.
                let stretches = |from: u32, to: u32, hidden: &BTreeSet<u32>| -> Vec<(u32, u32)> {
                    let mut out = Vec::new();
                    let mut start = from;
                    for at in hidden.range(from..=to) {
                        if *at > start {
                            out.push((start, at - 1));
                        }
                        start = at + 1;
                    }
                    if start <= to {
                        out.push((start, to));
                    }
                    out
                };
                let cols = stretches(range.c0, range.c1, &sh.hidden_cols);
                for (r0, r1) in stretches(range.r0, range.r1, &hidden_rows) {
                    out.extend(cols.iter().map(|(c0, c1)| Rect::new(r0, *c0, r1, *c1)));
                }
            }
            out
        }
        Special::ConditionalFormats { same } => {
            rule_ranges(sh.cond_formats.iter().map(|format| &format.ranges).collect(), same)
        }
        Special::DataValidation { same } => {
            rule_ranges(sh.validations.iter().map(|validation| &validation.ranges).collect(), same)
        }
    }
}

/// Go To Special: select the cells of a kind, or say there are none.
pub(crate) fn select_special(app: &mut ExcelApp, what: Special, cx: &mut Cx) {
    if what == Special::Objects {
        app.deselect_object(cx);
        if !app.select_all_objects(cx) {
            none_found(app, cx);
        }
        return;
    }
    let sheet = app.sheet;
    let mut ranges = special_cells(&app.wb, sheet, app.sel(), what);
    if ranges.is_empty() {
        none_found(app, cx);
        return;
    }
    // The active cell stays where it is when it is one of the cells found.
    let (row, col) = app.active();
    if let Some(index) = ranges.iter().position(|range| range.contains(row, col)) {
        ranges.rotate_left(index);
    }
    let areas: Vec<Reference> = ranges.into_iter().map(|range| Reference::new(sheet, range)).collect();
    names::select_areas(app, &areas, cx);
    if areas[0].rect.contains(row, col) {
        let view = &mut app.views[sheet];
        view.sel.active = (row, col);
        app.refresh(cx);
    }
}

// ------------------------------------------------------------ Go To Special

pub(super) fn open_special(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    app.open_modal(window, cx, |modal, _, _| GoToSpecial {
        modal,
        choice: Choice::Notes,
        kinds: Kinds::ALL,
        all_levels: false,
        same: false,
    });
}

/// The radio buttons, in the order of the dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choice {
    Notes,
    Constants,
    Formulas,
    Blanks,
    CurrentRegion,
    CurrentArray,
    Objects,
    RowDifferences,
    ColumnDifferences,
    Precedents,
    Dependents,
    LastCell,
    VisibleCells,
    ConditionalFormats,
    DataValidation,
}

impl Choice {
    fn label(self) -> &'static str {
        match self {
            Choice::Notes => t("Notes", "Notes"),
            Choice::Constants => t("Constants", "Constantes"),
            Choice::Formulas => t("Formulas", "Formules"),
            Choice::Blanks => t_ctx("special", "Blanks", "Vides"),
            Choice::CurrentRegion => t("Current region", "Zone active"),
            Choice::CurrentArray => t("Current array", "Tableau actif"),
            Choice::Objects => t("Objects", "Objets"),
            Choice::RowDifferences => t("Row differences", "Différences par ligne"),
            Choice::ColumnDifferences => t("Column differences", "Différences par colonne"),
            Choice::Precedents => t("Precedents", "Antécédents"),
            Choice::Dependents => t("Dependents", "Dépendants"),
            Choice::LastCell => t("Last cell", "Dernière cellule"),
            Choice::VisibleCells => t("Visible cells only", "Cellules visibles uniquement"),
            Choice::ConditionalFormats => t("Conditional formats", "Formats conditionnels"),
            Choice::DataValidation => t("Data validation", "Validation de données"),
        }
    }
}

/// One radio button of a group laid out by hand (the options of this
/// dialog have other controls between them).
#[derive(IntoElement)]
struct Radio {
    id: ElementId,
    label: SharedString,
    on: bool,
    on_pick: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl RenderOnce for Radio {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = widgets::focus_of(&self.id, true, window, cx);
        let focused = focus.is_focused(window);
        let (pick, key_pick) = (self.on_pick.clone(), self.on_pick);
        div()
            .id(self.id)
            .track_focus(&focus)
            .flex_none()
            .on_key_down(move |ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key == "space" && widgets::plain(ev) {
                    key_pick(window, cx);
                    cx.stop_propagation();
                }
            })
            .child(
                div()
                    .id("hit")
                    .h(px(widgets::ROW_H))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .size(px(14.))
                            .flex_none()
                            .rounded_full()
                            .border_1()
                            .border_color(c(if self.on { theme::ACCENT } else { theme::TEXT_MUTED }))
                            .bg(c(0xFFFFFF))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(self.on, |d| d.child(div().size(px(8.)).rounded_full().bg(c(theme::ACCENT)))),
                    )
                    .child(
                        div()
                            .px(px(2.))
                            .border_1()
                            .border_color(if focused { c(theme::ACCENT) } else { gpui::transparent_black() })
                            .rounded(px(2.))
                            .whitespace_nowrap()
                            .child(self.label),
                    )
                    .on_click(move |_, window, cx| pick(window, cx)),
            )
    }
}

struct GoToSpecial {
    modal: Modal,
    choice: Choice,
    kinds: Kinds,
    /// "All levels" rather than "Direct only".
    all_levels: bool,
    /// "Same" rather than "All".
    same: bool,
}

impl GoToSpecial {
    fn what(&self) -> Special {
        match self.choice {
            Choice::Notes => Special::Notes,
            Choice::Constants => Special::Constants(self.kinds),
            Choice::Formulas => Special::Formulas(self.kinds),
            Choice::Blanks => Special::Blanks,
            Choice::CurrentRegion => Special::CurrentRegion,
            Choice::CurrentArray => Special::CurrentArray,
            Choice::Objects => Special::Objects,
            Choice::RowDifferences => Special::RowDifferences,
            Choice::ColumnDifferences => Special::ColumnDifferences,
            Choice::Precedents => Special::Precedents {
                all_levels: self.all_levels,
            },
            Choice::Dependents => Special::Dependents {
                all_levels: self.all_levels,
            },
            Choice::LastCell => Special::LastCell,
            Choice::VisibleCells => Special::VisibleCells,
            Choice::ConditionalFormats => Special::ConditionalFormats { same: self.same },
            Choice::DataValidation => Special::DataValidation { same: self.same },
        }
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let what = self.what();
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| select_special(app, what, cx));
    }

    fn radio(&self, choice: Choice, cx: &mut Context<Self>) -> Radio {
        let pick = cx.listener(move |this, _: &(), _, cx| {
            this.choice = choice;
            cx.notify();
        });
        Radio {
            id: ElementId::Name(format!("choice-{choice:?}").into()),
            label: choice.label().into(),
            on: self.choice == choice,
            on_pick: Rc::new(move |window, cx| pick(&(), window, cx)),
        }
    }

    fn kind(&self, id: &'static str, text: &'static str, on: bool, set: fn(&mut Kinds, bool), cx: &mut Context<Self>) -> Checkbox {
        let enabled = matches!(self.choice, Choice::Constants | Choice::Formulas);
        Checkbox::new(id, text, on)
            .disabled(!enabled)
            .on_change(cx.listener(move |this, on: &bool, _, cx| {
                set(&mut this.kinds, *on);
                cx.notify();
            }))
    }
}

impl Render for GoToSpecial {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kinds = self.kinds;
        let levels = matches!(self.choice, Choice::Precedents | Choice::Dependents);
        let rules = matches!(self.choice, Choice::ConditionalFormats | Choice::DataValidation);
        let indent = |control: gpui::AnyElement| div().pl(px(22.)).child(control);
        let left = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(self.radio(Choice::Notes, cx))
            .child(self.radio(Choice::Constants, cx))
            .child(self.radio(Choice::Formulas, cx))
            .child(indent(
                self.kind("numbers", t("Numbers", "Nombres"), kinds.numbers, |k, on| k.numbers = on, cx).into_any_element(),
            ))
            .child(indent(self.kind("text", t("Text", "Texte"), kinds.text, |k, on| k.text = on, cx).into_any_element()))
            .child(indent(
                self.kind("logicals", t("Logicals", "Valeurs logiques"), kinds.logicals, |k, on| k.logicals = on, cx)
                    .into_any_element(),
            ))
            .child(indent(
                self.kind("errors", t("Errors", "Erreurs"), kinds.errors, |k, on| k.errors = on, cx).into_any_element(),
            ))
            .child(self.radio(Choice::Blanks, cx))
            .child(self.radio(Choice::CurrentRegion, cx))
            .child(self.radio(Choice::CurrentArray, cx))
            .child(self.radio(Choice::Objects, cx));
        let right = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(self.radio(Choice::RowDifferences, cx))
            .child(self.radio(Choice::ColumnDifferences, cx))
            .child(self.radio(Choice::Precedents, cx))
            .child(self.radio(Choice::Dependents, cx))
            .child(indent(
                RadioGroup::new(
                    "levels",
                    vec![t("Direct only", "Directs seulement").into(), t("All levels", "Tous niveaux").into()],
                    Some(self.all_levels as usize),
                )
                .disabled(!levels)
                .on_change(cx.listener(|this, index: &usize, _, cx| {
                    this.all_levels = *index == 1;
                    cx.notify();
                }))
                .into_any_element(),
            ))
            .child(self.radio(Choice::LastCell, cx))
            .child(self.radio(Choice::VisibleCells, cx))
            .child(self.radio(Choice::ConditionalFormats, cx))
            .child(self.radio(Choice::DataValidation, cx))
            .child(indent(
                RadioGroup::new(
                    "rules",
                    vec![t_ctx("rules", "All", "Tous").into(), t("Same", "Identiques").into()],
                    Some(self.same as usize),
                )
                .disabled(!rules)
                .on_change(cx.listener(|this, index: &usize, _, cx| {
                    this.same = *index == 1;
                    cx.notify();
                }))
                .into_any_element(),
            ));
        dialog(&self.modal, t("Go To Special", "Sélectionner les cellules"))
            .width(430.0)
            .on_ok(cx.listener(Self::ok))
            .child(GroupBox::new(t("Select", "Sélectionner")).child(div().flex().gap(px(28.)).child(left).child(right)))
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::Locale;

    fn workbook() -> Workbook {
        let mut wb = Workbook::new(Locale::en());
        let cells = [
            ("A1", "Item"),
            ("B1", "Qty"),
            ("C1", "Total"),
            ("A2", "Pens"),
            ("B2", "12"),
            ("C2", "=B2*2"),
            ("A3", "Ink"),
            ("B3", "3"),
            ("C3", "=B3*2"),
            ("A4", "Caps"),
            ("C4", "=B4*3"),
            ("A6", "TRUE"),
            ("B6", "=1/0"),
            ("C6", "=SUM(C2:C4)"),
            ("D6", "=C6&\"!\""),
        ];
        for (cell, text) in cells {
            let (row, col) = opensheet_engine::refs::parse_cell_name(cell).unwrap();
            wb.set_cell_text(0, row, col, text).unwrap();
        }
        wb.commit("cells");
        wb
    }

    fn selecting(ranges: &[&str]) -> Selection {
        let ranges: Vec<Rect> = ranges.iter().map(|range| Rect::parse(range).unwrap()).collect();
        let last = *ranges.last().unwrap();
        Selection {
            active: (last.r0, last.c0),
            anchor: (last.r0, last.c0),
            ranges,
        }
    }

    fn found(wb: &Workbook, selection: &[&str], what: Special) -> Vec<String> {
        special_cells(wb, 0, &selecting(selection), what).iter().map(|range| range.name()).collect()
    }

    #[test]
    fn the_last_places_left_are_remembered() {
        let mut places = Vec::new();
        for place in ["Sheet1!$A$1", "Sheet1!$B$2", "Sheet1!$A$1", "Sheet2!$C$3", "Sheet1!$D$4", "Sheet1!$E$5"] {
            remember(&mut places, place.to_string());
        }
        // Most recent first, each once, four at most.
        assert_eq!(places, ["Sheet1!$E$5", "Sheet1!$D$4", "Sheet2!$C$3", "Sheet1!$A$1"]);
    }

    #[test]
    fn cells_are_gathered_into_rectangles() {
        let cells: BTreeSet<(u32, u32)> = [(0, 0), (0, 1), (1, 0), (1, 1), (2, 1), (4, 0), (4, 2), (5, 2)].into_iter().collect();
        let names: Vec<String> = coalesce(&cells).iter().map(|range| range.name()).collect();
        assert_eq!(names, ["A1:B2", "B3", "A5", "C5:C6"]);
        assert!(coalesce(&BTreeSet::new()).is_empty());
    }

    #[test]
    fn constants_and_formulas_by_kind() {
        let wb = workbook();
        let only = |numbers, text, logicals, errors| Kinds {
            numbers,
            text,
            logicals,
            errors,
        };
        // One cell selected: the whole sheet.
        assert_eq!(found(&wb, &["A1"], Special::Constants(Kinds::ALL)), ["A1:C1", "A2:B3", "A4", "A6"]);
        assert_eq!(found(&wb, &["A1"], Special::Constants(only(true, false, false, false))), ["B2:B3"]);
        assert_eq!(found(&wb, &["A1"], Special::Constants(only(false, false, true, false))), ["A6"]);
        assert_eq!(found(&wb, &["A1"], Special::Formulas(Kinds::ALL)), ["C2:C4", "B6:D6"]);
        assert_eq!(found(&wb, &["A1"], Special::Formulas(only(false, false, false, true))), ["B6"]);
        assert_eq!(found(&wb, &["A1"], Special::Formulas(only(false, true, false, false))), ["D6"]);
        // Several cells selected: those only.
        assert_eq!(found(&wb, &["A1:C2"], Special::Formulas(Kinds::ALL)), ["C2"]);
        assert_eq!(found(&wb, &["A1:A2", "C1:C3"], Special::Constants(Kinds::ALL)), ["A1:A2", "C1"]);
        assert!(found(&wb, &["E1:F9"], Special::Constants(Kinds::ALL)).is_empty());
    }

    #[test]
    fn blanks_regions_and_last_cell() {
        let wb = workbook();
        assert_eq!(found(&wb, &["A1:C4"], Special::Blanks), ["B4"]);
        assert_eq!(found(&wb, &["A1"], Special::Blanks), ["D1:D4", "B4", "A5:D5"]);
        assert_eq!(found(&wb, &["B3"], Special::CurrentRegion), ["A1:C4"]);
        assert_eq!(found(&wb, &["B3"], Special::LastCell), ["D6"]);
        assert!(found(&wb, &["B3"], Special::CurrentArray).is_empty());
        assert!(found(&wb, &["B3"], Special::Notes).is_empty());
        assert!(found(&Workbook::new(Locale::en()), &["A1"], Special::LastCell).is_empty());
    }

    #[test]
    fn current_array_is_the_spilled_range() {
        let mut wb = Workbook::new(Locale::en());
        wb.set_cell_text(0, 1, 1, "=SEQUENCE(2,3)").unwrap();
        wb.commit("spill");
        assert_eq!(found(&wb, &["C3"], Special::CurrentArray), ["B2:D3"]);
        assert!(found(&wb, &["A1"], Special::CurrentArray).is_empty());
    }

    #[test]
    fn differences_compare_with_the_active_cell() {
        let wb = workbook();
        // Down column C, from C2: C4 multiplies by 3, not 2.
        assert_eq!(found(&wb, &["C2:C4"], Special::ColumnDifferences), ["C4"]);
        // Across row 2, from B2: a formula is not a constant.
        assert_eq!(found(&wb, &["B2:C2"], Special::RowDifferences), ["C2"]);
        // Same constants, same formulas: no difference.
        assert!(found(&wb, &["C2:C3"], Special::ColumnDifferences).is_empty());
        assert!(found(&wb, &["C2"], Special::RowDifferences).is_empty());
    }

    #[test]
    fn precedents_and_dependents_by_level() {
        let wb = workbook();
        assert_eq!(found(&wb, &["C6"], Special::Precedents { all_levels: false }), ["C2:C4"]);
        assert_eq!(found(&wb, &["D6"], Special::Precedents { all_levels: false }), ["C6"]);
        assert_eq!(
            found(&wb, &["D6"], Special::Precedents { all_levels: true }),
            ["C6", "C2:C4", "B2", "B3", "B4"]
        );
        assert_eq!(found(&wb, &["B2"], Special::Dependents { all_levels: false }), ["C2"]);
        assert_eq!(found(&wb, &["B2"], Special::Dependents { all_levels: true }), ["C2", "C6", "D6"]);
        assert!(found(&wb, &["A1"], Special::Precedents { all_levels: true }).is_empty());
        assert!(found(&wb, &["D6"], Special::Dependents { all_levels: true }).is_empty());
    }

    #[test]
    fn visible_cells_leave_hidden_rows_and_columns_out() {
        let mut wb = workbook();
        wb.set_rows_hidden(0, 2, 2, true);
        wb.set_cols_hidden(0, 1, 1, true);
        wb.commit("hide");
        assert_eq!(found(&wb, &["A1:C4"], Special::VisibleCells), ["A1:A2", "C1:C2", "A4", "C4"]);
        assert_eq!(found(&wb, &["A1:A2"], Special::VisibleCells), ["A1:A2"]);
    }
}
