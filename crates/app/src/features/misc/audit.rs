//! Formula auditing: Show Formulas, the tracer arrows, Error Checking,
//! Evaluate Formula and the Watch Window.

use super::links;
use crate::app::{EditMode, EditSource, ExcelApp};
use crate::features::{Cx, PaintLayer, Painter, ViewMode};
use crate::grid::{Axis, GridLayout};
use crate::i18n::{t, t_ctx};
use crate::shell::{Alert, Modal, RefStyle};
use crate::theme::{self, c};
use crate::widgets::{Button, ListBox, dialog, label};
use gpui::{
    AnyElement, Context, FontStyle, FontWeight, HighlightStyle, SharedString, StyledText, UnderlineStyle, WeakEntity,
    Window, div, prelude::*, px,
};
use opensheet_engine::eval::Calc;
use opensheet_engine::format::format_value;
use opensheet_engine::functions::{self, Kind};
use opensheet_engine::refs::cell_name;
use opensheet_engine::{Content, ErrorKind, Expr, Locale, MAX_COLS, Rect, Reference, Sheet, Value, Workbook};
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashSet};
use std::ops::Range;
use std::sync::Arc;

/// What auditing keeps for the open workbook.
#[derive(Default)]
pub(crate) struct Audit {
    /// Names of the sheets showing formulas instead of values.
    formula_view: HashSet<String>,
    /// The tracer arrows; pruned while painting, hence the cell.
    arrows: RefCell<Vec<Arrow>>,
    /// Workbook generation the arrows were last checked against.
    checked: Cell<u64>,
    /// Errors the user asked Error Checking to leave alone (sheet name,
    /// row, column).
    ignored: HashSet<(String, u32, u32)>,
    /// The cells of the Watch Window (sheet name, row, column).
    watches: Vec<(String, u32, u32)>,
}

const WATCH_PANE: &str = "watch-window";

fn tell(app: &mut ExcelApp, message: &'static str, cx: &mut Cx) {
    app.alert(Alert::new("OpenSheet", message), cx);
}

/// `Sheet1!$B$4`, as the dialogs name a cell.
fn cell_label(wb: &Workbook, sheet: usize, row: u32, col: u32) -> String {
    let style = RefStyle {
        absolute: true,
        sheet: true,
        equals: false,
    };
    let name = wb.sheets.get(sheet).map(|s| s.name.as_str()).unwrap_or_default();
    style.format(Rect::cell(row, col), name)
}

// ------------------------------------------------------------ Show Formulas

/// Whether the current sheet shows formulas instead of their results.
pub fn shows_formulas(app: &ExcelApp) -> bool {
    app.ext::<Audit>()
        .is_some_and(|audit| audit.formula_view.contains(&app.wb.sheets[app.sheet].name))
}

pub(super) fn toggle_formulas(app: &mut ExcelApp, cx: &mut Cx) {
    let name = app.wb.sheets[app.sheet].name.clone();
    let view = &mut app.ext_mut::<Audit>().formula_view;
    if !view.remove(&name) {
        view.insert(name);
    }
    app.refresh(cx);
}

/// What a cell shows while formulas are shown: its formula as typed
/// (nothing for a formula hidden by the protection of the sheet). `None`
/// for a cell without a formula, which shows its value unformatted.
pub fn shown_formula(wb: &Workbook, sheet: usize, row: u32, col: u32) -> Option<String> {
    let text = wb.formula_text(sheet, row, col)?;
    Some(if wb.is_formula_hidden(sheet, row, col) { String::new() } else { text })
}

/// The columns as Show Formulas lays them out: twice as wide, as in Excel.
fn doubled_columns(sheet: &Sheet, zoom: f32) -> Axis {
    // The width of the columns nobody resized (see `GridLayout::new`).
    let plain = (0..MAX_COLS)
        .rev()
        .find(|col| !sheet.col_widths.contains_key(col) && !sheet.hidden_cols.contains(col))
        .unwrap_or(MAX_COLS - 1);
    let wide = |width: f32| (2.0 * width * zoom).round();
    let mut sizes: BTreeMap<u32, f32> = sheet.col_widths.iter().map(|(col, width)| (*col, wide(*width))).collect();
    for col in &sheet.hidden_cols {
        sizes.insert(*col, 0.0);
    }
    Axis::new(wide(sheet.col_width(plain)).max(1.0), MAX_COLS, sizes.into_iter())
}

/// `Feature::adjust_layout`: widen the columns of a sheet showing formulas.
/// Not in the Page Layout view, whose pages are laid out on the widths the
/// sheet prints with.
pub(super) fn adjust_layout(app: &ExcelApp, layout: &mut GridLayout) {
    if shows_formulas(app) && app.view_mode() != ViewMode::PageLayout {
        layout.cols = doubled_columns(&app.wb.sheets[app.sheet], layout.zoom);
        layout.frozen_w = layout.cols.start(layout.frozen_cols) as f32;
    }
}

// ------------------------------------------------------------ tracer arrows

/// The command an arrow was drawn by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Trace {
    Precedents,
    Dependents,
}

/// A tracer arrow: from cells a formula reads to the cell of the formula.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Arrow {
    /// Sheet the arrow is drawn on.
    pub sheet: usize,
    /// The cells read; `None`: cells of another sheet.
    pub from: Option<Rect>,
    /// The cell holding the formula; `None`: a formula of another sheet.
    pub to: Option<(u32, u32)>,
    pub trace: Trace,
}

/// The arrows of Trace Precedents from a cell, `depth` levels deep: one
/// from each range its formula reads, then from what the formulas in
/// those ranges read, and so on.
pub(crate) fn precedent_arrows(wb: &Workbook, sheet: usize, cell: (u32, u32), depth: u32) -> Vec<Arrow> {
    let mut arrows: Vec<Arrow> = Vec::new();
    let mut seen: HashSet<(u32, u32)> = HashSet::from([cell]);
    let mut frontier = vec![cell];
    let Some(sh) = wb.sheets.get(sheet) else {
        return arrows;
    };
    for _ in 0..depth {
        let mut next: Vec<(u32, u32)> = Vec::new();
        for to in frontier {
            let mut elsewhere = false;
            for reference in links::precedents(wb, sheet, to.0, to.1) {
                if reference.sheet != sheet {
                    elsewhere = true;
                    continue;
                }
                let from = wb.clamp_to_used(sheet, reference.rect);
                arrows.push(Arrow {
                    sheet,
                    from: Some(from),
                    to: Some(to),
                    trace: Trace::Precedents,
                });
                for ((row, col), source) in sh.cells.range((from.r0, 0)..=(from.r1, u32::MAX)) {
                    let inside = *col >= from.c0 && *col <= from.c1;
                    if inside && matches!(source.content, Content::Formula(_)) && seen.insert((*row, *col)) {
                        next.push((*row, *col));
                    }
                }
            }
            if elsewhere {
                arrows.push(Arrow {
                    sheet,
                    from: None,
                    to: Some(to),
                    trace: Trace::Precedents,
                });
            }
        }
        frontier = next;
    }
    arrows
}

/// The arrows of Trace Dependents from a cell, `depth` levels deep.
pub(crate) fn dependent_arrows(wb: &Workbook, sheet: usize, cell: (u32, u32), depth: u32) -> Vec<Arrow> {
    let mut arrows: Vec<Arrow> = Vec::new();
    let mut seen: HashSet<(u32, u32)> = HashSet::from([cell]);
    let mut frontier = vec![cell];
    for _ in 0..depth {
        let mut next: Vec<(u32, u32)> = Vec::new();
        for from in frontier {
            let mut elsewhere = false;
            for (on, row, col) in links::dependents(wb, Reference::cell(sheet, from.0, from.1)) {
                if on != sheet {
                    elsewhere = true;
                    continue;
                }
                arrows.push(Arrow {
                    sheet,
                    from: Some(Rect::cell(from.0, from.1)),
                    to: Some((row, col)),
                    trace: Trace::Dependents,
                });
                if seen.insert((row, col)) {
                    next.push((row, col));
                }
            }
            if elsewhere {
                arrows.push(Arrow {
                    sheet,
                    from: Some(Rect::cell(from.0, from.1)),
                    to: None,
                    trace: Trace::Dependents,
                });
            }
        }
        frontier = next;
    }
    arrows
}

