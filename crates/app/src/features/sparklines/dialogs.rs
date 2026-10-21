//! The dialogs of sparklines: Create Sparklines (also Edit Group Location
//! & Data), Edit Sparkline Data, Sparkline Date Range, Hidden and Empty
//! Cell Settings, and the small boxes that ask for a number (a custom end
//! of the vertical axis, a line weight).

use super::{active_group, change, labels};
use crate::app::ExcelApp;
use crate::features::{Cx, Intent};
use crate::i18n::t;
use crate::shell::{Alert, Modal, RefStyle};
use crate::text_input::TextInput;
use crate::widgets::{self, Checkbox, RadioGroup, RefEdit, TextField, dialog, label, ok_cancel};
use gpui::{Context, Entity, SharedString, Window, div, prelude::*, px};
use opensheet_engine::Rect;
use opensheet_engine::sparkline::{AxisBound, EmptyCells, SparklineError, SparklineType};

/// The data range reads `Sheet1!A1:D3` when it is on another sheet.
const DATA: RefStyle = RefStyle {
    absolute: false,
    sheet: false,
    equals: false,
};

fn warn(app: &mut ExcelApp, message: impl Into<SharedString>, cx: &mut Cx) {
    app.alert(Alert::new("OpenSheet", message).warning(), cx);
}

/// Why the dialog cannot do what it holds, in Excel's words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    /// The data range is no reference.
    Data,
    /// The location is no reference to cells of this sheet.
    Location,
    /// The location is not cells of one row or column.
    LocationShape,
    /// The two do not go together.
    Mismatch,
}

impl Refusal {
    pub(super) fn message(self) -> &'static str {
        match self {
            Refusal::Data => t(
                "Data source reference is not valid.",
                "La référence à la source de données n'est pas valide.",
            ),
            Refusal::Location => t(
                "Location reference is not valid.",
                "La référence d’emplacement n’est pas valide.",
            ),
            Refusal::LocationShape => t(
                "Location reference is not valid because the cells are not all in the same column or row. Select cells that are all in a single column or row.",
                "La référence d’emplacement n’est pas valide car les cellules ne se trouvent pas toutes dans la même colonne ou ligne. Sélectionnez des cellules qui se trouvent toutes dans une même colonne ou ligne.",
            ),
            Refusal::Mismatch => t(
                "The reference for the location or data range is not valid.",
                "La référence de l’emplacement ou de la plage de données n’est pas valide.",
            ),
        }
    }
}

impl From<SparklineError> for Refusal {
    fn from(error: SparklineError) -> Refusal {
        match error {
            SparklineError::LocationShape => Refusal::LocationShape,
            SparklineError::BadData => Refusal::Data,
            SparklineError::Mismatch => Refusal::Mismatch,
        }
    }
}

/// The one range a reference typed in a dialog stands for, with its sheet.
fn typed_range(app: &ExcelApp, text: &str) -> Option<(usize, Rect)> {
    let (row, col) = app.active();
    let areas = app
        .wb
        .reference_areas(app.sheet, row, col, text.trim().trim_start_matches('='))?;
    match areas.as_slice() {
        [area] => Some((area.sheet, area.rect)),
        _ => None,
    }
}

/// What the two fields of the Create Sparklines dialog stand for: the
/// data (sheet, range) and the cells of the current sheet to draw in.
pub(super) fn read_fields(
    app: &ExcelApp,
    data: &str,
    location: &str,
) -> Result<((usize, Rect), Rect), Refusal> {
    let data = typed_range(app, data).ok_or(Refusal::Data)?;
    let (sheet, location) = typed_range(app, location).ok_or(Refusal::Location)?;
    if sheet != app.sheet {
        return Err(Refusal::Location);
    }
    if location.rows() > 1 && location.cols() > 1 {
        return Err(Refusal::LocationShape);
    }
    Ok((data, location))
}

// ------------------------------------------------- Create / Edit Sparklines

/// What OK does with the two ranges.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Purpose {
    Create(SparklineType),
    /// The group (its index on the sheet) gets other cells and data.
    EditGroup(usize),
}

pub(super) struct CreateDialog {
    modal: Modal,
    purpose: Purpose,
    pub(super) data: Entity<TextInput>,
    pub(super) location: Entity<TextInput>,
}

