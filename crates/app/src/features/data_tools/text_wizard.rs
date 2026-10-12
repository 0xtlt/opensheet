//! The Convert Text to Columns Wizard: three steps (the kind of text, the
//! delimiters or the break lines, the format of each column) over a
//! preview of the data.

use super::{typed_range, warn};
use crate::app::ExcelApp;
use crate::features::{ChangeCause, Cx, Intent};
use crate::i18n::t;
use crate::shell::{Alert, Modal, RefStyle};
use crate::text_input::TextInput;
use crate::theme::{self, c};
use crate::widgets::{self, Button, Checkbox, GroupBox, RadioGroup, RefEdit, Select, TextField, dialog, label};
use gpui::{
    Context, Entity, Font, FontStyle, FontWeight, MouseButton, MouseDownEvent, Pixels, Point, SharedString, Window, canvas,
    div, prelude::*, px,
};
use opensheet_engine::Rect;
use opensheet_engine::text_columns::{
    ColumnFormat, DateOrder, Delimiters, Layout, Spec, TextColumnsError, field_count, guess_breaks, guess_delimiters,
    looks_delimited, split_line,
};
use std::cell::Cell;
use std::rc::Rc;

/// Lines of data the preview shows.
const PREVIEW_LINES: usize = 7;
const PREVIEW_W: f32 = 536.0;
const LINE_H: f32 = 16.0;
/// Height of the ruler of step 2 and of the header of step 3.
const HEAD_H: f32 = 18.0;
const PREVIEW_H: f32 = HEAD_H + PREVIEW_LINES as f32 * LINE_H + 6.0;
const PAD: f32 = 6.0;
const FONT_SIZE: f32 = 11.0;
/// A column of the preview is never wider than this many characters.
const MAX_COLUMN: usize = 28;

/// The font of the preview: every character as wide as the others, so
/// that the break lines of fixed-width text fall between characters.
fn mono() -> Font {
    Font {
        family: "Menlo".into(),
        features: Default::default(),
        fallbacks: None,
        weight: FontWeight::NORMAL,
        style: FontStyle::Normal,
    }
}

/// The text qualifiers of step 2, in the order of the list.
const QUALIFIERS: [Option<char>; 3] = [Some('"'), Some('\''), None];

/// How a date order reads in the interface language.
pub(super) fn date_order_label(order: DateOrder) -> &'static str {
    match order {
        DateOrder::MDY => t("MDY", "MJA"),
        DateOrder::DMY => t("DMY", "JMA"),
        DateOrder::YMD => t("YMD", "AMJ"),
        DateOrder::MYD => t("MYD", "MAJ"),
        DateOrder::DYM => t("DYM", "JAM"),
        DateOrder::YDM => t("YDM", "AJM"),
    }
}

/// What the header of a preview column says about its format.
fn format_label(format: ColumnFormat) -> &'static str {
    match format {
        ColumnFormat::General => t("General", "Général"),
        ColumnFormat::Text => t("Text", "Texte"),
        ColumnFormat::Date(order) => date_order_label(order),
        ColumnFormat::Skip => t("Skip Column", "Non distribuée"),
    }
}

/// Where the preview is on screen and how wide its characters are, as
/// last drawn: what turns a click into a position in the text.
#[derive(Default)]
struct Geometry {
    origin: Cell<Point<Pixels>>,
    char_w: Cell<f32>,
}

/// The first character of each column of a table of fields, then the end
/// of the last one, when every column is as wide as its longest field.
fn column_starts(rows: &[Vec<String>], columns: usize) -> Vec<usize> {
    let mut starts = Vec::with_capacity(columns + 1);
    let mut at = 0usize;
    for col in 0..columns {
        starts.push(at);
        let widest = rows.iter().filter_map(|row| row.get(col)).map(|field| field.chars().count()).max().unwrap_or(0);
        // Room for the name of the format in the header of step 3.
        at += widest.clamp(8, MAX_COLUMN) + 2;
    }
    starts.push(at);
    starts
}

