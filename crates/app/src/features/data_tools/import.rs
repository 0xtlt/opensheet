//! Get Data ▸ From Text/CSV and Refresh All.
//!
//! A text file is chosen in the system's file dialog, cut by the Text
//! Import Wizard (`text_wizard.rs`) and placed by the Import Data dialog.
//! The import stays with the sheet as a connection to the file: Refresh
//! All reads every such file again, and recomputes the PivotTables.

use super::{text_wizard, typed_range, warn};
use crate::app::{ExcelApp, SheetView};
use crate::features::{ChangeCause, Cx, Intent, pivot};
use crate::i18n::t;
use crate::shell::{Alert, Modal, RefStyle};
use crate::text_input::TextInput;
use crate::widgets::{RadioGroup, RefEdit, dialog, label, ok_cancel};
use gpui::{Context, Entity, PathPromptOptions, Window, div, prelude::*, px};
use opensheet_engine::Rect;
use opensheet_engine::protect::Action;
use opensheet_engine::text_columns::{TextColumnsError, TextImport};
use std::path::Path;

fn import_label() -> &'static str {
    t("Import Data", "Importer des données")
}

/// The text of a file, whatever its encoding; `None` when it cannot be
/// read.
fn read_text(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|bytes| opensheet_io::decode_text(&bytes))
}

fn missing_file() -> &'static str {
    t(
        "Excel cannot find the text file to refresh this external data range.\n\nCheck to make sure the text file has not been moved or renamed, then try the refresh again.",
        "Impossible de trouver le fichier texte pour l'actualisation de cette plage de données externes.\n\nAssurez-vous que le texte n'a été ni déplacé ni renommé et réessayez.",
    )
}

/// Data ▸ Get Data ▸ From Text/CSV: ask for the file, then go on with
/// the wizard.
pub(super) fn choose_file(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    if app.is_editing() && !app.commit_edit(None, window, cx) {
        return;
    }
    let paths = cx.prompt_for_paths(PathPromptOptions {
        files: true,
        directories: false,
        multiple: false,
        prompt: Some(t("Import", "Importer").into()),
    });
    cx.spawn_in(window, async move |this, cx| {
        if let Ok(Ok(Some(paths))) = paths.await {
            if let Some(path) = paths.into_iter().next() {
                this.update_in(cx, |app, window, cx| {
                    import_file(app, &path, window, cx);
                })
                .ok();
            }
        }
    })
    .detach();
}

/// Open the Text Import Wizard on a file.
pub(super) fn import_file(app: &mut ExcelApp, path: &Path, window: &mut Window, cx: &mut Cx) -> Option<Entity<text_wizard::Wizard>> {
    match read_text(path).filter(|text| !text.trim().is_empty()) {
        Some(text) => Some(text_wizard::open_text_import(app, path.to_string_lossy().into_owned(), &text, window, cx)),
        None => {
            app.alert(
                Alert::new(t("Can't open this file", "Impossible d'ouvrir ce fichier"), path.display().to_string()).warning(),
                cx,
            );
            None
        }
    }
}

/// The wizard is finished: "Import Data" asks where the data goes.
pub(super) fn open_destination(app: &mut ExcelApp, import: TextImport, text: String, window: &mut Window, cx: &mut Cx) -> Entity<ImportDialog> {
    let (row, col) = app.active();
    let style = RefStyle {
        absolute: true,
        sheet: false,
        equals: true,
    };
    let place = style.format(Rect::cell(row, col), "");
    app.open_modal(window, cx, move |modal, _, cx| ImportDialog {
        place: modal.ref_input(style, place, cx),
        modal,
        import,
        text,
        new_sheet: false,
    })
}

/// Put the data on the sheet and remember where it came from. False
/// when it cannot be done; the user has been told why.
fn place(app: &mut ExcelApp, import: &TextImport, text: &str, new_sheet: bool, place: &str, cx: &mut Cx) -> bool {
    let (sheet, dest) = if new_sheet {
        if !app.may(Intent::InsertSheet, cx) {
            return false;
        }
        // After the current sheet, as Insert Sheet does.
        let index = app.wb.add_sheet(app.sheet + 1);
        app.views.insert(index.min(app.views.len()), SheetView::default());
        (index, (0, 0))
    } else {
        match typed_range(app, place) {
            Some((sheet, rect)) if sheet == app.sheet => (sheet, (rect.r0, rect.c0)),
            _ => {
                warn(app, t("Destination reference is not valid.", "Référence de destination non valide."), cx);
                return false;
            }
        }
    };
    // What is in the way is replaced, as when a query is refreshed; a
    // protected sheet says no first.
    if let (Ok(extent), false) = (app.wb.text_import_extent(dest, text, import), new_sheet) {
        if !app.may(Intent::EditCells(extent), cx) {
            return false;
        }
    }
    match app.wb.import_text(sheet, dest, text, import.clone()) {
        Ok(written) => {
            app.sheet = sheet;
            app.commit_cells(import_label(), &[written], ChangeCause::Fill, cx);
            app.select_cell(dest.0, dest.1, cx);
            true
        }
        Err(error) => {
            app.wb.rollback();
            if new_sheet {
                app.views.remove(sheet.min(app.views.len() - 1));
            }
            let message = match error {
                TextColumnsError::OffSheet => t("Destination reference is not valid.", "Référence de destination non valide."),
                _ => t("No data was selected to parse.", "Aucune donnée à convertir n'a été sélectionnée."),
            };
            warn(app, message, cx);
            false
        }
    }
}

