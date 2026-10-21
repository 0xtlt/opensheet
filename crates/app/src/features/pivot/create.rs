//! The dialogs that say where the data of a PivotTable comes from and
//! where its report goes: "PivotTable from table or range", "Move
//! PivotTable" and "Change PivotTable Data Source".

use super::{ANALYZE_TAB, actions, active_pivot, logic, perform_in_dialog, sheet_inserted};
use crate::app::ExcelApp;
use crate::features::Cx;
use crate::grid::Selection;
use crate::i18n::t;
use crate::shell::{Modal, RefStyle};
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{self, c};
use crate::widgets::{Button, Checkbox, RadioGroup, RefEdit, dialog, label, ok_cancel};
use gpui::{Context, Entity, FontWeight, Window, div, prelude::*, px};
use opensheet_engine::pivot::{PivotDestination, PivotError, PivotSource};

/// What the dialog is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Create,
    Move(u32),
    ChangeSource(u32),
}

/// References as these dialogs write them: `Sheet1!$A$1:$D$10`.
const REFERENCE: RefStyle = RefStyle {
    absolute: true,
    sheet: true,
    equals: false,
};

/// Insert > PivotTable > From Table/Range.
pub(super) fn open_create(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let source = logic::default_source(&app.wb, app.sheet, app.active(), app.sel().current());
    open(app, Kind::Create, source, false, String::new(), window, cx);
}

/// PivotTable Analyze > Move PivotTable: opens on where it is now.
pub(super) fn open_move(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some(pivot) = active_pivot(app) else {
        return;
    };
    let corner = logic::select_rect(pivot, logic::SelectWhat::Entire).map(|all| (all.r0, all.c0));
    let location = corner
        .map(|(row, col)| logic::destination_text(&app.wb, app.sheet, row, col))
        .unwrap_or_default();
    let id = pivot.id;
    open(
        app,
        Kind::Move(id),
        String::new(),
        true,
        location,
        window,
        cx,
    );
}

/// PivotTable Analyze > Change Data Source.
pub(super) fn open_change_source(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some(pivot) = active_pivot(app) else {
        return;
    };
    let (id, source) = (pivot.id, pivot.source.to_text());
    open(
        app,
        Kind::ChangeSource(id),
        source,
        false,
        String::new(),
        window,
        cx,
    );
}

fn open(
    app: &mut ExcelApp,
    kind: Kind,
    source: String,
    existing: bool,
    location: String,
    window: &mut Window,
    cx: &mut Cx,
) {
    app.open_modal(window, cx, move |modal, _, cx| {
        let source = modal.ref_input(REFERENCE, source, cx);
        let location = modal.ref_input(REFERENCE, location, cx);
        // Clicking in the Location box, typing in it or picking cells for
        // it is choosing an existing worksheet.
        cx.subscribe(
            &location,
            |this: &mut PlaceDialog, _, ev: &InputEvent, cx| {
                if *ev == InputEvent::Clicked && !this.existing {
                    this.existing = true;
                    cx.notify();
                }
            },
        )
        .detach();
        cx.observe(&location, |this: &mut PlaceDialog, location, cx| {
            let text = location.read(cx).text().to_string();
            if text != this.location_seen {
                this.location_seen = text;
                this.existing = true;
            }
        })
        .detach();
        PlaceDialog {
            modal,
            kind,
            source,
            existing,
            location_seen: location.read(cx).text().to_string(),
            location,
        }
    });
}

struct PlaceDialog {
    modal: Modal,
    kind: Kind,
    source: Entity<TextInput>,
    existing: bool,
    location: Entity<TextInput>,
    /// The text of the Location box when last looked at.
    location_seen: String,
}

impl PlaceDialog {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let kind = self.kind;
        let source = self.source.read(cx).text().to_string();
        let location = self.location.read(cx).text().to_string();
        let existing = self.existing;
        let modal = self.modal.clone();
        self.modal.update_app(cx, |app, cx| {
            apply(app, &modal, kind, &source, existing, &location, window, cx)
        });
    }
}

/// Do what the dialog asks. What cannot be read is refused with Excel's
/// message and the dialog stays open.
#[allow(clippy::too_many_arguments)]
fn apply(
    app: &mut ExcelApp,
    modal: &Modal,
    kind: Kind,
    source: &str,
    existing: bool,
    location: &str,
    window: &mut Window,
    cx: &mut Cx,
) {
    let source = match kind {
        Kind::Move(_) => None,
        _ => match PivotSource::parse(source, app.sheet, &app.wb) {
            Some(source) => Some(source),
            None => {
                actions::warn(app, logic::error_text(&PivotError::InvalidSource), cx);
                return;
            }
        },
    };
    // A new worksheet goes before the current one, as in Excel.
    let new_sheet = app.sheet;
    let dest = if !existing {
        PivotDestination::NewSheet { index: new_sheet }
    } else {
        match logic::parse_destination(location, app.sheet, &app.wb) {
            Some((sheet, row, col)) => PivotDestination::Existing { sheet, row, col },
            None => {
                actions::warn(
                    app,
                    t(
                        "Destination reference is not valid.",
                        "Référence de destination non valide.",
                    ),
                    cx,
                );
                return;
            }
        }
    };
    let label = match kind {
        Kind::Create => t("Insert PivotTable", "Insérer un tableau croisé dynamique"),
        Kind::Move(_) => t("Move PivotTable", "Déplacer le tableau croisé dynamique"),
        Kind::ChangeSource(_) => t("Change Data Source", "Changer la source de données"),
    };
    perform_in_dialog(app, modal, label, window, cx, move |app, overwrite| {
        let sheets = app.wb.sheets.len();
        let id = match (kind, &source) {
            (Kind::Create, Some(source)) => app.wb.pivot_create(source.clone(), dest, overwrite)?,
            (Kind::Move(id), _) => {
                app.wb.pivot_move(id, dest, overwrite)?;
                id
            }
            (Kind::ChangeSource(id), Some(source)) => {
                return app.wb.pivot_change_source(id, source.clone(), overwrite);
            }
            _ => return Err(PivotError::InvalidSource),
        };
        if app.wb.sheets.len() > sheets {
            sheet_inserted(app, new_sheet);
        }
        // The PivotTable is shown, its first cell active: the field list
        // and the PivotTable tabs come with it.
        if let Some((sheet, corner)) = app.wb.pivot(id).and_then(|(sheet, p)| {
            logic::select_rect(p, logic::SelectWhat::Body).map(|body| (sheet, (body.r0, body.c0)))
        }) {
            app.sheet = sheet;
            app.views[sheet].sel = Selection::single(corner.0, corner.1);
            app.context_tab = Some(ANALYZE_TAB);
        }
        Ok(())
    });
}