/// Data ▸ Text to Columns: open the wizard on the selected column.
pub(super) fn open_text_to_columns(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Option<Entity<Wizard>> {
    let source = app.wb.clamp_to_used(app.sheet, app.sel().current());
    let lines = match app.wb.text_column_lines(app.sheet, source) {
        Ok(lines) => lines,
        Err(error) => {
            warn(app, error_text(error), cx);
            return None;
        }
    };
    let destination = RefStyle::absolute().format(Rect::cell(source.r0, source.c0), &app.wb.sheets[app.sheet].name);
    Some(app.open_modal(window, cx, move |modal, _, cx| Wizard::new(modal, source, lines, destination, cx)))
}

fn error_text(error: TextColumnsError) -> &'static str {
    match error {
        TextColumnsError::SeveralColumns => t(
            "Microsoft Excel can convert only one column at a time. The range can be many rows tall but no more than one column wide. Try again by selecting cells in one column only.",
            "Microsoft Excel ne peut convertir qu’une colonne à la fois. La plage peut comprendre plusieurs lignes mais une seule colonne. Réessayez en ne sélectionnant les cellules que d’une seule colonne.",
        ),
        TextColumnsError::NoData => t("No data was selected to parse.", "Aucune donnée à convertir n'a été sélectionnée."),
        TextColumnsError::OffSheet => t("Destination reference is not valid.", "Référence de destination non valide."),
    }
}

/// A conversion checked and ready to be carried out.
struct Plan {
    sheet: usize,
    source: Rect,
    dest: (u32, u32),
    lines: Vec<String>,
    spec: Spec,
    extent: Rect,
}

/// Check what Finish asks: the text of `source` cut by `spec` and written
/// from the destination typed in the wizard. `None` when it cannot be
/// done; the user has been told why and the wizard stays open.
fn plan(app: &mut ExcelApp, source: Rect, destination: &str, spec: Spec, cx: &mut Cx) -> Option<Plan> {
    let sheet = app.sheet;
    let checked = match typed_range(app, destination) {
        Some((on, rect)) if on == sheet => Ok((rect.r0, rect.c0)),
        _ => Err(TextColumnsError::OffSheet),
    }
    .and_then(|dest| {
        let lines = app.wb.text_column_lines(sheet, source)?;
        let extent = app.wb.text_columns_extent(dest, &lines, &spec)?;
        Ok((dest, lines, extent))
    });
    match checked {
        Ok((dest, lines, extent)) => app.may(Intent::EditCells(extent), cx).then_some(Plan {
            sheet,
            source,
            dest,
            lines,
            spec,
            extent,
        }),
        Err(error) => {
            warn(app, error_text(error), cx);
            None
        }
    }
}

/// Write the columns, asking first when data is in the way.
fn carry_out(app: &mut ExcelApp, plan: Plan, cx: &mut Cx) {
    let overwrites = app.wb.text_columns_overwrites(plan.sheet, Some(plan.source), plan.dest, &plan.lines, &plan.spec);
    let write = move |app: &mut ExcelApp, cx: &mut Cx| {
        if app.wb.write_text_columns(plan.sheet, plan.dest, &plan.lines, &plan.spec).is_ok() {
            app.commit_cells(t("Text to Columns", "Convertir"), &[plan.extent], ChangeCause::Fill, cx);
        }
    };
    if overwrites {
        app.alert(
            Alert::new(
                "OpenSheet",
                t(
                    "There's already data here. Do you want to replace it?",
                    "Cette cellule contient déjà des données. Voulez-vous les remplacer ?",
                ),
            )
            .warning()
            .button(t("OK", "OK"), move |app, _, cx| write(app, cx))
            .close_button(t("Cancel", "Annuler")),
            cx,
        );
    } else {
        write(app, cx);
    }
}

