//! Find and Replace: the dialog, and the search of the find bar. The
//! searching itself is the engine's (`find.rs`); here are the choices the
//! user makes, where a search starts and what Excel says when it ends.

use super::named;
use crate::app::ExcelApp;
use crate::features::{ChangeCause, Cx, Intent};
use crate::i18n::t;
use crate::shell::{Alert, Modal};
use crate::text_input::TextInput;
use crate::theme::{self, c};
use crate::widgets::{self, Button, Checkbox, ListBox, Select, TabStrip, TextField, dialog, label};
use gpui::{Context, Entity, Focusable, SharedString, Window, div, prelude::*, px};
use opensheet_engine::Rect;
use opensheet_engine::find::{BrokenFormula, FindOptions, Found, LookIn, Place, Search};
use opensheet_engine::protect::Action;
use std::rc::Rc;

/// The choices of the dialog, kept from one search to the next as Excel
/// keeps them; the find bar searches with them too.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Choices {
    pub find: String,
    pub replace: String,
    pub options: FindOptions,
    pub workbook: bool,
}

impl Default for Choices {
    fn default() -> Self {
        Choices {
            find: String::new(),
            replace: String::new(),
            // Excel looks in formulas until told otherwise.
            options: FindOptions {
                look_in: LookIn::Formulas,
                ..FindOptions::default()
            },
            workbook: false,
        }
    }
}

pub(super) fn choices(app: &ExcelApp) -> Choices {
    app.ext::<Choices>().cloned().unwrap_or_default()
}

/// The ranges a search keeps to: the selection when it is more than one
/// cell, as in Excel.
fn within(app: &ExcelApp) -> Vec<Rect> {
    let sel = app.sel();
    let sheet = &app.wb.sheets[app.sheet];
    let one_cell = match sel.ranges.as_slice() {
        [range] => range.is_single() || sheet.merge_at(sel.active.0, sel.active.1) == Some(*range),
        _ => false,
    };
    if one_cell {
        Vec::new()
    } else {
        sel.ranges.clone()
    }
}

/// Make a cell found the active cell: inside the selection when the
/// search keeps to it, alone otherwise, on its own sheet.
fn show(app: &mut ExcelApp, place: Place, window: &mut Window, cx: &mut Cx) {
    let (sheet, row, col) = place;
    if sheet != app.sheet {
        app.switch_sheet(sheet, window, cx);
    }
    if !within(app).is_empty() && app.sel().contains(row, col) {
        let view = &mut app.views[sheet];
        // The range holding the cell becomes the current one.
        if let Some(index) = view
            .sel
            .ranges
            .iter()
            .position(|range| range.contains(row, col))
        {
            let range = view.sel.ranges.remove(index);
            view.sel.ranges.push(range);
        }
        view.sel.active = (row, col);
        app.ensure_visible(row, col);
        app.refresh(cx);
    } else {
        app.select_cell(row, col, cx);
    }
}

fn not_found() -> &'static str {
    t(
        "OpenSheet cannot find the data you're searching for. If you are certain the data exists in the current sheet, check what you typed and try again.",
        "OpenSheet ne trouve pas les données que vous recherchez. Si vous êtes sûr qu’elles sont présentes dans la feuille actuelle, contrôlez ce que vous avez tapé et réessayez.",
    )
}

fn nothing_to_replace() -> &'static str {
    t(
        "OpenSheet cannot find matching data to replace. No cell in the selection contains what you typed, or no records match the criteria.",
        "OpenSheet ne trouve pas de données correspondantes à remplacer. Aucune cellule de la sélection ne contient ce que vous avez tapé ou bien aucun enregistrement ne correspond aux critères.",
    )
}