/// Insert > Sparklines: the selection is proposed as the place for them
/// when it is cells of one row or column, as the data otherwise.
pub(super) fn open_create(
    app: &mut ExcelApp,
    kind: SparklineType,
    window: &mut Window,
    cx: &mut Cx,
) -> Option<Entity<CreateDialog>> {
    if !app.may(Intent::ChangeSheet, cx) {
        return None;
    }
    let selection = app.sel().current();
    let name = app.wb.sheets[app.sheet].name.clone();
    let along = selection.rows() == 1 || selection.cols() == 1;
    let (data, location) = if along {
        (String::new(), RefStyle::absolute().format(selection, &name))
    } else {
        (DATA.format(selection, &name), String::new())
    };
    Some(
        app.open_modal(window, cx, move |modal, _, cx| CreateDialog {
            data: modal.ref_input(DATA, data, cx),
            location: modal.ref_input(RefStyle::absolute(), location, cx),
            modal,
            purpose: Purpose::Create(kind),
        }),
    )
}

/// Edit Group Location & Data: the dialog opens on what the group of the
/// active cell has (a group whose data makes no block is opened empty).
pub(super) fn open_edit_group(
    app: &mut ExcelApp,
    window: &mut Window,
    cx: &mut Cx,
) -> Option<Entity<CreateDialog>> {
    if !app.may(Intent::ChangeSheet, cx) {
        return None;
    }
    let (row, col) = app.active();
    let sheet = &app.wb.sheets[app.sheet];
    let (index, _) = sheet.sparkline_at(row, col)?;
    let group = &sheet.sparklines[index];
    let union = |rects: &mut dyn Iterator<Item = Rect>| rects.reduce(|a, b| a.union(&b));
    let cells = union(&mut group.sparklines.iter().map(|s| Rect::cell(s.row, s.col)));
    let ranges = union(&mut group.sparklines.iter().map(|s| s.data.range));
    let source = group
        .sparklines
        .first()
        .map(|s| s.data.sheet.clone())
        .unwrap_or_default();
    let one_sheet = group.sparklines.iter().all(|s| s.data.sheet == source);
    let location = cells
        .map(|r| RefStyle::absolute().format(r, &sheet.name))
        .unwrap_or_default();
    let data = match ranges {
        Some(range) if one_sheet => {
            let style = RefStyle {
                sheet: source != sheet.name,
                ..DATA
            };
            style.format(range, &source)
        }
        _ => String::new(),
    };
    Some(
        app.open_modal(window, cx, move |modal, _, cx| CreateDialog {
            data: modal.ref_input(DATA, data, cx),
            location: modal.ref_input(RefStyle::absolute(), location, cx),
            modal,
            purpose: Purpose::EditGroup(index),
        }),
    )
}

impl CreateDialog {
    pub(super) fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let (data, location) = (
            self.data.read(cx).text().to_string(),
            self.location.read(cx).text().to_string(),
        );
        let purpose = self.purpose;
        let done = self.modal.update_app(cx, |app, cx| {
            let sheet = app.sheet;
            let made =
                read_fields(app, &data, &location).and_then(|(data, location)| match purpose {
                    Purpose::Create(kind) => app
                        .wb
                        .add_sparklines(sheet, kind, data, location)
                        .map(|_| labels::insert())
                        .map_err(Refusal::from),
                    Purpose::EditGroup(group) => app
                        .wb
                        .edit_sparkline_group(sheet, group, data, location)
                        .map(|()| labels::data())
                        .map_err(Refusal::from),
                });
            match made {
                Ok(label) => {
                    app.commit(label, cx);
                    true
                }
                Err(refusal) => {
                    warn(app, refusal.message(), cx);
                    false
                }
            }
        });
        if done == Some(true) {
            self.modal.close(window, cx);
        }
    }
}