/// Whether what an arrow shows is still true: Excel takes an arrow away
/// when the formula it points at changes or when its cells move.
pub(crate) fn arrow_holds(wb: &Workbook, arrow: &Arrow) -> bool {
    match (arrow.from, arrow.to) {
        (from, Some(to)) => links::precedents(wb, arrow.sheet, to.0, to.1).iter().any(|reference| match from {
            Some(from) if reference.sheet == arrow.sheet => {
                wb.clamp_to_used(arrow.sheet, reference.rect) == from || (from.is_single() && reference.rect.contains(from.r0, from.c0))
            }
            Some(_) => false,
            None => reference.sheet != arrow.sheet,
        }),
        (Some(from), None) => links::dependents(wb, Reference::new(arrow.sheet, from))
            .iter()
            .any(|(on, _, _)| *on != arrow.sheet),
        (None, None) => false,
    }
}

/// What a use of Trace Precedents / Trace Dependents did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Traced {
    /// One more level of arrows is shown.
    Added,
    /// Every level is shown already.
    Complete,
    /// The cell has nothing to trace.
    Nothing,
}

impl Audit {
    /// Drop the arrows the last change of the workbook made untrue.
    fn prune(&self, wb: &Workbook) {
        if self.checked.get() != wb.generation {
            self.checked.set(wb.generation);
            self.arrows.borrow_mut().retain(|arrow| arrow_holds(wb, arrow));
        }
    }

    /// Show one more level of arrows from a cell: the first level whose
    /// arrows are not all there yet.
    pub(crate) fn trace(&mut self, wb: &Workbook, sheet: usize, cell: (u32, u32), kind: Trace) -> Traced {
        self.prune(wb);
        let mut arrows = self.arrows.borrow_mut();
        let mut known = 0;
        // Chains of formulas end; the bound is for circular ones.
        for depth in 1..=256 {
            let wanted = match kind {
                Trace::Precedents => precedent_arrows(wb, sheet, cell, depth),
                Trace::Dependents => dependent_arrows(wb, sheet, cell, depth),
            };
            if wanted.len() == known {
                return if depth == 1 { Traced::Nothing } else { Traced::Complete };
            }
            let missing: Vec<Arrow> = wanted.iter().filter(|arrow| !arrows.contains(arrow)).cloned().collect();
            if !missing.is_empty() {
                arrows.extend(missing);
                return Traced::Added;
            }
            known = wanted.len();
        }
        Traced::Complete
    }

    /// Remove the arrows of a sheet: all of them, or those of one command.
    /// Returns false when there was none.
    pub(crate) fn remove(&mut self, sheet: usize, kind: Option<Trace>) -> bool {
        let mut arrows = self.arrows.borrow_mut();
        let before = arrows.len();
        arrows.retain(|arrow| arrow.sheet != sheet || kind.is_some_and(|kind| arrow.trace != kind));
        arrows.len() != before
    }
}

/// Trace Precedents / Trace Dependents: each use shows one more level.
pub(super) fn trace(app: &mut ExcelApp, kind: Trace, cx: &mut Cx) {
    let sheet = app.sheet;
    let cell = app.active();
    // The state and the workbook are two fields of the application.
    let mut audit = std::mem::take(app.ext_mut::<Audit>());
    let traced = audit.trace(&app.wb, sheet, cell, kind);
    *app.ext_mut::<Audit>() = audit;
    if traced == Traced::Nothing {
        let message = match kind {
            Trace::Precedents => t(
                "The Trace Precedents command requires that the active cell contain a formula which includes valid references.",
                "Pour utiliser la commande Repérer les antécédents, la cellule active doit contenir une formule avec des références valides.",
            ),
            Trace::Dependents => t(
                "The Trace Dependents command found no formulas that refer to the active cell.",
                "La commande Repérer les dépendants n'a pas trouvé de formule faisant référence à la cellule active.",
            ),
        };
        tell(app, message, cx);
        return;
    }
    cx.notify();
}

/// Remove Arrows: all of them, or those of one command.
pub(super) fn remove_arrows(app: &mut ExcelApp, kind: Option<Trace>, cx: &mut Cx) {
    let sheet = app.sheet;
    if !app.ext_mut::<Audit>().remove(sheet, kind) {
        tell(
            app,
            t(
                "The Remove Arrows command found no tracer arrows to remove.",
                "La commande Supprimer les flèches n'a pas trouvé de flèche d'audit à supprimer.",
            ),
            cx,
        );
        return;
    }
    cx.notify();
}

/// Trace Error: the arrows to the cells the error of the active cell
/// comes from.
pub(super) fn trace_error(app: &mut ExcelApp, cx: &mut Cx) {
    let (row, col) = app.active();
    if !app.wb.value_at(app.sheet, row, col).is_error() {
        tell(
            app,
            t(
                "The Trace Error command requires that the active cell contain an error value.",
                "Pour utiliser la commande Repérer une erreur, la cellule active doit contenir une valeur d'erreur.",
            ),
            cx,
        );
        return;
    }
    trace(app, Trace::Precedents, cx);
}

/// Where an arrow touches a cell, given the box of the cell: a third of
/// the way in (never far from the left edge of a wide cell), half-way
/// down.
pub(crate) fn touch_point(cell: (f32, f32, f32, f32), zoom: f32) -> (f32, f32) {
    let (x0, y0, x1, y1) = cell;
    (x0 + ((x1 - x0) * 0.35).min(22.0 * zoom), (y0 + y1) / 2.0)
}

/// The line of an arrow: its start, the tip of its head, and whether it
/// is the dashed arrow of cells on another sheet (which starts from, or
/// points at, a sheet icon set apart from the cell). `from` and `to` are
/// the boxes of the first cell read and of the formula's cell.
pub(crate) fn arrow_line(
    from: Option<(f32, f32, f32, f32)>,
    to: Option<(f32, f32, f32, f32)>,
    zoom: f32,
) -> Option<((f32, f32), (f32, f32), bool)> {
    let reach = 34.0 * zoom.max(0.6);
    match (from, to) {
        (Some(from), Some(to)) => Some((touch_point(from, zoom), touch_point(to, zoom), false)),
        (None, Some(to)) => {
            let end = touch_point(to, zoom);
            Some(((end.0 - reach, end.1 - reach * 0.6), end, true))
        }
        (Some(from), None) => {
            let start = touch_point(from, zoom);
            Some((start, (start.0 + reach, start.1 + reach * 0.6), true))
        }
        (None, None) => None,
    }
}

/// A line of dashes between two points.
fn dashes(p: &mut Painter, from: (f32, f32), to: (f32, f32), color: gpui::Hsla) {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let length = (dx * dx + dy * dy).sqrt();
    if length < 1.0 {
        return;
    }
    let (ux, uy) = (dx / length, dy / length);
    let (dash, gap) = (5.0, 3.0);
    let mut at = 0.0;
    while at < length {
        let end = (at + dash).min(length);
        p.polyline(&[(from.0 + ux * at, from.1 + uy * at), (from.0 + ux * end, from.1 + uy * end)], 1.2, color);
        at += dash + gap;
    }
}

/// The head of an arrow arriving at `tip` from `from`.
fn head(p: &mut Painter, from: (f32, f32), tip: (f32, f32), color: gpui::Hsla) {
    let (dx, dy) = (tip.0 - from.0, tip.1 - from.1);
    let length = (dx * dx + dy * dy).sqrt().max(0.001);
    let (ux, uy) = (dx / length, dy / length);
    let size = (8.0 * p.zoom()).clamp(6.0, 14.0);
    let back = (tip.0 - ux * size, tip.1 - uy * size);
    let half = size * 0.42;
    p.polygon(&[tip, (back.0 - uy * half, back.1 + ux * half), (back.0 + uy * half, back.1 - ux * half)], color);
}