fn broken_formula(app: &mut ExcelApp, broken: &BrokenFormula, cx: &mut Cx) {
    let message = format!(
        "{}\n\n({} {})",
        t(
            "There's a problem with this formula.\nNot trying to type a formula? When the first character is an equal (=) or minus (-) sign, it is treated as a formula. To avoid this, type an apostrophe (') first.",
            "Un problème a été rencontré avec cette formule.\nVous ne souhaitez pas saisir de formule ? Lorsque le premier caractère est le signe égal (=) ou moins (-), il est considéré comme une formule. Pour éviter cela, tapez d'abord une apostrophe (').",
        ),
        broken.error.message,
        broken.text,
    );
    app.alert(Alert::new("OpenSheet", message).warning(), cx);
}

/// Find Next (Find Previous with `backward`). False, after saying so,
/// when the text is nowhere.
pub(super) fn find_next(
    app: &mut ExcelApp,
    choices: &Choices,
    backward: bool,
    window: &mut Window,
    cx: &mut Cx,
) -> bool {
    *app.ext_mut::<Choices>() = choices.clone();
    let ranges = within(app);
    let search = Search {
        needle: &choices.find,
        options: &choices.options,
        workbook: choices.workbook,
        within: &ranges,
    };
    let (row, col) = app.active();
    match app.wb.find_from(&search, (app.sheet, row, col), backward) {
        Some(place) => {
            app.status_message = None;
            show(app, place, window, cx);
            true
        }
        None => {
            app.alert(Alert::new("OpenSheet", not_found()).warning(), cx);
            false
        }
    }
}

/// What Find All lists: the search as it was asked, and the cells that
/// hold the text now. The list is made again whenever the workbook
/// changes, so a replacement never leaves a line that is no longer true.
pub(super) struct Results {
    choices: Choices,
    within: Vec<Rect>,
    /// The sheet the search started from.
    sheet: usize,
    generation: u64,
    /// The workbook, as the Book column names it.
    book: SharedString,
    rows: Rc<Vec<Found>>,
    /// The line last clicked.
    picked: Option<usize>,
}

/// Every cell holding the text, in the order of the search.
pub(super) fn find_all(
    app: &ExcelApp,
    choices: &Choices,
    within: &[Rect],
    sheet: usize,
) -> Vec<Found> {
    let search = Search {
        needle: &choices.find,
        options: &choices.options,
        workbook: choices.workbook,
        within,
    };
    let sheet = sheet.min(app.wb.sheets.len().saturating_sub(1));
    app.wb
        .find_everywhere(&search, sheet)
        .into_iter()
        .map(|place| app.wb.found(place))
        .collect()
}

/// The search of the find bar: the next cell holding `needle`, with the
/// choices last made in the dialog. Says in the status bar when there is
/// none.
pub fn find_bar_next(app: &mut ExcelApp, needle: &str, cx: &mut Cx) {
    let choices = choices(app);
    let (row, col) = app.active();
    match app
        .wb
        .find_next(app.sheet, (row, col), needle, &choices.options)
    {
        Some((row, col)) => {
            // A match answers an earlier "no match" message.
            app.status_message = None;
            app.select_cell(row, col, cx);
        }
        None => app.set_status(
            t("No match for “{}”", "Aucune occurrence de « {} »").replace("{}", needle),
            cx,
        ),
    }
}

/// The Replace button: replace in the active cell when it holds the
/// text, then go on to the next cell that does.
pub(super) fn replace(app: &mut ExcelApp, choices: &Choices, window: &mut Window, cx: &mut Cx) {
    *app.ext_mut::<Choices>() = choices.clone();
    let ranges = within(app);
    let options = FindOptions {
        look_in: LookIn::Formulas,
        ..choices.options.clone()
    };
    let search = Search {
        needle: &choices.find,
        options: &options,
        workbook: choices.workbook,
        within: &ranges,
    };
    let (row, col) = app.active();
    let here = (app.sheet, row, col);
    let in_scope = ranges.is_empty() || ranges.iter().any(|range| range.contains(row, col));
    if in_scope
        && app
            .wb
            .replacement_text(here, &search, &choices.replace)
            .is_some()
    {
        if !app.may(Intent::EditCells(Rect::cell(row, col)), cx) {
            return;
        }
        match app.wb.replace_at(here, &search, &choices.replace) {
            Ok(_) => app.commit_cells(
                t("Replace", "Remplacer"),
                &[Rect::cell(row, col)],
                ChangeCause::Typing,
                cx,
            ),
            Err(broken) => {
                broken_formula(app, &broken, cx);
                return;
            }
        }
        // On to the next one, quietly when this was the last.
        if let Some(place) = app.wb.find_from(&search, here, false) {
            show(app, place, window, cx);
        }
        return;
    }
    match app.wb.find_from(&search, here, false) {
        Some(place) => show(app, place, window, cx),
        None => app.alert(Alert::new("OpenSheet", nothing_to_replace()).warning(), cx),
    }
}