pub(super) struct Wizard {
    modal: Modal,
    /// The column being converted.
    source: Rect,
    lines: Vec<String>,
    /// 0, 1 or 2.
    step: usize,
    delimited: bool,
    delimiters: Delimiters,
    /// "Other:" is ticked; its character is typed in `other`.
    other_on: bool,
    other: Entity<TextInput>,
    /// Break lines of fixed-width text, in characters.
    breaks: Vec<usize>,
    formats: Vec<ColumnFormat>,
    /// The column of the preview selected in step 3.
    column: usize,
    /// The order the Date list shows.
    date_order: DateOrder,
    destination: Entity<TextInput>,
    geometry: Rc<Geometry>,
    /// The break line being dragged (its index in `breaks`).
    dragging: Option<usize>,
}

impl Wizard {
    fn new(modal: Modal, source: Rect, lines: Vec<String>, destination: String, cx: &mut Context<Self>) -> Self {
        let sample = &lines[..lines.len().min(64)];
        // The order dates are typed in with this interface language.
        let date_order = match crate::i18n::lang().locale().date_order {
            opensheet_engine::locale::DateOrder::MDY => DateOrder::MDY,
            opensheet_engine::locale::DateOrder::DMY => DateOrder::DMY,
        };
        Wizard {
            delimited: looks_delimited(sample),
            delimiters: guess_delimiters(sample),
            breaks: guess_breaks(sample),
            other_on: false,
            other: widgets::input(cx, ""),
            destination: modal.ref_input(RefStyle::absolute(), destination, cx),
            modal,
            source,
            lines,
            step: 0,
            formats: Vec::new(),
            column: 0,
            date_order,
            geometry: Rc::new(Geometry::default()),
            dragging: None,
        }
    }

    /// How the lines are cut, as the first two steps say.
    fn layout(&self, cx: &Context<Self>) -> Layout {
        if !self.delimited {
            return Layout::FixedWidth(self.breaks.clone());
        }
        let mut delimiters = self.delimiters.clone();
        delimiters.other = if self.other_on { self.other.read(cx).text().chars().next() } else { None };
        Layout::Delimited(delimiters)
    }

    fn spec(&self, cx: &Context<Self>) -> Spec {
        Spec {
            layout: self.layout(cx),
            formats: self.formats.clone(),
        }
    }

    fn shown(&self) -> &[String] {
        &self.lines[..self.lines.len().min(PREVIEW_LINES)]
    }

    /// The shown lines cut into fields.
    fn shown_fields(&self, cx: &Context<Self>) -> Vec<Vec<String>> {
        let layout = self.layout(cx);
        self.shown().iter().map(|line| split_line(line, &layout)).collect()
    }

    pub(super) fn go_to_step(&mut self, step: usize, cx: &mut Context<Self>) {
        self.step = step.min(2);
        if self.step == 2 {
            // One format per column of the data as it is cut now.
            let columns = field_count(&self.lines, &self.layout(cx));
            self.formats.resize(columns, ColumnFormat::General);
            self.column = self.column.min(columns.saturating_sub(1));
        }
        cx.notify();
    }

    pub(super) fn set_delimited(&mut self, delimited: bool, cx: &mut Context<Self>) {
        self.delimited = delimited;
        cx.notify();
    }

    /// Change the delimiters of step 2.
    pub(super) fn edit_delimiters(&mut self, cx: &mut Context<Self>, change: impl FnOnce(&mut Delimiters)) {
        change(&mut self.delimiters);
        cx.notify();
    }

    /// Give the selected column of step 3 a format.
    pub(super) fn set_format(&mut self, format: ColumnFormat, cx: &mut Context<Self>) {
        if let ColumnFormat::Date(order) = format {
            self.date_order = order;
        }
        if let Some(slot) = self.formats.get_mut(self.column) {
            *slot = format;
        }
        cx.notify();
    }

    pub(super) fn select_column(&mut self, column: usize, cx: &mut Context<Self>) {
        if column < self.formats.len() {
            self.column = column;
            if let ColumnFormat::Date(order) = self.formats[column] {
                self.date_order = order;
            }
            cx.notify();
        }
    }

    #[cfg(test)]
    pub(super) fn set_destination(&mut self, text: &str, cx: &mut Context<Self>) {
        self.destination.update(cx, |field, cx| field.set_text(text, cx));
    }