/// The little sheet that stands for cells of another sheet.
fn sheet_icon(p: &mut Painter, at: (f32, f32)) {
    let (w, h) = (15.0, 11.0);
    let (x0, y0) = (at.0 - w / 2.0, at.1 - h / 2.0);
    p.rect(x0, y0, x0 + w, y0 + h, c(0xFFFFFF));
    p.frame(x0, y0, x0 + w, y0 + h, 1.0, c(0x000000));
    p.rect(x0, y0 + 3.0, x0 + w, y0 + 4.0, c(0x000000));
    p.rect(x0 + 5.0, y0 + 3.0, x0 + 6.0, y0 + h, c(0x000000));
    p.rect(x0, y0 + 7.0, x0 + w, y0 + 8.0, c(0x000000));
}

/// `Feature::paint`: the arrows of the current sheet, over its cells.
pub(super) fn paint(p: &mut Painter) {
    if p.layer != PaintLayer::Over {
        return;
    }
    let app = p.app;
    let Some(audit) = app.ext::<Audit>() else {
        return;
    };
    audit.prune(&app.wb);
    let arrows = audit.arrows.borrow();
    let blue = c(0x2B4FE0);
    let black = c(0x000000);
    for arrow in arrows.iter().filter(|arrow| arrow.sheet == app.sheet) {
        let first = |range: Rect| p.cell_box(Rect::cell(range.r0, range.c0));
        let boxes = (arrow.from.map(first), arrow.to.map(|(row, col)| p.cell_box(Rect::cell(row, col))));
        let Some((start, end, dashed)) = arrow_line(boxes.0, boxes.1, p.zoom()) else {
            continue;
        };
        // Nothing of it in this pane.
        let (left, right) = (start.0.min(end.0), start.0.max(end.0));
        let (top, bottom) = (start.1.min(end.1), start.1.max(end.1));
        let range_box = arrow.from.filter(|from| !from.is_single()).map(|from| p.cell_box(from));
        let in_view = |(x0, y0, x1, y1): (f32, f32, f32, f32)| x1 >= p.clip.0 && x0 <= p.clip.2 && y1 >= p.clip.1 && y0 <= p.clip.3;
        if !in_view((left - 16.0, top - 16.0, right + 16.0, bottom + 16.0)) && !range_box.is_some_and(in_view) {
            continue;
        }
        if let Some((x0, y0, x1, y1)) = range_box {
            p.frame(x0 - 1.0, y0 - 1.0, x1, y1, 1.0, blue);
        }
        if dashed {
            dashes(p, start, end, black);
            head(p, start, end, black);
            match arrow.from {
                None => sheet_icon(p, start),
                Some(_) => {
                    p.disc(start.0, start.1, 2.6 * p.zoom().max(0.7), black);
                    sheet_icon(p, (end.0 + 9.0, end.1 + 7.0));
                }
            }
        } else {
            p.polyline(&[start, end], 1.3, blue);
            p.disc(start.0, start.1, 2.6 * p.zoom().max(0.7), blue);
            head(p, start, end, blue);
        }
    }
}

// ----------------------------------------------------------- Error Checking

/// The cells of a sheet whose formula results in an error, in reading
/// order, without those the user chose to ignore.
pub(crate) fn error_cells(wb: &Workbook, sheet: usize, ignored: &HashSet<(String, u32, u32)>) -> Vec<(u32, u32)> {
    let Some(sh) = wb.sheets.get(sheet) else {
        return Vec::new();
    };
    sh.cells
        .iter()
        .filter(|(_, cell)| matches!(cell.content, Content::Formula(_)) && cell.value.is_error())
        .map(|(at, _)| *at)
        .filter(|(row, col)| !ignored.contains(&(sh.name.clone(), *row, *col)))
        .collect()
}

/// The error cell Error Checking goes to from `from`: the next one in
/// reading order (`from` itself when `inclusive`), or the previous one.
pub(crate) fn next_error(errors: &[(u32, u32)], from: (u32, u32), forward: bool, inclusive: bool) -> Option<(u32, u32)> {
    if forward {
        errors.iter().copied().find(|cell| *cell > from || (inclusive && *cell == from))
    } else {
        errors.iter().copied().rev().find(|cell| *cell < from)
    }
}

/// Name and explanation of an error, as Error Checking gives them.
fn error_text(kind: ErrorKind) -> (&'static str, &'static str) {
    match kind {
        ErrorKind::Div0 => (
            t("Divide by Zero Error", "Erreur lors de la division par zéro"),
            t(
                "The formula or function used is dividing by zero or empty cells.",
                "La formule ou la fonction utilisée effectue une division par zéro ou par des cellules vides.",
            ),
        ),
        ErrorKind::Name => (
            t("Invalid Name Error", "Erreur due à un nom non valide"),
            t("The formula contains unrecognized text.", "La formule contient du texte non reconnu."),
        ),
        ErrorKind::Value => (
            t("Error in Value", "Erreur de valeur"),
            t(
                "A value used in the formula is of the wrong data type.",
                "Le type de données d'une valeur utilisée dans la formule est incorrect.",
            ),
        ),
        ErrorKind::Ref => (
            t("Invalid Cell Reference Error", "Erreur due à une référence de cellule non valide"),
            t(
                "Moving or deleting cells caused an invalid cell reference, or function is returning reference error.",
                "Déplacer ou supprimer des cellules provoque une référence de cellule non valide, ou la fonction renvoie une erreur de référence.",
            ),
        ),
        ErrorKind::NA => (
            t("Value Not Available Error", "Erreur due à une valeur non disponible"),
            t(
                "A value is not available to the formula or function.",
                "Une valeur n'est pas disponible pour la formule ou la fonction.",
            ),
        ),
        ErrorKind::Num => (
            t("Number Error", "Erreur de nombre"),
            t(
                "There is a problem with a number used in the formula.",
                "Problème avec un nombre utilisé dans la formule.",
            ),
        ),
        ErrorKind::Null => (
            t("Null Error", "Erreur de valeur nulle"),
            t("The ranges in the formula do not intersect.", "Les plages de la formule ne se coupent pas."),
        ),
        ErrorKind::Spill => (
            t("Spill range isn't blank", "La plage propagée n'est pas vide"),
            t(
                "A cell we need to spill data into isn't blank.",
                "Une cellule dans laquelle nous avons besoin de propager des données n'est pas vide.",
            ),
        ),
        ErrorKind::Calc => (t("Formula Results in Error", "La formule a abouti à une erreur"), ""),
    }
}

fn check_complete(app: &mut ExcelApp, cx: &mut Cx) {
    tell(
        app,
        t(
            "The error check is complete for the entire sheet.",
            "La vérification des erreurs est terminée pour la totalité de la feuille.",
        ),
        cx,
    );
}

fn errors_of(app: &ExcelApp) -> Vec<(u32, u32)> {
    let none = HashSet::new();
    let ignored = app.ext::<Audit>().map(|audit| &audit.ignored).unwrap_or(&none);
    error_cells(&app.wb, app.sheet, ignored)
}

pub(super) fn open_error_checking(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let errors = errors_of(app);
    let Some(first) = next_error(&errors, app.active(), true, true).or(errors.first().copied()) else {
        check_complete(app, cx);
        return;
    };
    app.select_cell(first.0, first.1, cx);
    app.open_modal(window, cx, |modal, _, _| ErrorChecking { modal });
}

/// The dialog always shows the error of the active cell; Previous and
/// Next move the active cell.
struct ErrorChecking {
    modal: Modal,
}