/// Replace All, in one undo step, then Excel's account of it.
pub(super) fn replace_all(app: &mut ExcelApp, choices: &Choices, window: &mut Window, cx: &mut Cx) {
    *app.ext_mut::<Choices>() = choices.clone();
    let ranges = within(app);
    let search = Search {
        needle: &choices.find,
        options: &choices.options,
        workbook: choices.workbook,
        within: &ranges,
    };
    let sheet = app.sheet;
    // What the features protect on this sheet (the cells of a pivot
    // table) stops the replacement; locked cells are passed over.
    let formulas = FindOptions {
        look_in: LookIn::Formulas,
        ..choices.options.clone()
    };
    let hits = app.wb.find_everywhere(
        &Search {
            options: &formulas,
            ..search
        },
        sheet,
    );
    let mut locked: Option<Place> = None;
    for place in hits {
        let cell = [Rect::cell(place.1, place.2)];
        let editable = app
            .wb
            .permits(&Action::EditCells {
                sheet: place.0,
                ranges: &cell,
            })
            .is_ok();
        if !editable {
            locked.get_or_insert(place);
        } else if place.0 == sheet && !app.may(Intent::EditCells(cell[0]), cx) {
            return;
        }
    }
    match app.wb.replace_everywhere(&search, sheet, &choices.replace) {
        Err(broken) => {
            show(app, broken.place, window, cx);
            broken_formula(app, &broken, cx);
        }
        Ok(done) if done.cells == 0 => {
            // Every cell holding the text is locked: Excel's message for
            // a protected sheet.
            let message: SharedString = match locked {
                Some((sheet, row, col)) => {
                    let cell = [Rect::cell(row, col)];
                    match app.wb.permits(&Action::EditCells {
                        sheet,
                        ranges: &cell,
                    }) {
                        Err(denied) => denied.message(app.wb.locale).into(),
                        Ok(()) => nothing_to_replace().into(),
                    }
                }
                None => nothing_to_replace().into(),
            };
            app.alert(Alert::new("OpenSheet", message).warning(), cx);
        }
        Ok(done) => {
            app.commit(t("Replace All", "Remplacer tout"), cx);
            let message = t(
                "All done. We made {} replacements.",
                "Cette action est terminée, {} remplacements ont été effectués.",
            )
            .replace("{}", &done.cells.to_string());
            app.alert(Alert::new("OpenSheet", message), cx);
        }
    }
}

// ------------------------------------------------------------------- dialog

pub(super) fn open(app: &mut ExcelApp, replace: bool, window: &mut Window, cx: &mut Cx) {
    let choices = choices(app);
    app.open_modal(window, cx, move |modal, _, cx| FindReplace {
        modal,
        replacing: replace,
        find: widgets::input(cx, choices.find.clone()),
        replace: widgets::input(cx, choices.replace.clone()),
        options: choices.options,
        workbook: choices.workbook,
        placed: false,
        results: None,
    });
}

pub(super) struct FindReplace {
    modal: Modal,
    /// The Replace tab is shown.
    replacing: bool,
    find: Entity<TextInput>,
    replace: Entity<TextInput>,
    options: FindOptions,
    workbook: bool,
    /// The keyboard was given to "Find what" (the dialog opens there, not
    /// on its tabs).
    placed: bool,
    /// The list of Find All, once it was asked for.
    results: Option<Results>,
}