impl Render for CreateDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = match self.purpose {
            Purpose::Create(_) => t("Insert Sparklines", "Insérer des graphiques sparkline"),
            Purpose::EditGroup(_) => t("Edit Sparklines", "Modifier les graphiques sparkline"),
        };
        dialog(&self.modal, title)
            .width(420.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(t(
                "Select a data range for the sparklines:",
                "Sélectionnez une plage de données pour les graphiques sparkline :",
            )))
            .child(
                div()
                    .flex()
                    .child(RefEdit::new(&self.data, &self.modal).width(380.0)),
            )
            .child(label(t(
                "Select where to place sparklines:",
                "Sélectionnez l’emplacement des graphiques sparkline :",
            )))
            .child(
                div()
                    .flex()
                    .child(RefEdit::new(&self.location, &self.modal).width(380.0)),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// ------------------------------------------------------ Edit Sparkline Data

pub(super) struct SingleDialog {
    modal: Modal,
    cell: (u32, u32),
    pub(super) data: Entity<TextInput>,
}

pub(super) fn open_edit_single(
    app: &mut ExcelApp,
    window: &mut Window,
    cx: &mut Cx,
) -> Option<Entity<SingleDialog>> {
    if !app.may(Intent::ChangeSheet, cx) {
        return None;
    }
    let cell = app.active();
    let sheet = &app.wb.sheets[app.sheet];
    let (group, index) = sheet.sparkline_at(cell.0, cell.1)?;
    let data = &sheet.sparklines[group].sparklines[index].data;
    let style = RefStyle {
        sheet: data.sheet != sheet.name,
        ..DATA
    };
    let text = style.format(data.range, &data.sheet);
    Some(
        app.open_modal(window, cx, move |modal, _, cx| SingleDialog {
            data: modal.ref_input(DATA, text, cx),
            modal,
            cell,
        }),
    )
}

impl SingleDialog {
    pub(super) fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let text = self.data.read(cx).text().to_string();
        let cell = self.cell;
        let done = self.modal.update_app(cx, |app, cx| {
            let sheet = app.sheet;
            let edited = typed_range(app, &text)
                .ok_or(Refusal::Data)
                .and_then(|data| {
                    app.wb
                        .edit_sparkline_data(sheet, cell.0, cell.1, data)
                        .map_err(Refusal::from)
                });
            match edited {
                Ok(()) => {
                    app.commit(labels::data(), cx);
                    true
                }
                Err(refusal) => {
                    // One sparkline plots one row or one column.
                    let refusal = if refusal == Refusal::Mismatch {
                        Refusal::Data
                    } else {
                        refusal
                    };
                    warn(app, refusal.message(), cx);
                    false
                }
            }
        });
        if done == Some(true) {
            self.modal.close(window, cx);
        }
    }
}

impl Render for SingleDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        dialog(
            &self.modal,
            t(
                "Edit Sparkline Data",
                "Modifier les données du graphique sparkline",
            ),
        )
        .width(420.0)
        .on_ok(cx.listener(Self::ok))
        .child(label(t(
            "Select a source data range for the sparkline:",
            "Sélectionnez une plage de données sources pour le graphique sparkline :",
        )))
        .child(
            div()
                .flex()
                .child(RefEdit::new(&self.data, &self.modal).width(380.0)),
        )
        .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// ----------------------------------------------------- Sparkline Date Range

/// Give the selected sparklines a date axis reading `dates`, or the
/// general axis back (`None`). False, with Excel's message, when the
/// range does not hold one date for each point.
pub(super) fn set_date_axis(app: &mut ExcelApp, dates: Option<(usize, Rect)>, cx: &mut Cx) -> bool {
    if !app.may(Intent::ChangeSheet, cx) {
        return false;
    }
    let (sheet, ranges) = (app.sheet, app.sel().ranges.clone());
    match app.wb.set_sparkline_date_axis(sheet, &ranges, dates) {
        Ok(changed) => {
            if changed {
                app.commit(labels::axis(), cx);
            }
            true
        }
        Err(_) => {
            let block = dates.is_some_and(|(_, range)| range.rows() > 1 && range.cols() > 1);
            let message = if block {
                t(
                    "Date axis reference is not valid because the cells are not all in the same column or row. Select cells that are all in a single column or row.",
                    "La référence de l’axe des dates n’est pas valide car les cellules ne se trouvent pas toutes dans la même colonne ou ligne. Sélectionnez des cellules qui se trouvent toutes dans une même colonne ou ligne.",
                )
            } else {
                date_reference_refusal()
            };
            warn(app, message, cx);
            false
        }
    }
}

fn date_reference_refusal() -> &'static str {
    t(
        "Date axis reference is not valid.",
        "La référence de l’axe des dates n’est pas valide.",
    )
}