impl ErrorChecking {
    fn step(&mut self, forward: bool, ignore: bool, window: &mut Window, cx: &mut Context<Self>) {
        let modal = self.modal.clone();
        self.modal.update_app(cx, |app, cx| {
            let from = app.active();
            if ignore {
                let name = app.wb.sheets[app.sheet].name.clone();
                app.ext_mut::<Audit>().ignored.insert((name, from.0, from.1));
            }
            match next_error(&errors_of(app), from, forward, false) {
                Some(cell) => app.select_cell(cell.0, cell.1, cx),
                None => {
                    app.close_modal_of(&modal.focus, window, cx);
                    check_complete(app, cx);
                }
            }
        });
        cx.notify();
    }

    fn steps(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| open_evaluate(app, window, cx));
    }

    fn edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.modal.close(window, cx);
        self.modal
            .update_app(cx, |app, cx| app.start_edit(None, EditMode::Edit, EditSource::FormulaBar, window, cx));
    }
}

impl Render for ErrorChecking {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = self.modal.read_app(cx, |app| {
            let (row, col) = app.active();
            let formula = app.wb.formula_bar_text(app.sheet, row, col);
            let kind = app.wb.value_at(app.sheet, row, col).as_error();
            (cell_name(row, col), formula, kind)
        });
        let (cell, formula, kind) = shown.unwrap_or_default();
        let (title, explanation) = kind.map(error_text).unwrap_or_default();
        let button = |id: &'static str, text: &'static str| Button::new(id, text).width(230.0);
        dialog(&self.modal, t("Error Checking", "Vérification des erreurs"))
            .width(540.0)
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(label(t("Error in cell {}", "Erreur dans la cellule {}").replace("{}", &cell)))
                            .child(div().truncate().child(SharedString::from(formula)))
                            .child(div().h(px(6.)))
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                            .child(div().line_height(px(16.)).child(explanation)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(
                                button("steps", t("Show Calculation Steps...", "Afficher les étapes du calcul..."))
                                    .on_click(cx.listener(|this, _, window, cx| this.steps(window, cx))),
                            )
                            .child(
                                button("ignore", t("Ignore Error", "Ignorer l'erreur"))
                                    .on_click(cx.listener(|this, _, window, cx| this.step(true, true, window, cx))),
                            )
                            .child(
                                button("edit", t("Edit in Formula Bar", "Modifier dans la barre de formule"))
                                    .on_click(cx.listener(|this, _, window, cx| this.edit(window, cx))),
                            ),
                    ),
            )
            .footer(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        Button::new("previous", t("Previous", "Précédent"))
                            .on_click(cx.listener(|this, _, window, cx| this.step(false, false, window, cx))),
                    )
                    .child(
                        Button::new("next", t("Next", "Suivant"))
                            .primary()
                            .on_click(cx.listener(|this, _, window, cx| this.step(true, false, window, cx))),
                    ),
            )
    }
}

// --------------------------------------------------------- Evaluate Formula

/// Where a part of an expression is: the child to take at each level.
type Path = Vec<usize>;

fn children(expr: &Expr) -> Vec<&Expr> {
    match expr {
        Expr::Unary(_, e) | Expr::Percent(e) | Expr::Paren(e) | Expr::Spill(e) | Expr::Implicit(e) => vec![e],
        Expr::Binary(_, a, b) | Expr::Intersect(a, b) => vec![a, b],
        Expr::Func(_, args) | Expr::Union(args) => args.iter().collect(),
        _ => Vec::new(),
    }
}

fn node_at<'a>(expr: &'a Expr, path: &[usize]) -> Option<&'a Expr> {
    match path.split_first() {
        None => Some(expr),
        Some((first, rest)) => node_at(children(expr).get(*first).copied()?, rest),
    }
}

/// The expression with the part at `path` replaced.
fn replaced(expr: &Expr, path: &[usize], new: &Expr) -> Expr {
    let Some((first, rest)) = path.split_first() else {
        return new.clone();
    };
    let sub = |child: &Expr, index: usize| Box::new(if index == *first { replaced(child, rest, new) } else { child.clone() });
    let each = |list: &[Expr]| list.iter().enumerate().map(|(i, child)| *sub(child, i)).collect();
    match expr {
        Expr::Unary(op, e) => Expr::Unary(*op, sub(e, 0)),
        Expr::Percent(e) => Expr::Percent(sub(e, 0)),
        Expr::Paren(e) => Expr::Paren(sub(e, 0)),
        Expr::Spill(e) => Expr::Spill(sub(e, 0)),
        Expr::Implicit(e) => Expr::Implicit(sub(e, 0)),
        Expr::Binary(op, a, b) => Expr::Binary(*op, sub(a, 0), sub(b, 1)),
        Expr::Intersect(a, b) => Expr::Intersect(sub(a, 0), sub(b, 1)),
        Expr::Func(name, args) => Expr::Func(name.clone(), each(args)),
        Expr::Union(items) => Expr::Union(each(items)),
        other => other.clone(),
    }
}

fn under(index: usize, path: Option<Path>) -> Option<Path> {
    path.map(|mut path| {
        path.insert(0, index);
        path
    })
}

/// The part of an expression evaluated next: the first one, reading from
/// the left and from the inside, that is not a value yet. `as_value`: the
/// part is wanted for its value (an operand); otherwise a reference is
/// left for its function to read (`SUM(A1:A3)` is one step).
fn next_part(expr: &Expr, as_value: bool) -> Option<Path> {
    match expr {
        Expr::Number(_) | Expr::Text(_) | Expr::Bool(_) | Expr::Error(_) | Expr::Missing | Expr::Value(_) | Expr::Array(_) => None,
        Expr::Ref(_)
        | Expr::Ref3D(_)
        | Expr::RefError(_)
        | Expr::Name(_)
        | Expr::SheetName(..)
        | Expr::Table(_)
        | Expr::Spill(_)
        | Expr::Intersect(..)
        | Expr::Union(_) => as_value.then(Vec::new),
        Expr::Implicit(_) => Some(Vec::new()),
        // A value between parentheses is a value.
        Expr::Paren(inner) => under(0, next_part(inner, as_value)),
        Expr::Unary(_, e) | Expr::Percent(e) => under(0, next_part(e, true)).or(Some(Vec::new())),
        Expr::Binary(_, a, b) => under(0, next_part(a, true))
            .or_else(|| under(1, next_part(b, true)))
            .or(Some(Vec::new())),
        // Only the test of IF is worked out before IF itself, which then
        // gives way to one of its branches.
        Expr::Func(name, args) if &**name == "IF" && !args.is_empty() => under(0, next_part(&args[0], true)).or(Some(Vec::new())),
        Expr::Func(name, args) => {
            // Functions of single values see their arguments as values.
            let scalar = functions::lookup(name).is_some_and(|def| def.kind == Kind::Scalar);
            args.iter()
                .enumerate()
                .find_map(|(i, arg)| under(i, next_part(arg, scalar)))
                .or(Some(Vec::new()))
        }
    }
}

/// A value as a part of an expression (arrays as array constants when
/// they are small enough to read).
fn value_expr(value: Value) -> Expr {
    let literal = |v: &Value| match v {
        Value::Empty => Expr::Number(0.0),
        other => Expr::Value(other.clone()),
    };
    match &value {
        Value::Array(array) if array.len() <= 64 && array.cols > 0 => {
            Expr::Array(array.data.chunks(array.cols).map(|row| row.iter().map(literal).collect()).collect())
        }
        Value::Empty => Expr::Number(0.0),
        _ => Expr::Value(value),
    }
}

/// One formula being evaluated step by step.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Frame {
    pub cell: (usize, u32, u32),
    /// The formula as far as it is evaluated; `None` for a cell without a
    /// formula.
    pub expr: Option<Expr>,
    /// The part the last step worked out.
    pub last: Option<Path>,
}

impl Frame {
    fn of(wb: &Workbook, cell: (usize, u32, u32)) -> Frame {
        let expr = match wb.cell(cell.0, cell.1, cell.2).map(|c| &c.content) {
            Some(Content::Formula(expr)) => Some((**expr).clone()),
            _ => None,
        };
        Frame { cell, expr, last: None }
    }

