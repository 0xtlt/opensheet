//! Data ▸ Advanced: the Advanced Filter dialog. The criteria ranges are
//! read by the engine (`filter/advanced.rs`).

use super::{typed_range, warn};
use crate::app::ExcelApp;
use crate::features::filters::records_found;
use crate::features::{ChangeCause, Cx, Intent};
use crate::i18n::t;
use crate::shell::{Modal, RefStyle};
use crate::text_input::TextInput;
use crate::widgets::{Checkbox, RadioGroup, RefEdit, dialog, label, ok_cancel};
use gpui::{Context, Entity, Window, div, prelude::*, px};
use opensheet_engine::filter::AdvancedFilterError;

/// What the dialog holds when OK is pressed.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Request {
    /// Copy to another location, rather than filter in place.
    pub copy: bool,
    pub list: String,
    pub criteria: String,
    pub copy_to: String,
    pub unique: bool,
}

fn label_text() -> &'static str {
    t("Advanced Filter", "Filtre avancé")
}

fn error_text(error: AdvancedFilterError) -> &'static str {
    match error {
        AdvancedFilterError::NoList => t(
            "This command requires at least two rows of source data. You cannot use the command on a selection in only one row. Try the following:\n\n• If you're using an advanced filter, select a range of cells that contains at least two rows of data. Then click the Advanced Filter command again.\n• If you're creating a PivotTable report or PivotChart report, type a cell reference or select a range that includes at least two rows of data.",
            "Cette commande requiert au moins deux lignes de données sources. Vous ne pouvez pas l'utiliser sur une seule ligne de données. Essayez la méthode suivante :\n\n• Si vous utilisez un filtre avancé, sélectionnez une plage de cellules qui contient au moins deux lignes de données. Puis cliquez à nouveau sur la commande Filtre élaboré.\n• Si vous créez un rapport de tableau croisé dynamique ou de graphique croisé dynamique, tapez une référence de cellule ou sélectionnez une plage de cellule contenant au moins deux lignes de données.",
        ),
        AdvancedFilterError::BadCriteria => t(
            "Criteria range is not valid.",
            "Zone de critères non valide.",
        ),
        AdvancedFilterError::BadExtract => t(
            "The extract range has a missing or invalid field name.",
            "Le nom de champ est incorrect ou manquant dans la zone d’extraction.",
        ),
        AdvancedFilterError::BadDestination => t(
            "Destination reference is not valid.",
            "Référence de destination non valide.",
        ),
    }
}

/// Do what the dialog asks. False when it cannot be done: the user has
/// been told why and the dialog stays open.
pub(super) fn apply(app: &mut ExcelApp, request: &Request, cx: &mut Cx) -> bool {
    let sheet = app.sheet;
    let Some(list) = typed_range(app, &request.list) else {
        warn(
            app,
            t("Reference isn't valid.", "La référence n’est pas valide."),
            cx,
        );
        return false;
    };
    let criteria = match request.criteria.trim() {
        "" => None,
        text => match typed_range(app, text) {
            Some(range) => Some(range),
            None => {
                warn(app, error_text(AdvancedFilterError::BadCriteria), cx);
                return false;
            }
        },
    };
    let outcome = if request.copy {
        let dest = match typed_range(app, &request.copy_to) {
            Some((on, dest)) if on == sheet => dest,
            Some(_) => {
                warn(
                    app,
                    t(
                        "You can only copy filtered data to the active sheet.",
                        "Vous pouvez seulement copier des données filtrées vers la feuille active.",
                    ),
                    cx,
                );
                return false;
            }
            None => {
                warn(app, error_text(AdvancedFilterError::BadDestination), cx);
                return false;
            }
        };
        app.wb
            .advanced_filter_extent(list, criteria, request.unique, sheet, dest)
            .and_then(|extent| {
                if !app.may(Intent::EditCells(extent), cx) {
                    return Ok(None);
                }
                let written =
                    app.wb
                        .advanced_filter_copy(list, criteria, request.unique, sheet, dest)?;
                app.commit_cells(label_text(), &[written], ChangeCause::Fill, cx);
                Ok(Some(()))
            })
    } else if list.0 != sheet {
        // A list is filtered in place on the sheet it is on.
        Err(AdvancedFilterError::NoList)
    } else if !app.may(Intent::ToggleAutoFilter, cx) {
        Ok(None)
    } else {
        app.wb
            .advanced_filter_in_place(sheet, list.1, criteria, request.unique)
            .map(|found| {
                let total = app.wb.sheets[sheet]
                    .advanced_filter
                    .as_ref()
                    .map_or(found, |filter| filter.range.rows() as usize - 1);
                app.commit(label_text(), cx);
                app.set_status(records_found(found, total), cx);
                Some(())
            })
    };
    match outcome {
        Ok(done) => done.is_some(),
        Err(error) => {
            app.wb.rollback();
            warn(app, error_text(error), cx);
            false
        }
    }
}