/// Axis > Date Axis Type: the range that holds the dates.
pub(super) struct DateRangeDialog {
    modal: Modal,
    pub(super) range: Entity<TextInput>,
}

pub(super) fn open_date_range(
    app: &mut ExcelApp,
    window: &mut Window,
    cx: &mut Cx,
) -> Option<Entity<DateRangeDialog>> {
    active_group(app)?;
    if !app.may(Intent::ChangeSheet, cx) {
        return None;
    }
    // The range the group has, as the data ranges are written.
    let current = active_group(app)?.date_axis.as_ref().map(|dates| {
        let style = RefStyle {
            sheet: dates.sheet != app.wb.sheets[app.sheet].name,
            ..DATA
        };
        style.format(dates.range, &dates.sheet)
    });
    Some(
        app.open_modal(window, cx, move |modal, _, cx| DateRangeDialog {
            range: modal.ref_input(DATA, current.unwrap_or_default(), cx),
            modal,
        }),
    )
}

impl DateRangeDialog {
    pub(super) fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let text = self.range.read(cx).text().to_string();
        let done = self
            .modal
            .update_app(cx, |app, cx| match typed_range(app, &text) {
                Some(dates) => set_date_axis(app, Some(dates), cx),
                None => {
                    warn(app, date_reference_refusal(), cx);
                    false
                }
            });
        if done == Some(true) {
            self.modal.close(window, cx);
        }
    }
}

impl Render for DateRangeDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        dialog(&self.modal, t("Sparkline Date Range", "Graphique sparkline - Plage de dates"))
            .width(420.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(t(
                "Select the range that contains the date values for the Sparkline data",
                "Sélectionnez la plage qui contient les valeurs de date pour les données du graphique sparkline",
            )))
            .child(div().flex().child(RefEdit::new(&self.range, &self.modal).width(380.0)))
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// ------------------------------------------- Hidden and Empty Cell Settings

pub(super) struct HiddenEmptyDialog {
    modal: Modal,
    pub(super) empty: EmptyCells,
    pub(super) show_hidden: bool,
    /// Lines only can run over an empty cell.
    line: bool,
}

pub(super) fn open_hidden_empty(
    app: &mut ExcelApp,
    window: &mut Window,
    cx: &mut Cx,
) -> Option<Entity<HiddenEmptyDialog>> {
    let group = active_group(app)?;
    let (empty, show_hidden, line) = (
        group.empty,
        group.show_hidden,
        group.kind == SparklineType::Line,
    );
    Some(
        app.open_modal(window, cx, move |modal, _, _| HiddenEmptyDialog {
            modal,
            empty,
            show_hidden,
            line,
        }),
    )
}

impl HiddenEmptyDialog {
    pub(super) fn set(&mut self, empty: EmptyCells, show_hidden: bool, cx: &mut Context<Self>) {
        self.empty = empty;
        self.show_hidden = show_hidden;
        cx.notify();
    }

    pub(super) fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let (empty, show_hidden) = (self.empty, self.show_hidden);
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            change(app, labels::data(), cx, |group| {
                group.empty = empty;
                group.show_hidden = show_hidden;
            })
        });
    }
}

impl Render for HiddenEmptyDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        const ORDER: [EmptyCells; 3] = [EmptyCells::Gaps, EmptyCells::Zero, EmptyCells::Connect];
        let mut options: Vec<SharedString> =
            vec![t("Gaps", "Intervalles").into(), t("Zero", "Zéro").into()];
        if self.line {
            options.push(
                t(
                    "Connect data points with line",
                    "Relier les points de données par une courbe",
                )
                .into(),
            );
        }
        let selected = ORDER
            .iter()
            .position(|e| *e == self.empty)
            .filter(|i| *i < options.len());
        dialog(
            &self.modal,
            t(
                "Hidden and Empty Cell Settings",
                "Paramètres des cellules masquées et vides",
            ),
        )
        .width(400.0)
        .on_ok(cx.listener(Self::ok))
        .child(
            div()
                .flex()
                .gap(px(16.))
                .child(label(t(
                    "Show empty cells as:",
                    "Afficher les cellules vides comme :",
                )))
                .child(
                    RadioGroup::new("empty-cells", options, selected).on_change(cx.listener(
                        |this, index: &usize, _, cx| this.set(ORDER[*index], this.show_hidden, cx),
                    )),
                ),
        )
        .child(
            Checkbox::new(
                "show-hidden",
                t(
                    "Show data in hidden rows and columns",
                    "Afficher les données des lignes et colonnes masquées",
                ),
                self.show_hidden,
            )
            .on_change(cx.listener(|this, on: &bool, _, cx| this.set(this.empty, *on, cx))),
        )
        .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// ------------------------------------------------------- a number to enter