    /// The part evaluated next.
    pub fn next(&self) -> Option<Path> {
        next_part(self.expr.as_ref()?, true)
    }
}

/// Evaluate Formula: the formula of a cell, and those it steps into.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Evaluation {
    /// The formula of the cell first, then the ones stepped into.
    pub frames: Vec<Frame>,
}

impl Evaluation {
    pub fn start(wb: &Workbook, cell: (usize, u32, u32)) -> Evaluation {
        Evaluation {
            frames: vec![Frame::of(wb, cell)],
        }
    }

    /// Nothing is left to evaluate.
    pub fn finished(&self) -> bool {
        self.frames.len() <= 1 && self.frames.first().is_none_or(|frame| frame.next().is_none())
    }

    /// The cell the next step would read, if it is a single cell holding a
    /// formula: what Step In goes into.
    pub fn step_target(&self, wb: &Workbook) -> Option<(usize, u32, u32)> {
        let frame = self.frames.last()?;
        let part = node_at(frame.expr.as_ref()?, &frame.next()?)?;
        let (sheet, row, col) = frame.cell;
        let [reference] = links::expr_refs(wb, sheet, row, col, part)[..] else {
            return None;
        };
        let is_reference = matches!(part, Expr::Ref(_) | Expr::Name(_) | Expr::SheetName(..));
        let cell = (reference.sheet, reference.rect.r0, reference.rect.c0);
        let formula = matches!(wb.cell(cell.0, cell.1, cell.2).map(|c| &c.content), Some(Content::Formula(_)));
        (is_reference && reference.is_single() && formula).then_some(cell)
    }

    /// Evaluate: work out the underlined part. A formula stepped into
    /// that is finished gives its place back to the one it came from.
    pub fn evaluate(&mut self, wb: &Workbook) {
        let Some(frame) = self.frames.last_mut() else {
            return;
        };
        let (Some(expr), Some(path)) = (frame.expr.clone(), frame.next()) else {
            self.step_out(wb);
            return;
        };
        let Some(part) = node_at(&expr, &path) else {
            return;
        };
        let (sheet, row, col) = frame.cell;
        let value = |e: &Expr| Calc::eval_once(wb, sheet, row, col, e);
        let result = match part {
            Expr::Func(name, args) if &**name == "IF" && !args.is_empty() => {
                // The branch IF takes replaces it, still to be evaluated.
                let branch = match value(&args[0]) {
                    Value::Bool(test) => Some(test),
                    Value::Number(n) => Some(n != 0.0),
                    Value::Empty => Some(false),
                    _ => None,
                }
                .and_then(|test| args.get(if test { 1 } else { 2 }))
                .filter(|branch| !matches!(branch, Expr::Missing));
                match branch {
                    Some(branch) if next_part(branch, true).is_some_and(|path| !path.is_empty()) => {
                        Expr::Paren(Box::new(branch.clone()))
                    }
                    Some(branch) if next_part(branch, true).is_some() => branch.clone(),
                    _ => value_expr(value(part)),
                }
            }
            other => value_expr(value(other)),
        };
        frame.expr = Some(replaced(&expr, &path, &result));
        frame.last = Some(path);
    }

    /// Step In: show the formula of the cell the underlined reference
    /// points at.
    pub fn step_in(&mut self, wb: &Workbook) {
        if let Some(cell) = self.step_target(wb) {
            self.frames.push(Frame::of(wb, cell));
        }
    }

    /// Step Out: back to the formula stepped in from, with the reference
    /// replaced by its value.
    pub fn step_out(&mut self, wb: &Workbook) {
        if self.frames.len() > 1 {
            self.frames.pop();
            self.evaluate(wb);
        }
    }
}

/// A mark no formula holds, to find a part again in the printed text.
const MARK: &str = "\u{E000}\u{E001}";

/// The text of an expression (with its `=`) and the bytes of one of its
/// parts in it.
pub(crate) fn text_and_span(expr: &Expr, path: Option<&[usize]>, locale: &Locale) -> (String, Option<Range<usize>>) {
    let plain = || (format!("={}", expr.to_text(locale)), None);
    let Some((path, part)) = path.and_then(|path| Some((path, node_at(expr, path)?))) else {
        return plain();
    };
    let marked = replaced(expr, path, &Expr::Name(Arc::from(MARK))).to_text(locale);
    let Some(at) = marked.find(MARK) else {
        return plain();
    };
    let part = part.to_text(locale);
    let text = format!("={}{}{}", &marked[..at], part, &marked[at + MARK.len()..]);
    (text, Some(at + 1..at + 1 + part.len()))
}

pub(super) fn open_evaluate(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let (row, col) = app.active();
    let cell = (app.sheet, row, col);
    let evaluation = Evaluation::start(&app.wb, cell);
    app.open_modal(window, cx, move |modal, _, _| Evaluate { modal, cell, evaluation });
}

struct Evaluate {
    modal: Modal,
    cell: (usize, u32, u32),
    evaluation: Evaluation,
}

impl Evaluate {
    /// Run a step that needs the workbook.
    fn with_workbook(&mut self, cx: &mut Context<Self>, step: impl FnOnce(&mut Evaluation, &Workbook)) {
        let mut evaluation = std::mem::take(&mut self.evaluation);
        self.modal.read_app(cx, |app| step(&mut evaluation, &app.wb));
        self.evaluation = evaluation;
        cx.notify();
    }

    fn evaluate(&mut self, _: &(), _: &mut Window, cx: &mut Context<Self>) {
        let cell = self.cell;
        if self.evaluation.finished() {
            // The button reads Restart.
            self.with_workbook(cx, |evaluation, wb| *evaluation = Evaluation::start(wb, cell));
        } else {
            self.with_workbook(cx, |evaluation, wb| evaluation.evaluate(wb));
        }
    }
}