    #[cfg(test)]
    pub(super) fn breaks(&self) -> &[usize] {
        &self.breaks
    }

    /// A press on the ruler or the text of fixed-width data, `at`
    /// characters from the left: a double click removes the break line
    /// there, a click on one starts moving it, a click elsewhere adds one.
    pub(super) fn ruler_press(&mut self, at: usize, click_count: usize, cx: &mut Context<Self>) {
        let near = self.breaks.iter().position(|b| b.abs_diff(at) <= 1);
        match (near, click_count >= 2) {
            (Some(index), true) => {
                self.breaks.remove(index);
                self.dragging = None;
            }
            (Some(index), false) => self.dragging = Some(index),
            (None, false) if at > 0 => {
                let index = self.breaks.partition_point(|b| *b < at);
                self.breaks.insert(index, at);
                self.dragging = Some(index);
            }
            _ => {}
        }
        cx.notify();
    }

    /// The break line being dragged follows the pointer, between its
    /// neighbours.
    pub(super) fn ruler_drag(&mut self, at: usize, cx: &mut Context<Self>) {
        let Some(index) = self.dragging else {
            return;
        };
        let low = if index == 0 { 1 } else { self.breaks[index - 1] + 1 };
        let high = self.breaks.get(index + 1).map_or(usize::MAX, |next| next - 1);
        if low <= high {
            self.breaks[index] = at.clamp(low, high);
            cx.notify();
        }
    }

    pub(super) fn ruler_release(&mut self, cx: &mut Context<Self>) {
        self.dragging = None;
        cx.notify();
    }

    /// The character position of a point of the window.
    fn char_at(&self, position: Point<Pixels>) -> usize {
        let char_w = self.geometry.char_w.get().max(1.0);
        let x = f32::from(position.x - self.geometry.origin.get().x) - PAD;
        (x / char_w).round().max(0.0) as usize
    }

    fn preview_down(&mut self, ev: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let at = self.char_at(ev.position);
        match self.step {
            1 if !self.delimited => self.ruler_press(at, ev.click_count, cx),
            2 => {
                let starts = column_starts(&self.shown_fields(cx), self.formats.len());
                if let Some(column) = starts.windows(2).position(|w| at >= w[0] && at < w[1]) {
                    self.select_column(column, cx);
                }
            }
            _ => {}
        }
    }