/// What the number is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Asked {
    AxisMin,
    AxisMax,
    Weight,
}

pub(super) struct ValueDialog {
    modal: Modal,
    asked: Asked,
    pub(super) value: Entity<TextInput>,
    decimal: char,
}

fn open_value(
    app: &mut ExcelApp,
    asked: Asked,
    value: f64,
    window: &mut Window,
    cx: &mut Cx,
) -> Entity<ValueDialog> {
    let decimal = app.wb.locale.decimal_sep;
    let text = widgets::number_text(value, decimal);
    app.open_modal(window, cx, move |modal, _, cx| ValueDialog {
        modal,
        asked,
        value: widgets::input(cx, text),
        decimal,
    })
}

/// Axis > Custom Value: Excel proposes the end of the axis as it is, 0
/// for an automatic minimum.
pub(super) fn open_axis_value(
    app: &mut ExcelApp,
    max: bool,
    window: &mut Window,
    cx: &mut Cx,
) -> Option<Entity<ValueDialog>> {
    let group = active_group(app)?;
    let current = match if max { group.max } else { group.min } {
        AxisBound::Custom(value) => value,
        _ if max => app
            .wb
            .sparkline_scale(&super::SparklineGroup {
                max: AxisBound::Group,
                ..group.clone()
            })
            .max
            .unwrap_or(0.0),
        _ => 0.0,
    };
    Some(open_value(
        app,
        if max { Asked::AxisMax } else { Asked::AxisMin },
        current,
        window,
        cx,
    ))
}

/// Sparkline Color > Weight > Custom Weight.
pub(super) fn open_weight(
    app: &mut ExcelApp,
    window: &mut Window,
    cx: &mut Cx,
) -> Option<Entity<ValueDialog>> {
    let weight = active_group(app)?.line_weight;
    Some(open_value(app, Asked::Weight, weight, window, cx))
}

impl ValueDialog {
    pub(super) fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let text = self.value.read(cx).text().trim().replace(self.decimal, ".");
        let asked = self.asked;
        // Excel takes weights up to 1584 points.
        let value = text
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && (asked != Asked::Weight || (*v > 0.0 && *v <= 1584.0)));
        let Some(value) = value else {
            self.modal.update_app(cx, |app, cx| {
                warn(
                    app,
                    t(
                        "Custom value is not a valid number.",
                        "La valeur personnalisée n’est pas un nombre valide.",
                    ),
                    cx,
                )
            });
            return;
        };
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| match asked {
            Asked::AxisMin => change(app, labels::axis(), cx, |group| {
                group.min = AxisBound::Custom(value)
            }),
            Asked::AxisMax => change(app, labels::axis(), cx, |group| {
                group.max = AxisBound::Custom(value)
            }),
            Asked::Weight => change(app, labels::color(), cx, |group| group.line_weight = value),
        });
    }
}

impl Render for ValueDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (title, prompt) = match self.asked {
            Asked::AxisMin => (
                t(
                    "Sparkline Vertical Axis Setting",
                    "Paramètre de l’axe vertical du graphique sparkline",
                ),
                t(
                    "Enter the minimum value for the vertical axis:",
                    "Entrez la valeur minimale de l'axe vertical :",
                ),
            ),
            Asked::AxisMax => (
                t(
                    "Sparkline Vertical Axis Setting",
                    "Paramètre de l’axe vertical du graphique sparkline",
                ),
                t(
                    "Enter the maximum value for the vertical axis:",
                    "Entrez la valeur maximale de l'axe vertical :",
                ),
            ),
            Asked::Weight => (
                t(
                    "Sparkline Weight",
                    "Épaisseur de trait du graphique sparkline",
                ),
                t(
                    "Enter sparkline weight:",
                    "Entrez le poids du graphique sparkline :",
                ),
            ),
        };
        dialog(&self.modal, title)
            .width(360.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(prompt))
            .child(div().flex().child(TextField::new(&self.value).width(140.0)))
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}