impl Render for Evaluate {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let evaluation = &self.evaluation;
        let shown = self.modal.read_app(cx, |app| {
            let wb = &app.wb;
            let lines: Vec<(String, String, Option<Range<usize>>, Option<Range<usize>>)> = evaluation
                .frames
                .iter()
                .enumerate()
                .map(|(i, frame)| {
                    let (sheet, row, col) = frame.cell;
                    let reference = cell_label(wb, sheet, row, col);
                    let Some(expr) = &frame.expr else {
                        let value = format_value(&wb.value_at(sheet, row, col), "General", wb.locale).text;
                        return (reference, value, None, None);
                    };
                    // Only the formula in front has a next step.
                    let next = if i + 1 == evaluation.frames.len() { frame.next() } else { None };
                    let (text, underlined) = text_and_span(expr, next.as_deref(), wb.locale);
                    let (_, italic) = text_and_span(expr, frame.last.as_deref(), wb.locale);
                    (reference, text, underlined, italic)
                })
                .collect();
            (lines, evaluation.step_target(wb).is_some())
        });
        let (lines, can_step_in) = shown.unwrap_or_default();
        let finished = evaluation.finished();
        let nested = evaluation.frames.len() > 1;
        let front = evaluation.frames.last();
        let note = match front {
            Some(frame) if frame.expr.is_none() => {
                let empty = self
                    .modal
                    .read_app(cx, |app| app.wb.value_at(frame.cell.0, frame.cell.1, frame.cell.2).is_empty())
                    .unwrap_or(true);
                if empty {
                    t("The cell currently being evaluated is empty.", "La cellule en cours d'évaluation est vide.")
                } else {
                    t(
                        "The cell currently being evaluated contains a constant.",
                        "La cellule en cours d'évaluation contient une constante.",
                    )
                }
            }
            _ => t(
                "To show the result of the underlined expression, click Evaluate.  The most recent result appears italicized.",
                "Pour afficher le résultat de l'expression soulignée, cliquez sur Évaluer. Le résultat le plus récent apparaît en italique.",
            ),
        };
        let rows: Vec<AnyElement> = lines
            .into_iter()
            .enumerate()
            .map(|(i, (reference, text, underlined, italic))| {
                let mut highlights: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
                let underline = HighlightStyle {
                    underline: Some(UnderlineStyle {
                        thickness: px(1.),
                        color: None,
                        wavy: false,
                    }),
                    ..Default::default()
                };
                let slanted = HighlightStyle {
                    font_style: Some(FontStyle::Italic),
                    ..Default::default()
                };
                // Highlights go in order and do not overlap: the next
                // step wins over the last result.
                match (underlined, italic) {
                    (Some(next), Some(last)) if last.end <= next.start => highlights.extend([(last, slanted), (next, underline)]),
                    (Some(next), Some(last)) if next.end <= last.start => highlights.extend([(next, underline), (last, slanted)]),
                    (Some(next), _) => highlights.push((next, underline)),
                    (None, Some(last)) => highlights.push((last, slanted)),
                    (None, None) => {}
                }
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        div()
                            .w(px(120.))
                            .flex_none()
                            .pl(px(10.0 * i as f32))
                            .pt(px(4.))
                            .truncate()
                            .child(SharedString::from(reference)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .min_h(px(28.))
                            .px(px(6.))
                            .py(px(4.))
                            .bg(c(0xFFFFFF))
                            .border_1()
                            .border_color(c(theme::BORDER_STRONG))
                            .rounded(px(3.))
                            .line_height(px(18.))
                            .child(StyledText::new(text).with_highlights(highlights)),
                    )
                    .into_any_element()
            })
            .collect();
        dialog(&self.modal, t_ctx("title", "Evaluate Formula", "Évaluation de formule"))
            .width(560.0)
            .on_ok(cx.listener(Self::evaluate))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(div().w(px(120.)).flex_none().child(label(t("Reference:", "Référence :"))))
                    .child(label(t("Evaluation:", "Évaluation :"))),
            )
            .child(div().min_h(px(120.)).flex().flex_col().gap(px(6.)).children(rows))
            .child(div().min_h(px(34.)).line_height(px(16.)).text_color(c(theme::TEXT_MUTED)).child(note))
            .footer(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        Button::new("evaluate", if finished { t("Restart", "Redémarrer") } else { t("Evaluate", "Évaluer") })
                            .primary()
                            .on_click(cx.listener(Self::evaluate)),
                    )
                    .child(
                        Button::new("step-in", t("Step In", "Pas à pas détaillé"))
                            .disabled(!can_step_in)
                            .on_click(cx.listener(|this, _, _, cx| this.with_workbook(cx, |evaluation, wb| evaluation.step_in(wb)))),
                    )
                    .child(
                        Button::new("step-out", t("Step Out", "Pas à pas sortant"))
                            .disabled(!nested)
                            .on_click(cx.listener(|this, _, _, cx| this.with_workbook(cx, |evaluation, wb| evaluation.step_out(wb)))),
                    )
                    .child(Button::new("close", t("Close", "Fermer")).on_click(cx.listener(|this, _, window, cx| {
                        this.modal.close(window, cx);
                    }))),
            )
    }
}

// ------------------------------------------------------------- Watch Window

pub(super) fn watch_open(app: &ExcelApp) -> bool {
    app.pane_open(WATCH_PANE)
}

/// Show or hide the Watch Window, docked beside the grid.
pub(super) fn toggle_watch_window(app: &mut ExcelApp, cx: &mut Cx) {
    if app.pane_open(WATCH_PANE) {
        app.close_pane(WATCH_PANE, cx);
        return;
    }
    let weak = cx.weak_entity();
    let view = cx.new(|_| WatchWindow { app: weak, selected: None });
    app.open_pane(WATCH_PANE, t("Watch Window", "Fenêtre Espion"), view, cx);
}

/// The lines of the Watch Window: sheet, cell, value, formula.
pub(crate) fn watch_lines(wb: &Workbook, watches: &[(String, u32, u32)]) -> Vec<[String; 4]> {
    watches
        .iter()
        .map(|(sheet, row, col)| {
            let cell = cell_name(*row, *col);
            match wb.sheet_index(sheet) {
                Some(index) => [
                    sheet.clone(),
                    cell,
                    wb.display_text(index, *row, *col).text,
                    wb.formula_text(index, *row, *col).unwrap_or_default(),
                ],
                None => [sheet.clone(), cell, wb.locale.error_name(ErrorKind::Ref).to_string(), String::new()],
            }
        })
        .collect()
}

/// Widths of the Sheet, Cell and Value columns; the formula takes the
/// rest.
const WATCH_WIDTHS: [f32; 3] = [70.0, 50.0, 70.0];

fn watch_column(text: SharedString, index: usize) -> gpui::Div {
    div()
        .px(px(4.))
        .when_some(WATCH_WIDTHS.get(index), |d, width| d.w(px(*width)).flex_none())
        .when(index >= WATCH_WIDTHS.len(), |d| d.flex_1().min_w(px(0.)))
        .truncate()
        .child(text)
}

struct WatchWindow {
    app: WeakEntity<ExcelApp>,
    selected: Option<usize>,
}

impl WatchWindow {
    /// Add Watch: the cells of the selection that hold something.
    fn add(&mut self, cx: &mut Context<Self>) {
        self.app
            .update(cx, |app, cx| {
                let sheet = &app.wb.sheets[app.sheet];
                let name = sheet.name.clone();
                let mut cells: Vec<(u32, u32)> = Vec::new();
                for range in &app.sel().ranges {
                    let range = app.wb.clamp_to_used(app.sheet, *range);
                    if range.is_single() {
                        cells.push((range.r0, range.c0));
                        continue;
                    }
                    cells.extend(
                        sheet
                            .cells
                            .range((range.r0, 0)..=(range.r1, u32::MAX))
                            .filter(|((_, col), cell)| *col >= range.c0 && *col <= range.c1 && !matches!(cell.content, Content::Empty))
                            .map(|(at, _)| *at),
                    );
                }
                let watches = &mut app.ext_mut::<Audit>().watches;
                for (row, col) in cells {
                    let watch = (name.clone(), row, col);
                    if !watches.contains(&watch) {
                        watches.push(watch);
                    }
                }
                cx.notify();
            })
            .ok();
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.selected.take() else {
            return;
        };
        self.app
            .update(cx, |app, cx| {
                let watches = &mut app.ext_mut::<Audit>().watches;
                if index < watches.len() {
                    watches.remove(index);
                }
                cx.notify();
            })
            .ok();
    }

    /// A double click goes to the watched cell.
    fn go_to(&mut self, index: usize, cx: &mut Context<Self>) {
        self.app
            .update(cx, |app, cx| {
                let Some((sheet, row, col)) = app.ext::<Audit>().and_then(|audit| audit.watches.get(index).cloned()) else {
                    return;
                };
                if let Some(sheet) = app.wb.sheet_index(&sheet) {
                    crate::features::names::select_areas(app, &[Reference::cell(sheet, row, col)], cx);
                }
            })
            .ok();
    }
}