    pub(super) fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Finish from an earlier step keeps every column General.
        if self.step < 2 {
            let columns = field_count(&self.lines, &self.layout(cx));
            self.formats.resize(columns, ColumnFormat::General);
        }
        let (source, spec) = (self.source, self.spec(cx));
        let destination = self.destination.read(cx).text().to_string();
        let Some(Some(plan)) = self.modal.update_app(cx, |app, cx| plan(app, source, &destination, spec, cx)) else {
            return;
        };
        // The wizard goes away first: the question about data in the way
        // is asked over the sheet, as in Excel.
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| carry_out(app, plan, cx));
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        if self.step < 2 {
            self.go_to_step(self.step + 1, cx);
        } else {
            self.finish(window, cx);
        }
    }

    // ---- drawing ----------------------------------------------------------

    fn preview(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let (step, delimited) = (self.step, self.delimited);
        let lines: Vec<String> = self.shown().to_vec();
        let fields = self.shown_fields(cx);
        let breaks = self.breaks.clone();
        let formats = self.formats.clone();
        let selected = self.column;
        let geometry = self.geometry.clone();
        let measured = self.geometry.clone();
        let font = mono();
        let drawing = widgets::preview(PREVIEW_W, PREVIEW_H, move |s| {
            let char_w = s.measure("0000000000", &font, FONT_SIZE) / 10.0;
            measured.char_w.set(char_w);
            let x_of = |chars: usize| PAD + chars as f32 * char_w;
            let (ink, rule) = (c(theme::TEXT), c(theme::BORDER_STRONG));
            s.rect(0.0, 0.0, s.width, s.height, c(theme::PANEL_BG));
            let top = if step == 0 { 4.0 } else { HEAD_H };
            if step == 0 || (step == 1 && !delimited) {
                // The text as it is, under the ruler in step 2.
                for (i, line) in lines.iter().enumerate() {
                    s.text(line, PAD, top + i as f32 * LINE_H + 1.0, &font, FONT_SIZE, ink);
                }
                if step == 1 {
                    s.rect(0.0, 0.0, s.width, HEAD_H, c(theme::CHROME_BG));
                    let mut tick = 0usize;
                    while x_of(tick) < s.width {
                        let tall = tick % 10 == 0;
                        let x = x_of(tick);
                        s.rect(x, HEAD_H - if tall { 7.0 } else { 3.0 }, x + 1.0, HEAD_H, rule);
                        if tall && tick > 0 {
                            s.text(&tick.to_string(), x + 2.0, 1.0, &font, 9.0, c(theme::TEXT_MUTED));
                        }
                        tick += 1;
                    }
                    for at in &breaks {
                        let x = x_of(*at);
                        s.rect(x, 4.0, x + 1.0, s.height, ink);
                        s.rect(x - 2.0, 2.0, x + 3.0, 5.0, ink);
                    }
                }
            } else {
                // The fields in columns; in step 3 a header names the
                // format of each and the selected column is inverted.
                let columns = fields.iter().map(Vec::len).max().unwrap_or(0).max(formats.len());
                let starts = column_starts(&fields, columns);
                for col in 0..columns {
                    let (x0, x1) = (x_of(starts[col]) - 3.0, x_of(starts[col + 1]) - 3.0);
                    let chosen = step == 2 && col == selected;
                    if chosen {
                        s.rect(x0, 0.0, x1, s.height, c(0x000000));
                    }
                    let color = if chosen { c(0xFFFFFF) } else { ink };
                    if step == 2 {
                        let name = format_label(formats.get(col).copied().unwrap_or_default());
                        s.text(name, x0 + 3.0, 2.0, &font, FONT_SIZE, color);
                    }
                    for (i, row) in fields.iter().enumerate() {
                        let Some(field) = row.get(col) else { continue };
                        let shown: String = field.chars().take(MAX_COLUMN).collect();
                        s.text(&shown, x0 + 3.0, top + i as f32 * LINE_H + 1.0, &font, FONT_SIZE, color);
                    }
                    s.rect(x1, 0.0, x1 + 1.0, s.height, rule);
                }
                if step == 2 {
                    s.rect(0.0, HEAD_H - 1.0, s.width, HEAD_H, rule);
                }
            }
            s.frame(0.0, 0.0, s.width, s.height, 1.0, rule);
        });
        let on_move = cx.listener(|this, position: &Point<Pixels>, _, cx| {
            let at = this.char_at(*position);
            this.ruler_drag(at, cx);
        });
        let on_end = cx.listener(|this, _: &(), _, cx| this.ruler_release(cx));
        div()
            .id("text-wizard-preview")
            .relative()
            .flex_none()
            .w(px(PREVIEW_W))
            .h(px(PREVIEW_H))
            .child(drawing)
            .child(canvas(move |bounds, _, _| geometry.origin.set(bounds.origin), |_, _, _, _| {}).absolute().top_0().left_0().size_full())
            .child(widgets::drag_tracker(
                self.dragging.is_some(),
                move |position, window, cx| on_move(&position, window, cx),
                move |window, cx| on_end(&(), window, cx),
            ))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::preview_down))
    }

    fn step_one(&self, cx: &mut Context<Self>) -> gpui::Div {
        let kind = if self.delimited { t("Delimited", "Délimité") } else { t("Fixed Width", "Largeur fixe") };
        let found = t(
            "The Text Wizard has determined that your data is |.",
            "L'Assistant Texte a déterminé que vos données sont de type |.",
        )
        .replace('|', kind);
        let option = |name: &'static str, what: &'static str| SharedString::from(format!("{name}   {what}"));
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().child(found))
            .child(div().child(t(
                "If this is correct, choose Next, or choose the Data Type that best describes your data.",
                "Si ce choix vous convient, cliquez sur Suivant, sinon choisissez le type qui décrit le mieux vos données.",
            )))
            .child(
                RadioGroup::new(
                    "kind",
                    vec![
                        option(
                            t("Delimited", "Délimité"),
                            t(
                                "- Characters such as commas or tabs separate each field.",
                                "- Des caractères tels que des virgules ou des tabulations séparent chaque champ.",
                            ),
                        ),
                        option(
                            t("Fixed width", "Largeur fixe"),
                            t(
                                "- Fields are aligned in columns with spaces between each field.",
                                "- Les champs sont alignés en colonnes et séparés par des espaces.",
                            ),
                        ),
                    ],
                    Some(usize::from(!self.delimited)),
                )
                .on_change(cx.listener(|this, index: &usize, _, cx| this.set_delimited(*index == 0, cx))),
            )
    }

    fn step_two(&self, cx: &mut Context<Self>) -> gpui::Div {
        let page = div().flex().flex_col().gap(px(8.));
        if !self.delimited {
            return page
                .child(div().child(t(
                    "This screen lets you set field widths (column breaks).",
                    "Cette étape vous permet de choisir la largeur des champs (séparateurs de colonnes).",
                )))
                .child(div().child(t(
                    "To CREATE a break line, click at the desired position.",
                    "Pour CRÉER un séparateur, cliquez à l’emplacement voulu.",
                )))
                .child(div().child(t(
                    "To DELETE a break line, double click on the line.",
                    "Pour SUPPRIMER un séparateur, double-cliquez dessus.",
                )))
                .child(div().child(t(
                    "To MOVE a break line, click and drag it.",
                    "Pour DÉPLACER un séparateur, cliquez dessus et faites-le glisser.",
                )));
        }
        let d = &self.delimiters;
        let tick = |id: &'static str, text: &'static str, on: bool, set: fn(&mut Delimiters, bool), cx: &mut Context<Self>| {
            Checkbox::new(id, text, on).on_change(cx.listener(move |this, on: &bool, _, cx| {
                let on = *on;
                this.edit_delimiters(cx, |d| set(d, on));
            }))
        };
        let qualifier = QUALIFIERS.iter().position(|q| *q == d.qualifier);
        page.child(div().child(t(
            "This screen lets you set the delimiters your data contains.",
            "Cette étape vous permet de choisir les séparateurs contenus dans vos données.",
        )))
        .child(
            div()
                .flex()
                .gap(px(24.))
                .child(
                    GroupBox::new(t("Delimiters", "Délimiteurs"))
                        .child(tick("d-tab", t("Tab", "Tabulation"), d.tab, |d, on| d.tab = on, cx))
                        .child(tick("d-semicolon", t("Semicolon", "Point-virgule"), d.semicolon, |d, on| d.semicolon = on, cx))
                        .child(tick("d-comma", t("Comma", "Virgule"), d.comma, |d, on| d.comma = on, cx))
                        .child(tick("d-space", t("Space", "Espace"), d.space, |d, on| d.space = on, cx))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .child(Checkbox::new("d-other", t("Other:", "Autre :"), self.other_on).on_change(cx.listener(
                                    |this, on: &bool, _, cx| {
                                        this.other_on = *on;
                                        cx.notify();
                                    },
                                )))
                                .child(TextField::new(&self.other).width(32.0)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .pt(px(22.))
                        .child(tick(
                            "d-consecutive",
                            t(
                                "Treat consecutive delimiters as one",
                                "Interpréter des séparateurs identiques consécutifs comme uniques",
                            ),
                            d.consecutive_as_one,
                            |d, on| d.consecutive_as_one = on,
                            cx,
                        ))
                        .child(widgets::labeled(
                            t("Text qualifier:", "Identificateur de texte :"),
                            Select::new("d-qualifier", vec!["\"".into(), "'".into(), t("{none}", "{aucun}").into()], qualifier)
                                .width(90.0)
                                .on_change(cx.listener(|this, index: &usize, _, cx| {
                                    let qualifier = QUALIFIERS[(*index).min(QUALIFIERS.len() - 1)];
                                    this.edit_delimiters(cx, |d| d.qualifier = qualifier);
                                })),
                        )),
                ),
        )
    }

    fn step_three(&self, cx: &mut Context<Self>) -> gpui::Div {
        let format = self.formats.get(self.column).copied().unwrap_or_default();
        let choice = match format {
            ColumnFormat::General => 0,
            ColumnFormat::Text => 1,
            ColumnFormat::Date(_) => 2,
            ColumnFormat::Skip => 3,
        };
        let orders: Vec<SharedString> = DateOrder::ALL.iter().map(|order| date_order_label(*order).into()).collect();
        let order = DateOrder::ALL.iter().position(|o| *o == self.date_order);
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(div().child(t(
                "This screen lets you select each column and set the Data Format.",
                "Cette étape vous permet de sélectionner chaque colonne et de définir le format des données.",
            )))
            .child(
                GroupBox::new(t("Column data format", "Format des données en colonne")).child(
                    div()
                        .flex()
                        .gap(px(12.))
                        .child(
                            RadioGroup::new(
                                "format",
                                vec![
                                    t("General", "Général").into(),
                                    t("Text", "Texte").into(),
                                    t("Date:", "Date :").into(),
                                    t("Do not import column (Skip)", "Colonne non distribuée").into(),
                                ],
                                Some(choice),
                            )
                            .on_change(cx.listener(|this, index: &usize, _, cx| {
                                let format = match *index {
                                    0 => ColumnFormat::General,
                                    1 => ColumnFormat::Text,
                                    2 => ColumnFormat::Date(this.date_order),
                                    _ => ColumnFormat::Skip,
                                };
                                this.set_format(format, cx);
                            })),
                        )
                        .child(
                            // Level with the third option.
                            div().pt(px(2.0 * (widgets::ROW_H + 4.0) - 2.0)).child(
                                Select::new("date-order", orders, order).width(80.0).on_change(cx.listener(
                                    |this, index: &usize, _, cx| {
                                        let order = DateOrder::ALL[(*index).min(DateOrder::ALL.len() - 1)];
                                        this.set_format(ColumnFormat::Date(order), cx);
                                    },
                                )),
                            ),
                        ),
                ),
            )
            .child(widgets::labeled(
                t("Destination:", "Destination :"),
                RefEdit::new(&self.destination, &self.modal).width(220.0),
            ))
    }
}