impl Render for PlaceDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let heading = |text: &'static str| label(text).font_weight(FontWeight::SEMIBOLD);
        let field = |text: &'static str, input: &Entity<TextInput>, modal: &Modal| {
            div()
                .pl(px(16.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(div().w(px(104.)).flex_none().child(label(text)))
                .child(RefEdit::new(input, modal))
        };
        let title = match self.kind {
            Kind::Create => t(
                "PivotTable from table or range",
                "Tableau croisé dynamique à partir d'une table ou d'une plage",
            ),
            Kind::Move(_) => t("Move PivotTable", "Déplacer le tableau croisé dynamique"),
            Kind::ChangeSource(_) => t(
                "Change PivotTable Data Source",
                "Modifier la source de données du tableau croisé dynamique",
            ),
        };
        let mut frame = dialog(&self.modal, title)
            .width(540.0)
            .on_ok(cx.listener(Self::ok));

        // Where the data comes from.
        match self.kind {
            Kind::Create => {
                frame = frame
                    .child(heading(t(
                        "Select a table or range",
                        "Sélectionner un tableau ou une plage",
                    )))
                    .child(field(
                        t("Table/Range:", "Tableau/Plage :"),
                        &self.source,
                        &self.modal,
                    ));
            }
            Kind::ChangeSource(_) => {
                frame = frame
                    .child(heading(t(
                        "Choose the data that you want to analyze",
                        "Choisissez les données à analyser",
                    )))
                    .child(RadioGroup::new(
                        "range",
                        vec![
                            t(
                                "Select a table or range",
                                "Sélectionner un tableau ou une plage",
                            )
                            .into(),
                        ],
                        Some(0),
                    ))
                    .child(field(
                        t("Table/Range:", "Tableau/Plage :"),
                        &self.source,
                        &self.modal,
                    ))
                    // The engine reads the data of the workbook only.
                    .child(
                        RadioGroup::new(
                            "external",
                            vec![
                                t(
                                    "Use an external data source",
                                    "Utiliser une source de données externe",
                                )
                                .into(),
                            ],
                            None,
                        )
                        .disabled(true),
                    )
                    .child(
                        div().pl(px(16.)).child(
                            Button::new(
                                "connection",
                                t("Choose Connection...", "Choisir la connexion..."),
                            )
                            .disabled(true),
                        ),
                    );
            }
            Kind::Move(_) => {}
        }

        // Where the report goes.
        if !matches!(self.kind, Kind::ChangeSource(_)) {
            let question = match self.kind {
                Kind::Move(_) => t(
                    "Choose where you want the PivotTable report to be placed",
                    "Choisissez l'emplacement de votre rapport de tableau croisé dynamique",
                ),
                _ => t(
                    "Choose where you want the PivotTable to be placed",
                    "Choisissez l’emplacement où vous voulez placer le tableau croisé dynamique",
                ),
            };
            frame = frame
                .child(heading(question))
                .child(
                    div().pl(px(16.)).child(
                        RadioGroup::new(
                            "where",
                            vec![
                                t("New Worksheet", "Nouvelle feuille de calcul").into(),
                                t("Existing Worksheet", "Feuille de calcul existante").into(),
                            ],
                            Some(self.existing as usize),
                        )
                        .on_change(cx.listener(
                            |this, ix: &usize, _, cx| {
                                this.existing = *ix == 1;
                                cx.notify();
                            },
                        )),
                    ),
                )
                .child(
                    field(t("Location:", "Emplacement :"), &self.location, &self.modal)
                        .pl(px(36.))
                        .when(!self.existing, |d| d.text_color(c(theme::TEXT_DISABLED))),
                );
        }
        if self.kind == Kind::Create {
            // The engine has no Data Model.
            frame = frame
                .child(heading(t(
                    "Choose whether you want to analyze multiple tables",
                    "Indiquez si vous souhaitez analyser plusieurs tables",
                )))
                .child(
                    div().pl(px(16.)).child(
                        Checkbox::new(
                            "data-model",
                            t(
                                "Add this data to the Data Model",
                                "Ajouter ces données au modèle de données",
                            ),
                            false,
                        )
                        .disabled(true),
                    ),
                );
        }
        frame.footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}