/// The entries of "Look in" on each tab: a replacement always works on
/// formulas.
fn look_in_choices(replacing: bool) -> Vec<LookIn> {
    if replacing {
        vec![LookIn::Formulas]
    } else {
        vec![LookIn::Formulas, LookIn::Values, LookIn::Comments]
    }
}

fn look_in_label(look_in: LookIn) -> &'static str {
    match look_in {
        LookIn::Formulas => t("Formulas", "Formules"),
        LookIn::Values => t("Values", "Valeurs"),
        LookIn::Comments => t("Notes", "Notes"),
    }
}

impl FindReplace {
    /// The cells of the Find All list, for the tests: `None` while the
    /// dialog shows no list.
    #[cfg(test)]
    pub(super) fn listed(&self) -> Option<Vec<Place>> {
        self.results
            .as_ref()
            .map(|results| results.rows.iter().map(|found| found.place).collect())
    }

    fn choices(&self, cx: &Context<Self>) -> Choices {
        let mut options = self.options.clone();
        if self.replacing {
            options.look_in = LookIn::Formulas;
        }
        Choices {
            find: self.find.read(cx).text().to_string(),
            replace: self.replace.read(cx).text().to_string(),
            options,
            workbook: self.workbook,
        }
    }

    fn find_next(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        self.step(false, window, cx);
    }

    fn step(&mut self, backward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let choices = self.choices(cx);
        if choices.find.is_empty() {
            return;
        }
        self.modal
            .update_app(cx, |app, cx| find_next(app, &choices, backward, window, cx));
    }

    /// Find All: list every cell holding the text under the dialog.
    fn find_all(&mut self, cx: &mut Context<Self>) {
        let choices = self.choices(cx);
        if choices.find.is_empty() {
            return;
        }
        self.results = self
            .modal
            .update_app(cx, |app, cx| {
                *app.ext_mut::<Choices>() = choices.clone();
                let within = within(app);
                let rows = find_all(app, &choices, &within, app.sheet);
                if rows.is_empty() {
                    app.alert(Alert::new("OpenSheet", not_found()).warning(), cx);
                    return None;
                }
                Some(Results {
                    choices,
                    within,
                    sheet: app.sheet,
                    generation: app.wb.generation,
                    book: app.title.clone().into(),
                    rows: Rc::new(rows),
                    picked: None,
                })
            })
            .flatten();
        cx.notify();
    }

    /// Search again when the workbook changed under the list (a
    /// replacement, an undo): the line that was picked stays picked when
    /// its cell still holds the text.
    fn refresh_results(&mut self, cx: &mut Context<Self>) {
        let Some(results) = &mut self.results else {
            return;
        };
        let fresh = self.modal.read_app(cx, |app| {
            (app.wb.generation != results.generation).then(|| {
                let rows = find_all(app, &results.choices, &results.within, results.sheet);
                (
                    rows,
                    app.wb.generation,
                    SharedString::from(app.title.clone()),
                )
            })
        });
        if let Some(Some((rows, generation, book))) = fresh {
            let picked = results
                .picked
                .and_then(|i| results.rows.get(i))
                .map(|found| found.place);
            results.picked =
                picked.and_then(|place| rows.iter().position(|found| found.place == place));
            results.rows = Rc::new(rows);
            results.generation = generation;
            results.book = book;
        }
    }

    /// A click on a line of the list goes to its cell, on its sheet.
    fn pick(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(results) = &mut self.results else {
            return;
        };
        let Some(place) = results.rows.get(index).map(|found| found.place) else {
            return;
        };
        results.picked = Some(index);
        self.modal.update_app(cx, |app, cx| {
            if place.0 < app.wb.sheets.len() {
                show(app, place, window, cx);
            }
        });
        cx.notify();
    }