impl Render for Wizard {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = t(
            "Convert Text to Columns Wizard - Step | of 3",
            "Assistant Conversion - Étape | sur 3",
        )
        .replace('|', &(self.step + 1).to_string());
        let page = match self.step {
            0 => self.step_one(cx),
            1 => self.step_two(cx),
            _ => self.step_three(cx),
        };
        let step = self.step;
        let modal = self.modal.clone();
        let footer = div()
            .flex()
            .gap(px(8.))
            .child(Button::new("wizard-cancel", t("Cancel", "Annuler")).on_click(move |_, window, cx| modal.close(window, cx)))
            .child(
                Button::new("wizard-back", t("< Back", "< Précédent"))
                    .disabled(step == 0)
                    .on_click(cx.listener(|this, _, _, cx| this.go_to_step(this.step.saturating_sub(1), cx))),
            )
            .child(
                Button::new("wizard-next", t("Next >", "Suivant >"))
                    .disabled(step == 2)
                    .on_click(cx.listener(|this, _, _, cx| this.go_to_step(this.step + 1, cx))),
            )
            .child(
                Button::new("wizard-finish", t("Finish", "Fin"))
                    .primary()
                    .on_click(cx.listener(|this, _, window, cx| this.finish(window, cx))),
            );
        dialog(&self.modal, title)
            .width(PREVIEW_W + 32.0)
            .on_ok(cx.listener(Self::ok))
            // Every step is as tall as the tallest, so that the buttons
            // do not move under the pointer.
            .child(page.h(px(176.)))
            .child(label(t("Preview of selected data:", "Aperçu des données sélectionnées :")))
            .child(self.preview(cx))
            .footer(footer)
    }
}