pub(super) struct ImportDialog {
    modal: Modal,
    import: TextImport,
    /// The lines of the file, as the wizard cut them apart.
    text: String,
    new_sheet: bool,
    place: Entity<TextInput>,
}

impl ImportDialog {
    #[cfg(test)]
    pub(super) fn set(&mut self, new_sheet: bool, place: &str, cx: &mut Context<Self>) {
        self.new_sheet = new_sheet;
        self.place.update(cx, |field, cx| field.set_text(place, cx));
        cx.notify();
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let (import, text, new_sheet) = (self.import.clone(), self.text.clone(), self.new_sheet);
        let target = self.place.read(cx).text().to_string();
        if self.modal.update_app(cx, |app, cx| place(app, &import, &text, new_sheet, &target, cx)) == Some(true) {
            self.modal.close(window, cx);
        }
    }
}

impl Render for ImportDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        dialog(&self.modal, import_label())
            .width(360.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(t("Where do you want to put the data?", "Insérer les données dans ")))
            .child(
                div()
                    .pl(px(10.))
                    .flex()
                    .gap(px(12.))
                    .child(
                        RadioGroup::new(
                            "place",
                            vec![t("Existing sheet:", "Feuille existante :").into(), t("New sheet", "Nouvelle feuille").into()],
                            Some(usize::from(self.new_sheet)),
                        )
                        .on_change(cx.listener(|this, index: &usize, _, cx| {
                            this.new_sheet = *index == 1;
                            cx.notify();
                        })),
                    )
                    .child(div().when(self.new_sheet, |d| d.opacity(0.5)).child(RefEdit::new(&self.place, &self.modal).width(150.0))),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// ------------------------------------------------------------------ refresh

/// Read the file of one import again. `Ok(None)`: the sheet is protected
/// and the import is left as it is.
fn refresh_import(app: &mut ExcelApp, sheet: usize, index: usize) -> Result<Option<Rect>, ()> {
    let import = &app.wb.sheets[sheet].text_imports[index];
    let ranges = [import.range];
    if app.wb.permits(&Action::EditCells { sheet, ranges: &ranges }).is_err() {
        return Ok(None);
    }
    let text = read_text(Path::new(&import.path)).ok_or(())?;
    app.wb.refresh_text_import(sheet, index, &text).map(Some).map_err(|_| ())
}

/// Data ▸ Refresh All: every imported text file is read again and every
/// PivotTable recomputed, in one undo step.
pub(super) fn refresh_all(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let label = t("Refresh All", "Actualiser tout");
    // PivotTables first: when one of them would cover data, the question
    // is the PivotTable feature's to ask, in a step of its own.
    let pivots = app.wb.pivots().next().is_some();
    let mut ask_pivots = false;
    if pivots && app.may(Intent::UsePivotTable, cx) && app.wb.pivot_refresh_all(false).is_err() {
        app.wb.rollback();
        ask_pivots = true;
    }
    let mut missing = false;
    let mut written = Vec::new();
    for sheet in 0..app.wb.sheets.len() {
        for index in 0..app.wb.sheets[sheet].text_imports.len() {
            match refresh_import(app, sheet, index) {
                Ok(Some(rect)) if sheet == app.sheet => written.push(rect),
                Ok(_) => {}
                Err(()) => missing = true,
            }
        }
    }
    app.commit_cells(label, &written, ChangeCause::Fill, cx);
    if missing {
        warn(app, missing_file(), cx);
    }
    if ask_pivots {
        pivot::refresh_all(app, window, cx);
    }
}

/// Data ▸ Refresh All ▸ Refresh: the import or the PivotTable the active
/// cell is in.
pub(super) fn refresh_here(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let (sheet, (row, col)) = (app.sheet, app.active());
    if let Some(index) = app.wb.text_import_at(sheet, row, col) {
        let range = app.wb.sheets[sheet].text_imports[index].range;
        if !app.may(Intent::EditCells(range), cx) {
            return;
        }
        match refresh_import(app, sheet, index) {
            Ok(Some(rect)) => app.commit_cells(t("Refresh", "Actualiser"), &[rect], ChangeCause::Fill, cx),
            Ok(None) => {}
            Err(()) => {
                app.wb.rollback();
                warn(app, missing_file(), cx);
            }
        }
    } else if app.wb.pivot_at(sheet, row, col).is_some() {
        pivot::refresh_active(app, window, cx);
    }
}

/// Whether Refresh has something to refresh at the active cell.
pub(super) fn can_refresh_here(app: &ExcelApp) -> bool {
    let (row, col) = app.active();
    app.wb.text_import_at(app.sheet, row, col).is_some() || app.wb.pivot_at(app.sheet, row, col).is_some()
}