impl Render for WatchWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lines = self
            .app
            .upgrade()
            .map(|app| {
                let app = app.read(cx);
                let none = Vec::new();
                let watches = app.ext::<Audit>().map(|audit| &audit.watches).unwrap_or(&none);
                watch_lines(&app.wb, watches)
            })
            .unwrap_or_default();
        self.selected = self.selected.filter(|index| *index < lines.len());
        let names: Vec<SharedString> = lines.iter().map(|line| SharedString::from(line[1].clone())).collect();
        let titles = [t("Sheet", "Feuille"), t("Cell", "Cellule"), t("Value", "Valeur"), t("Formula", "Formule")];
        let header = div()
            .h(px(22.))
            .px(px(6.))
            .flex()
            .items_center()
            .bg(c(theme::CHROME_BG))
            .border_1()
            .border_color(c(0xABABAB))
            .rounded_t(px(3.))
            .children(titles.into_iter().enumerate().map(|(i, title)| watch_column(title.into(), i)));
        let rows = std::rc::Rc::new(lines);
        div()
            .p(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(Button::new("add-watch", t("Add Watch...", "Ajouter un espion...")).on_click(cx.listener(|this, _, _, cx| this.add(cx))))
                    .child(
                        Button::new("delete-watch", t("Delete Watch", "Supprimer un espion"))
                            .disabled(self.selected.is_none())
                            .on_click(cx.listener(|this, _, _, cx| this.delete(cx))),
                    ),
            )
            .child(
                div().flex().flex_col().child(header).child(
                    div().flex().child(
                        ListBox::new("watches", names, self.selected)
                            .height(260.0)
                            .row(move |index, _| {
                                div()
                                    .w_full()
                                    .flex()
                                    .items_center()
                                    .children(rows[index].iter().enumerate().map(|(i, text)| watch_column(SharedString::from(text.clone()), i)))
                                    .into_any_element()
                            })
                            .on_change(cx.listener(|this, index: &usize, _, cx| {
                                this.selected = Some(*index);
                                cx.notify();
                            }))
                            .on_activate(cx.listener(|this, index: &usize, _, cx| this.go_to(*index, cx))),
                    ),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::names::NameScope;

    fn workbook() -> Workbook {
        let mut wb = Workbook::new(Locale::en());
        wb.add_sheet(1);
        let cells = [
            (0, "A1", "10"),
            (0, "A2", "5"),
            (0, "A3", "0"),
            (0, "B1", "=A1+A2*2"),
            (0, "B2", "=SUM(A1:A3)"),
            (0, "B3", "=A1/A3"),
            (0, "C1", "=IF(B1>15,B2*2,\"low\")"),
            (0, "C2", "=B1+Sheet2!A1"),
            (0, "C3", "=(A1+A2)%"),
            (1, "A1", "=Sheet1!B2"),
        ];
        for (sheet, cell, text) in cells {
            let (row, col) = opensheet_engine::refs::parse_cell_name(cell).unwrap();
            wb.set_cell_text(sheet, row, col, text).unwrap();
        }
        wb.commit("setup");
        wb
    }

    fn at(cell: &str) -> (u32, u32) {
        opensheet_engine::refs::parse_cell_name(cell).unwrap()
    }

    /// The steps of Evaluate on a cell: each line is the formula with the
    /// part evaluated next between brackets.
    fn steps(wb: &Workbook, cell: &str) -> Vec<String> {
        let (row, col) = at(cell);
        let mut evaluation = Evaluation::start(wb, (0, row, col));
        let mut out = Vec::new();
        for _ in 0..50 {
            let frame = evaluation.frames.last().unwrap();
            let expr = frame.expr.as_ref().unwrap();
            let (text, span) = text_and_span(expr, frame.next().as_deref(), wb.locale);
            out.push(match span {
                Some(span) => format!("{}[{}]{}", &text[..span.start], &text[span.clone()], &text[span.end..]),
                None => text,
            });
            if evaluation.finished() {
                break;
            }
            evaluation.evaluate(wb);
        }
        out
    }

    #[test]
    fn formulas_are_evaluated_inside_out_and_left_to_right() {
        let wb = workbook();
        assert_eq!(
            steps(&wb, "B1"),
            ["=[A1]+A2*2", "=10+[A2]*2", "=10+[5*2]", "=[10+10]", "=20"]
        );
        // A range stays a range for the function that reads it.
        assert_eq!(steps(&wb, "B2"), ["=[SUM(A1:A3)]", "=15"]);
        assert_eq!(steps(&wb, "B3"), ["=[A1]/A3", "=10/[A3]", "=[10/0]", "=#DIV/0!"]);
        // Parentheses stay around their value; a percent sign is a step.
        assert_eq!(
            steps(&wb, "C3"),
            ["=([A1]+A2)%", "=(10+[A2])%", "=([10+5])%", "=[(15)%]", "=0.15"]
        );
    }

    #[test]
    fn if_gives_way_to_the_branch_it_takes() {
        let wb = workbook();
        assert_eq!(
            steps(&wb, "C1"),
            [
                "=IF([B1]>15,B2*2,\"low\")",
                "=IF([20>15],B2*2,\"low\")",
                "=[IF(TRUE,B2*2,\"low\")]",
                "=([B2]*2)",
                "=([15*2])",
                "=(30)"
            ]
        );
        let mut low = workbook();
        low.set_cell_text(0, 0, 0, "1").unwrap();
        low.commit("low");
        assert_eq!(steps(&low, "C1").last().map(String::as_str), Some("=\"low\""));
    }

    #[test]
    fn stepping_in_and_out_of_a_referenced_formula() {
        let wb = workbook();
        let (row, col) = at("C1");
        let mut evaluation = Evaluation::start(&wb, (0, row, col));
        // B1 is underlined: a formula to step into.
        assert_eq!(evaluation.step_target(&wb), Some((0, 0, 1)));
        evaluation.step_in(&wb);
        assert_eq!(evaluation.frames.len(), 2);
        assert_eq!(evaluation.frames[1].cell, (0, 0, 1));
        // A1 is underlined now: a constant, nothing to step into.
        assert_eq!(evaluation.step_target(&wb), None);
        evaluation.evaluate(&wb);
        assert!(!evaluation.finished());
        evaluation.step_out(&wb);
        assert_eq!(evaluation.frames.len(), 1);
        let frame = &evaluation.frames[0];
        assert_eq!(text_and_span(frame.expr.as_ref().unwrap(), None, wb.locale).0, "=IF(20>15,B2*2,\"low\")");
        assert_eq!(frame.last, Some(vec![0, 0]));
        // A formula stepped into and finished steps out by itself.
        let mut auto = Evaluation::start(&wb, (0, row, col));
        auto.step_in(&wb);
        for _ in 0..4 {
            auto.evaluate(&wb);
        }
        assert_eq!(auto.frames.len(), 2);
        auto.evaluate(&wb);
        assert_eq!(auto.frames.len(), 1);
        // A constant has nothing to evaluate.
        let constant = Evaluation::start(&wb, (0, 0, 0));
        assert!(constant.finished() && constant.frames[0].expr.is_none());
    }

    #[test]
    fn arrows_follow_precedents_level_by_level() {
        let wb = workbook();
        let (row, col) = at("C1");
        let arrow = |from: Option<&str>, to: Option<&str>, trace: Trace| Arrow {
            sheet: 0,
            from: from.map(|range| Rect::parse(range).unwrap()),
            to: to.map(at),
            trace,
        };
        let p = Trace::Precedents;
        let one = precedent_arrows(&wb, 0, (row, col), 1);
        assert_eq!(one, [arrow(Some("B1"), Some("C1"), p), arrow(Some("B2"), Some("C1"), p)]);
        let two = precedent_arrows(&wb, 0, (row, col), 2);
        assert_eq!(
            two[2..],
            [
                arrow(Some("A1"), Some("B1"), p),
                arrow(Some("A2"), Some("B1"), p),
                arrow(Some("A1:A3"), Some("B2"), p)
            ]
        );
        // Nothing further: the third level adds nothing.
        assert_eq!(precedent_arrows(&wb, 0, (row, col), 3), two);
        // Cells of another sheet: one arrow from the sheet icon.
        let (row, col) = at("C2");
        assert_eq!(
            precedent_arrows(&wb, 0, (row, col), 1),
            [arrow(Some("B1"), Some("C2"), p), arrow(None, Some("C2"), p)]
        );
        assert!(precedent_arrows(&wb, 0, at("A1"), 3).is_empty());
    }

    #[test]
    fn arrows_follow_dependents_and_stop_holding_when_formulas_change() {
        let mut wb = workbook();
        let d = Trace::Dependents;
        let arrow = |from: &str, to: Option<&str>| Arrow {
            sheet: 0,
            from: Some(Rect::parse(from).unwrap()),
            to: to.map(at),
            trace: d,
        };
        let one = dependent_arrows(&wb, 0, at("A2"), 1);
        assert_eq!(one, [arrow("A2", Some("B1")), arrow("A2", Some("B2")), arrow("A2", Some("C3"))]);
        let two = dependent_arrows(&wb, 0, at("A2"), 2);
        assert!(two.contains(&arrow("B1", Some("C1"))) && two.contains(&arrow("B2", None)));
        assert!(dependent_arrows(&wb, 0, at("C1"), 2).is_empty());
        assert!(two.iter().all(|a| arrow_holds(&wb, a)));

        // B1 no longer reads A2: its arrow goes, the others stay.
        wb.set_cell_text(0, 0, 1, "=A1*3").unwrap();
        wb.commit("edit");
        assert!(!arrow_holds(&wb, &arrow("A2", Some("B1"))));
        assert!(arrow_holds(&wb, &arrow("A2", Some("B2"))));
        assert!(arrow_holds(&wb, &arrow("B2", None)));
        let precedent = Arrow {
            sheet: 0,
            from: Some(Rect::parse("A1:A3").unwrap()),
            to: Some(at("B2")),
            trace: Trace::Precedents,
        };
        assert!(arrow_holds(&wb, &precedent));
        wb.set_cell_text(1, 0, 0, "1").unwrap();
        wb.commit("edit");
        assert!(!arrow_holds(&wb, &arrow("B2", None)));
    }

    #[test]
    fn each_use_of_a_trace_command_shows_one_more_level() {
        let mut wb = workbook();
        let mut audit = Audit::default();
        let c1 = at("C1");
        let count = |audit: &Audit| audit.arrows.borrow().len();
        assert_eq!(audit.trace(&wb, 0, c1, Trace::Precedents), Traced::Added);
        assert_eq!(count(&audit), 2);
        assert_eq!(audit.trace(&wb, 0, c1, Trace::Precedents), Traced::Added);
        assert_eq!(count(&audit), 5);
        // Every level is there: nothing more, and no message.
        assert_eq!(audit.trace(&wb, 0, c1, Trace::Precedents), Traced::Complete);
        assert_eq!(count(&audit), 5);
        // A constant has no precedents, the last formula no dependents.
        assert_eq!(audit.trace(&wb, 0, at("A1"), Trace::Precedents), Traced::Nothing);
        assert_eq!(audit.trace(&wb, 0, c1, Trace::Dependents), Traced::Nothing);
        // Dependents add to what is shown; arrows of another cell too.
        assert_eq!(audit.trace(&wb, 0, at("A2"), Trace::Dependents), Traced::Added);
        assert_eq!(count(&audit), 8);

        // Remove Dependent Arrows leaves the others; a second time there
        // is nothing to remove.
        assert!(audit.remove(0, Some(Trace::Dependents)));
        assert_eq!(count(&audit), 5);
        assert!(!audit.remove(0, Some(Trace::Dependents)));
        assert!(!audit.remove(1, None));

        // Changing a formula takes its arrows away at the next look, and
        // the level it was at is traced again from what is left.
        wb.set_cell_text(0, 0, 1, "=A1*3").unwrap();
        wb.commit("edit");
        assert_eq!(audit.trace(&wb, 0, c1, Trace::Precedents), Traced::Complete);
        assert_eq!(count(&audit), 4);
        assert!(audit.remove(0, None));
        assert_eq!(count(&audit), 0);
        assert_eq!(audit.trace(&wb, 0, c1, Trace::Precedents), Traced::Added);
        assert_eq!(count(&audit), 2);
    }

    #[test]
    fn arrows_run_between_the_cells_or_to_a_sheet_icon() {
        let a = (100.0, 40.0, 164.0, 60.0);
        let b = (300.0, 100.0, 364.0, 120.0);
        // A third of the way into each cell, half-way down.
        assert_eq!(touch_point(a, 1.0), (122.0, 50.0));
        assert_eq!(arrow_line(Some(a), Some(b), 1.0), Some(((122.0, 50.0), (322.0, 110.0), false)));
        // A wide cell is touched near its left edge, and zoom scales it.
        assert_eq!(touch_point((0.0, 0.0, 600.0, 20.0), 1.0).0, 22.0);
        assert_eq!(touch_point((0.0, 0.0, 600.0, 40.0), 2.0), (44.0, 20.0));
        // Another sheet: a dashed line from an icon up and left of the
        // formula, or down and right of the cell read.
        let (start, end, dashed) = arrow_line(None, Some(b), 1.0).unwrap();
        assert!(dashed && end == (322.0, 110.0) && start.0 < end.0 && start.1 < end.1);
        let (start, end, dashed) = arrow_line(Some(a), None, 1.0).unwrap();
        assert!(dashed && start == (122.0, 50.0) && end.0 > start.0 && end.1 > start.1);
        assert_eq!(arrow_line(None, None, 1.0), None);
    }

    #[test]
    fn error_cells_are_visited_in_reading_order() {
        let mut wb = workbook();
        wb.set_cell_text(0, 5, 0, "=NoSuchName").unwrap();
        wb.set_cell_text(0, 5, 2, "#N/A").unwrap();
        wb.commit("errors");
        let mut ignored = HashSet::new();
        let errors = error_cells(&wb, 0, &ignored);
        // The typed #N/A is a constant, not a formula in error.
        assert_eq!(errors, [at("B3"), at("A6")]);
        assert_eq!(next_error(&errors, at("A1"), true, true), Some(at("B3")));
        assert_eq!(next_error(&errors, at("B3"), true, true), Some(at("B3")));
        assert_eq!(next_error(&errors, at("B3"), true, false), Some(at("A6")));
        assert_eq!(next_error(&errors, at("A6"), true, false), None);
        assert_eq!(next_error(&errors, at("A6"), false, false), Some(at("B3")));
        assert_eq!(next_error(&errors, at("B3"), false, false), None);
        ignored.insert(("Sheet1".to_string(), 2, 1));
        assert_eq!(error_cells(&wb, 0, &ignored), [at("A6")]);
        assert_eq!(error_text(ErrorKind::Div0).0, "Divide by Zero Error");
    }

    #[test]
    fn show_formulas_doubles_the_columns_and_shows_what_was_typed() {
        let mut wb = workbook();
        wb.set_col_width(0, 2, Some(100.0));
        wb.set_cols_hidden(0, 4, 4, true);
        wb.commit("columns");
        let sheet = &wb.sheets[0];
        let axis = doubled_columns(sheet, 1.0);
        let normal = sheet.col_width(0);
        assert_eq!(axis.size(0), 2.0 * normal);
        assert_eq!(axis.size(2), 200.0);
        assert_eq!(axis.size(4), 0.0);
        assert_eq!(axis.start(3), (4.0 * normal + 200.0) as f64);
        assert_eq!(doubled_columns(sheet, 1.5).size(2), 300.0);

        assert_eq!(shown_formula(&wb, 0, 0, 1).as_deref(), Some("=A1+A2*2"));
        assert_eq!(shown_formula(&wb, 0, 0, 0), None);
    }

    #[test]
    fn watches_show_value_and_formula() {
        let mut wb = workbook();
        let expr = wb.parse_name_formula("=Sheet1!$A$1", 0, 0, 0).unwrap();
        wb.add_name(NameScope::Workbook, "First", expr, "").unwrap();
        wb.commit("name");
        let watches = [("Sheet1".to_string(), 1, 1), ("Sheet1".to_string(), 0, 0), ("Gone".to_string(), 0, 0)];
        let lines = watch_lines(&wb, &watches);
        assert_eq!(lines[0], ["Sheet1", "B2", "15", "=SUM(A1:A3)"]);
        assert_eq!(lines[1], ["Sheet1", "A1", "10", ""]);
        assert_eq!(lines[2], ["Gone", "A1", "#REF!", ""]);
    }
}