/// Data ▸ Advanced: the dialog opens on the list around the active cell
/// and on the criteria range used last on this sheet.
pub(super) fn open(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Entity<AdvancedDialog> {
    let (row, col) = app.active();
    let sheet = &app.wb.sheets[app.sheet];
    let style = RefStyle::absolute();
    let selection = app.sel().current();
    let list = if selection.is_single() {
        app.wb.current_region(app.sheet, row, col)
    } else {
        selection
    };
    let list = style.format(list, &sheet.name);
    let criteria = sheet
        .advanced_filter
        .as_ref()
        .and_then(|filter| filter.criteria)
        .map(|range| style.format(range, &sheet.name))
        .unwrap_or_default();
    app.open_modal(window, cx, move |modal, _, cx| AdvancedDialog {
        list: modal.ref_input(style, list, cx),
        criteria: modal.ref_input(style, criteria, cx),
        copy_to: modal.ref_input(style, "", cx),
        modal,
        copy: false,
        unique: false,
    })
}

pub(super) struct AdvancedDialog {
    modal: Modal,
    copy: bool,
    list: Entity<TextInput>,
    criteria: Entity<TextInput>,
    copy_to: Entity<TextInput>,
    unique: bool,
}

impl AdvancedDialog {
    /// Fill the dialog in as a user would.
    #[cfg(test)]
    pub(super) fn set(&mut self, request: &Request, cx: &mut Context<Self>) {
        self.copy = request.copy;
        self.unique = request.unique;
        for (field, text) in [
            (&self.list, &request.list),
            (&self.criteria, &request.criteria),
            (&self.copy_to, &request.copy_to),
        ] {
            field.update(cx, |field, cx| field.set_text(text.clone(), cx));
        }
        cx.notify();
    }

    /// What the three range fields hold.
    #[cfg(test)]
    pub(super) fn texts(&self, cx: &Context<Self>) -> [String; 3] {
        [&self.list, &self.criteria, &self.copy_to].map(|field| field.read(cx).text().to_string())
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let request = Request {
            copy: self.copy,
            list: self.list.read(cx).text().to_string(),
            criteria: self.criteria.read(cx).text().to_string(),
            copy_to: self.copy_to.read(cx).text().to_string(),
            unique: self.unique,
        };
        if self
            .modal
            .update_app(cx, |app, cx| apply(app, &request, cx))
            == Some(true)
        {
            self.modal.close(window, cx);
        }
    }
}

impl Render for AdvancedDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let field = |text: &'static str, input: &Entity<TextInput>, modal: &Modal, dimmed: bool| {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .when(dimmed, |d| d.opacity(0.5))
                .child(div().w(px(120.)).flex_none().child(label(text)))
                .child(RefEdit::new(input, modal).width(190.0))
        };
        dialog(&self.modal, label_text())
            .width(370.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(t("Action", "Action")))
            .child(
                div().pl(px(10.)).child(
                    RadioGroup::new(
                        "action",
                        vec![
                            t("Filter the list, in-place", "Filtrer la liste sur place").into(),
                            t("Copy to another location", "Copier à un autre emplacement").into(),
                        ],
                        Some(usize::from(self.copy)),
                    )
                    .on_change(cx.listener(|this, index: &usize, _, cx| {
                        this.copy = *index == 1;
                        cx.notify();
                    })),
                ),
            )
            .child(field(
                t("List range:", "Plage :"),
                &self.list,
                &self.modal,
                false,
            ))
            .child(field(
                t("Criteria range:", "Zone de critères :"),
                &self.criteria,
                &self.modal,
                false,
            ))
            // "Copy to" only counts when the copy is asked for.
            .child(field(
                t("Copy to:", "Copier dans :"),
                &self.copy_to,
                &self.modal,
                !self.copy,
            ))
            .child(
                Checkbox::new(
                    "unique",
                    t("Unique records only", "Extraction sans doublon"),
                    self.unique,
                )
                .on_change(cx.listener(|this, on: &bool, _, cx| {
                    this.unique = *on;
                    cx.notify();
                })),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}