    /// The list of Find All: Excel's six columns over the count of cells.
    fn results_view(&self, cx: &mut Context<Self>) -> Option<gpui::Div> {
        let results = self.results.as_ref()?;
        let columns: [(&'static str, f32); 6] = [
            (t("Book", "Livre"), 96.0),
            (t("Sheet", "Feuille"), 84.0),
            (t("Name", "Nom"), 78.0),
            (t("Cell", "Cellule"), 64.0),
            (t("Value", "Valeur"), 124.0),
            (t("Formula", "Formule"), 124.0),
        ];
        let mut header = div()
            .flex()
            .px(px(7.))
            .h(px(22.))
            .items_center()
            .bg(c(theme::HOVER))
            .border_1()
            .border_color(c(theme::BORDER))
            .text_size(px(12.));
        for (title, width) in columns {
            header = header.child(div().w(px(width)).flex_none().truncate().child(title));
        }
        let rows = results.rows.clone();
        let book = results.book.clone();
        let cells: Vec<SharedString> = rows.iter().map(|found| found.cell.clone().into()).collect();
        let list = ListBox::new("find-results", cells, results.picked)
            .height(132.0)
            .row(move |i, _| {
                let found = &rows[i];
                let texts: [SharedString; 6] = [
                    book.clone(),
                    found.sheet.clone().into(),
                    found.name.clone().into(),
                    found.cell.clone().into(),
                    found.value.clone().into(),
                    found.formula.clone().into(),
                ];
                let mut line = div()
                    .flex()
                    .debug_selector(move || format!("find-result-{i}"));
                for (text, (_, width)) in texts.into_iter().zip(columns) {
                    line = line.child(
                        div()
                            .w(px(width))
                            .flex_none()
                            .pr(px(6.))
                            .truncate()
                            .child(text),
                    );
                }
                line.into_any_element()
            })
            .on_change(
                cx.listener(|this, index: &usize, window, cx| this.pick(*index, window, cx)),
            );
        let count = format!(
            "{} {}",
            t("cell(s) found:", "cellule(s) trouvée(s):"),
            results.rows.len()
        );
        Some(
            div()
                .flex()
                .flex_col()
                .child(header)
                .child(list)
                .child(div().pt(px(6.)).child(label(count))),
        )
    }

    fn replace_one(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let choices = self.choices(cx);
        if choices.find.is_empty() {
            return;
        }
        self.modal
            .update_app(cx, |app, cx| replace(app, &choices, window, cx));
    }

    fn replace_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let choices = self.choices(cx);
        if choices.find.is_empty() {
            return;
        }
        self.modal
            .update_app(cx, |app, cx| replace_all(app, &choices, window, cx));
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // What was typed is there again next time.
        let choices = self.choices(cx);
        self.modal
            .update_app(cx, |app, _| *app.ext_mut::<Choices>() = choices);
        self.modal.close(window, cx);
    }
}

impl Render for FindReplace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.placed {
            self.placed = true;
            let handle = self.find.read(cx).focus_handle(cx);
            window.focus(&handle);
            self.find.update(cx, |input, cx| input.select_all_text(cx));
        }
        self.refresh_results(cx);
        let results = self.results_view(cx);
        let replacing = self.replacing;
        let empty = self.find.read(cx).text().is_empty();
        let looks = look_in_choices(replacing);
        let look_at = looks
            .iter()
            .position(|look| *look == self.options.look_in)
            .unwrap_or(0);
        let look_labels: Vec<SharedString> = looks
            .iter()
            .map(|look| look_in_label(*look).into())
            .collect();
        // The labels share a column wide enough for the longest of them.
        let field = |text: &'static str, control: gpui::AnyElement| {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(label(text).w(px(124.)))
                .child(control)
        };
        let choice = |text: &'static str, control: Select| {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(label(text).w(px(124.)))
                .child(control.width(150.0))
        };
        let within = Select::new(
            "within",
            vec![
                t("Sheet", "Feuille").into(),
                t("Workbook", "Classeur").into(),
            ],
            Some(self.workbook as usize),
        )
        .on_change(cx.listener(|this, index: &usize, _, cx| {
            this.workbook = *index == 1;
            cx.notify();
        }));
        let order = Select::new(
            "order",
            vec![
                t("By Rows", "Par ligne").into(),
                t("By Columns", "Par colonne").into(),
            ],
            Some(self.options.by_columns as usize),
        )
        .on_change(cx.listener(|this, index: &usize, _, cx| {
            this.options.by_columns = *index == 1;
            cx.notify();
        }));
        let look_in = Select::new("look-in", look_labels, Some(look_at)).on_change(cx.listener(
            move |this, index: &usize, _, cx| {
                if let Some(look) = look_in_choices(this.replacing).get(*index) {
                    this.options.look_in = *look;
                }
                cx.notify();
            },
        ));
        let checks = div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(
                Checkbox::new(
                    "match-case",
                    t("Match case", "Respecter la casse"),
                    self.options.match_case,
                )
                .on_change(cx.listener(|this, on: &bool, _, cx| {
                    this.options.match_case = *on;
                    cx.notify();
                })),
            )
            .child(
                Checkbox::new(
                    "whole-cell",
                    t("Find entire cells only", "Cellule entière"),
                    self.options.whole_cell,
                )
                .on_change(cx.listener(|this, on: &bool, _, cx| {
                    this.options.whole_cell = *on;
                    cx.notify();
                })),
            );
        let mut footer = div().w_full().flex().items_center().gap(px(8.));
        if replacing {
            footer = footer
                .child(named(
                    "find-replace-all",
                    Button::new("replace-all", t("Replace All", "Remplacer tout"))
                        .disabled(empty)
                        .on_click(cx.listener(|this, _, window, cx| this.replace_all(window, cx))),
                ))
                .child(named(
                    "find-replace-one",
                    Button::new("replace-one", t("Replace", "Remplacer"))
                        .disabled(empty)
                        .on_click(cx.listener(|this, _, window, cx| this.replace_one(window, cx))),
                ));
        }
        footer = footer
            .child(div().flex_1())
            .child(named(
                "find-all",
                Button::new("find-all", t("Find All", "Rechercher tout"))
                    .disabled(empty)
                    .on_click(cx.listener(|this, _, _, cx| this.find_all(cx))),
            ))
            .child(named(
                "find-close",
                Button::new("close", t("Close", "Fermer"))
                    .on_click(cx.listener(|this, _, window, cx| this.close(window, cx))),
            ))
            .child(named(
                "find-previous",
                Button::new("find-previous", t("Previous", "Précédent"))
                    .disabled(empty)
                    .on_click(cx.listener(|this, _, window, cx| this.step(true, window, cx))),
            ))
            .child(named(
                "find-next",
                Button::new("find-next", t("Find Next", "Suivant"))
                    .primary()
                    .disabled(empty)
                    .on_click(cx.listener(Self::find_next)),
            ));
        dialog(&self.modal, t("Find & Replace", "Rechercher et remplacer"))
            .width(640.0)
            .on_ok(cx.listener(Self::find_next))
            .on_cancel(cx.listener(|this, _, window, cx| this.close(window, cx)))
            .child(
                TabStrip::new(
                    "tabs",
                    vec![
                        t("Find", "Rechercher").into(),
                        t("Replace", "Remplacer").into(),
                    ],
                    replacing as usize,
                )
                .on_change(cx.listener(|this, index: &usize, _, cx| {
                    this.replacing = *index == 1;
                    cx.notify();
                })),
            )
            .child(field(
                t("Find what:", "Rechercher :"),
                TextField::new(&self.find).width(480.0).into_any_element(),
            ))
            .when(replacing, |d| {
                d.child(field(
                    t("Replace with:", "Remplacer par :"),
                    TextField::new(&self.replace)
                        .width(480.0)
                        .into_any_element(),
                ))
            })
            .child(
                div()
                    .flex()
                    .gap(px(24.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .child(choice(t("Within:", "Dans :"), within))
                            .child(choice(t("Search:", "Sens :"), order))
                            .child(choice(t("Look in:", "Regarder dans :"), look_in)),
                    )
                    .child(checks),
            )
            .children(results)
            .footer(footer)
    }
}
