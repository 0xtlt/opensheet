//! Get Data and Refresh All.
//!
//! A source is chosen (a text file, an address on the web, a workbook, a
//! JSON or an XML file), read by `sources.rs`, shown by the Text Import
//! Wizard (`text_wizard.rs`) or the Navigator (`navigator.rs`) and placed
//! by the Import Data dialog. The query stays with the sheet as a
//! connection: Refresh and Refresh All read every source again, and
//! recompute the PivotTables. What is on this machine is read at once;
//! the web is fetched off the window thread and its data arrives as a
//! step of its own.

use super::sources::{self, Document, FetchError, Fetched};
use super::{navigator, text_wizard, typed_range, warn};
use crate::app::{ExcelApp, SheetView};
use crate::features::{ChangeCause, Cx, Intent, pivot};
use crate::i18n::t;
use crate::shell::{Alert, Modal, RefStyle};
use crate::text_input::TextInput;
use crate::widgets::{RadioGroup, RefEdit, dialog, label, ok_cancel};
use gpui::{Context, Entity, PathPromptOptions, Task, Window, div, prelude::*, px};
use opensheet_engine::Rect;
use opensheet_engine::external::{SourceTable, WebQuery, pick};
use opensheet_engine::protect::Action;
use opensheet_engine::text_columns::{TextColumnsError, TextImport};
use std::path::{Path, PathBuf};

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

/// Why an address gave nothing, in words.
pub(super) fn fetch_message(error: FetchError, address: &str) -> String {
    let text = match error {
        FetchError::BadAddress => t("Invalid Web Query", "Requête sur le web non valide"),
        FetchError::Missing => t(
            "We couldn't connect to the external data source. Make sure you have access to the source file and that it's not protected.",
            "Nous n’avons pas pu nous connecter à la source de données externe. Vérifiez que vous avez accès au fichier source et qu’il n’est pas protégé.",
        ),
        FetchError::NoConnection => t(
            "Unable to open |0. Cannot locate the Internet server or proxy server.",
            "Impossible d'ouvrir |0. Impossible de localiser le serveur Internet ou le serveur proxy.",
        ),
        FetchError::Status(404) => t(
            "Unable to open |0. The Internet site reports that the item you requested could not be found. (HTTP/1.0 404)",
            "Impossible d'ouvrir |0. Le site Internet signale que l'élément demandé est introuvable. (HTTP/1.0 404)",
        ),
        FetchError::Status(_) => t("Unable to open |0. The Internet site reports an error. (HTTP |1)", "Impossible d'ouvrir |0. Le site Internet signale une erreur. (HTTP |1)"),
        FetchError::Timeout => t("Unable to open |0. The server did not answer in time.", "Impossible d'ouvrir |0. Le serveur n'a pas répondu à temps."),
        FetchError::TooLarge => t("Unable to open |0. The data is too large.", "Impossible d'ouvrir |0. Les données sont trop volumineuses."),
        FetchError::LocalFile => t(
            "Unable to open |0. A web query cannot open a local file. Use an http or https address.",
            "Impossible d'ouvrir |0. Une requête sur le web ne peut pas ouvrir un fichier local. Indiquez une adresse http ou https.",
        ),
    };
    let code = match error {
        FetchError::Status(code) => code.to_string(),
        _ => String::new(),
    };
    text.replace("|0", address).replace("|1", &code)
}

fn no_data() -> &'static str {
    t(
        "This Web query returned no data. To change the query, click OK, click the arrow on the name box in the formula bar, click the name of the external data range for the Web query, right-click the selection, and then click Edit Query.",
        "Aucune donnée renvoyée par la requête sur le web. Pour modifier la requête, cliquez sur OK, cliquez sur la flèche de la zone de nom dans la barre de formule, cliquez sur le nom de la plage de données externes dans la requête sur le web, cliquez avec le bouton droit sur la sélection, puis cliquez sur Modifier la requête.",
    )
}

/// Fetch an address and hand what it gave to `then`: at once for a file
/// the caller named, later and off the window thread for the web.
pub(super) fn with_fetched(
    app: &mut ExcelApp,
    address: String,
    window: &mut Window,
    cx: &mut Cx,
    then: impl FnOnce(&mut ExcelApp, Result<Fetched, FetchError>, &mut Window, &mut Cx) + 'static,
) {
    if !sources::is_remote(&address) {
        // A file the caller just named (the file dialog, or a test). A
        // web query stored in a workbook never arrives here.
        let fetched = if sources::local_path(&address).is_some() {
            remember_chosen_file(app, &address);
            sources::read_local(&address)
        } else if sources::names_a_file(&address) {
            Err(FetchError::LocalFile)
        } else {
            Err(FetchError::BadAddress)
        };
        then(app, fetched, window, cx);
        return;
    }
    app.set_status(t("Connecting…", "Connexion…"), cx);
    let fetching = cx.background_executor().spawn(async move { sources::fetch(&address) });
    cx.spawn_in(window, async move |this, cx| {
        let fetched = fetching.await;
        this.update_in(cx, |app, window, cx| {
            app.set_status("", cx);
            then(app, fetched, window, cx);
        })
        .ok();
    })
    .detach();
}

/// Show what an address holds: the Text Import Wizard for text, the
/// Navigator for tables. `replaces` is the query being edited, if any.
pub(super) fn open_address(app: &mut ExcelApp, address: String, replaces: Option<QueryRef>, window: &mut Window, cx: &mut Cx) {
    let shown = address.clone();
    with_fetched(app, address, window, cx, move |app, fetched, window, cx| match fetched {
        Err(error) => warn(app, fetch_message(error, &shown), cx),
        Ok(fetched) => match sources::read_document(&fetched, &shown, app.wb.locale) {
            Some(Document::Text(text)) if replaces.is_none() => {
                text_wizard::open_text_import(app, shown, &text, window, cx);
            }
            Some(Document::Tables(tables)) => {
                navigator::open(app, shown, tables, replaces, window, cx);
            }
            _ => warn(app, no_data(), cx),
        },
    });
}

/// Data ▸ Get Data ▸ From Excel Workbook, From JSON, From XML: ask for
/// the file, then show what it holds.
pub(super) fn choose_document(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
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
                    open_address(app, sources::file_url(&path), None, window, cx);
                })
                .ok();
            }
        }
    })
    .detach();
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
    open_placement(app, import, Payload::Text(text), window, cx)
}

/// What the Import Data dialog places.
pub(super) enum Payload {
    /// The lines of a text file, as the wizard cut them apart.
    Text(String),
    /// The table chosen in the Navigator.
    Table(SourceTable),
}

/// "Import Data" for either kind of data.
pub(super) fn open_placement(app: &mut ExcelApp, import: TextImport, payload: Payload, window: &mut Window, cx: &mut Cx) -> Entity<ImportDialog> {
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
        payload,
        new_sheet: false,
    })
}

/// Put the data on the sheet and remember where it came from. False
/// when it cannot be done; the user has been told why.
pub(super) fn place(app: &mut ExcelApp, import: &TextImport, payload: &Payload, new_sheet: bool, place: &str, window: &mut Window, cx: &mut Cx) -> bool {
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
    let extent = match payload {
        Payload::Text(text) => app.wb.text_import_extent(dest, text, import),
        Payload::Table(table) => app.wb.source_table_extent(dest, table),
    };
    if let (Ok(extent), false) = (extent, new_sheet) {
        // The corner of a new query is not inside another one.
        if app.wb.text_import_at(sheet, dest.0, dest.1).is_some() {
            warn(
                app,
                t(
                    "The start of the destination range specified lies within an existing External Data Range.  Please choose another destination. ",
                    "Le début de la plage de destination spécifiée se trouve dans une plage de données externes existante. Choisissez une autre destination.",
                ),
                cx,
            );
            return false;
        }
        if !app.may(Intent::EditCells(extent), cx) {
            return false;
        }
    }
    let imported = match payload {
        Payload::Text(text) => app.wb.import_text(sheet, dest, text, import.clone()),
        Payload::Table(table) => app.wb.import_source_table(sheet, dest, table, import.clone()),
    };
    match imported {
        Ok(written) => {
            app.sheet = sheet;
            if import.props.adjust_width {
                fit_columns(app, sheet, written, window);
            }
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
    payload: Payload,
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
        let new_sheet = self.new_sheet;
        let target = self.place.read(cx).text().to_string();
        let (import, payload) = (&self.import, &self.payload);
        if self.modal.update_app(cx, |app, cx| place(app, import, payload, new_sheet, &target, window, cx)) == Some(true) {
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

/// A query, named so that it is found again after a fetch: its sheet and
/// its own name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct QueryRef {
    pub sheet: String,
    pub name: String,
}

impl QueryRef {
    pub(super) fn of(app: &ExcelApp, sheet: usize, index: usize) -> Option<QueryRef> {
        let on = app.wb.sheets.get(sheet)?;
        Some(QueryRef {
            sheet: on.name.clone(),
            name: on.text_imports.get(index)?.name.clone(),
        })
    }

    /// Where the query is now: its sheet and its place there.
    pub(super) fn find(&self, app: &ExcelApp) -> Option<(usize, usize)> {
        let sheet = app.wb.sheets.iter().position(|sheet| sheet.name == self.sheet)?;
        let index = app.wb.sheets[sheet].text_imports.iter().position(|import| import.name == self.name)?;
        Some((sheet, index))
    }
}

/// Every query of the workbook, sheet by sheet.
pub(super) fn all_queries(app: &ExcelApp) -> Vec<(usize, usize)> {
    let sheets = app.wb.sheets.iter().enumerate();
    sheets.flat_map(|(sheet, on)| (0..on.text_imports.len()).map(move |index| (sheet, index))).collect()
}

/// Give the columns of freshly written data the width they need
/// ("Adjust column width"), in the undo step being built.
pub(super) fn fit_columns(app: &mut ExcelApp, sheet: usize, range: Rect, window: &mut Window) {
    let ts = window.text_system().clone();
    for col in range.c0..=range.c1 {
        if let Some(width) = crate::grid::autofit_col_width(&app.wb, sheet, col, &app.fonts, &ts) {
            app.wb.set_col_width(sheet, col, Some(width));
        }
    }
}

/// Why a query was not refreshed.
enum Failed {
    /// Its sheet is protected: the query is left as it is, silently.
    Protected,
    Fetch(FetchError),
    /// The source no longer holds what the query takes.
    NoData,
}

/// Write what the source of a query holds now. The caller commits.
fn apply(app: &mut ExcelApp, sheet: usize, index: usize, fetched: &Fetched, window: &mut Window) -> Result<Rect, Failed> {
    let import = app.wb.sheets[sheet].text_imports[index].clone();
    let ranges = [import.range];
    if app.wb.permits(&Action::EditCells { sheet, ranges: &ranges }).is_err() {
        return Err(Failed::Protected);
    }
    let written = match &import.web {
        None => {
            let text = opensheet_io::decode_text(&fetched.bytes);
            app.wb.refresh_text_import(sheet, index, &text)
        }
        Some(web) => {
            let Some(Document::Tables(tables)) = sources::read_document(fetched, &import.path, app.wb.locale) else {
                return Err(Failed::NoData);
            };
            let table = pick(&tables, web).ok_or(Failed::NoData)?;
            app.wb.refresh_source_table(sheet, index, table)
        }
    }
    .map_err(|_| Failed::NoData)?;
    if import.props.adjust_width {
        fit_columns(app, sheet, written, window);
    }
    Ok(written)
}

/// Tell the user which queries could not be refreshed. One message, for
/// the first that failed, as Excel stops at the first.
fn report(app: &mut ExcelApp, failures: Vec<(TextImport, String, Failed)>, cx: &mut Cx) {
    let Some((import, sheet, failed)) = failures.into_iter().find(|(_, _, failed)| !matches!(failed, Failed::Protected)) else {
        return;
    };
    let message = match failed {
        Failed::Fetch(FetchError::Missing) if import.web.is_none() => missing_file().to_string(),
        Failed::Fetch(error) => fetch_message(error, &import.path),
        _ => t("The external data range |0 on |1 could not be refreshed.", "La plage de données externes |0 sur |1 ne peut être actualisée.")
            .replace("|0", &import.name)
            .replace("|1", &sheet),
    };
    warn(app, message, cx);
}

/// Refresh the given queries in one undo step called `label`: those on
/// this machine at once, those on the web when their data has arrived,
/// in a step of their own. Returns the task that waits for the web, for
/// the callers that keep it.
pub(super) fn refresh_queries(app: &mut ExcelApp, queries: Vec<(usize, usize)>, label: &'static str, window: &mut Window, cx: &mut Cx) -> Option<Task<()>> {
    let mut written = Vec::new();
    let mut failures = Vec::new();
    let mut remote = Vec::new();
    for (sheet, index) in queries {
        let import = app.wb.sheets[sheet].text_imports[index].clone();
        if sources::is_remote(&import.path) {
            remote.extend(QueryRef::of(app, sheet, index).map(|query| (query, import.path)));
            continue;
        }
        // A text import re-reads its file. A web query may read a local
        // file only when the user picked that file in this session; a
        // path that came in with the workbook is refused.
        let loaded = if import.web.is_some() && !user_chose_file(app, &import.path) {
            Err(if sources::names_a_file(&import.path) { FetchError::LocalFile } else { FetchError::BadAddress })
        } else {
            sources::read_local(&import.path)
        };
        let outcome = loaded.map_err(Failed::Fetch).and_then(|fetched| apply(app, sheet, index, &fetched, window));
        match outcome {
            Ok(rect) if sheet == app.sheet => written.push(rect),
            Ok(_) => {}
            Err(failed) => failures.push((import, app.wb.sheets[sheet].name.clone(), failed)),
        }
    }
    app.commit_cells(label, &written, ChangeCause::Fill, cx);
    report(app, failures, cx);
    if remote.is_empty() {
        return None;
    }
    app.set_status(t("Connecting…", "Connexion…"), cx);
    let fetching = cx.background_executor().spawn(async move {
        // One address is fetched once, however many queries read it.
        let mut fetched: Vec<(String, Result<Fetched, FetchError>)> = Vec::new();
        for (_, address) in &remote {
            if !fetched.iter().any(|(known, _)| known == address) {
                fetched.push((address.clone(), sources::fetch(address)));
            }
        }
        (remote, fetched)
    });
    Some(cx.spawn_in(window, async move |this, cx| {
        let (remote, fetched) = fetching.await;
        this.update_in(cx, |app, window, cx| {
            app.set_status("", cx);
            let mut written = Vec::new();
            let mut failures = Vec::new();
            for (query, address) in remote {
                // A query deleted or renamed meanwhile is not refreshed.
                let Some((sheet, index)) = query.find(app) else { continue };
                let import = app.wb.sheets[sheet].text_imports[index].clone();
                let outcome = match fetched.iter().find(|(known, _)| *known == address) {
                    Some((_, Ok(fetched))) => apply(app, sheet, index, fetched, window),
                    Some((_, Err(error))) => Err(Failed::Fetch(*error)),
                    None => continue,
                };
                match outcome {
                    Ok(rect) if sheet == app.sheet => written.push(rect),
                    Ok(_) => {}
                    Err(failed) => failures.push((import, query.sheet.clone(), failed)),
                }
            }
            app.commit_cells(label, &written, ChangeCause::Fill, cx);
            report(app, failures, cx);
        })
        .ok();
    }))
}

/// Data ▸ Refresh All: every query is read again and every PivotTable
/// recomputed, in one undo step.
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
    let queries = all_queries(app);
    if let Some(task) = refresh_queries(app, queries, label, window, cx) {
        task.detach();
    }
    if ask_pivots {
        pivot::refresh_all(app, window, cx);
    }
}

/// Data ▸ Refresh All ▸ Refresh: the query or the PivotTable the active
/// cell is in.
pub(super) fn refresh_here(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let (sheet, (row, col)) = (app.sheet, app.active());
    if let Some(index) = app.wb.text_import_at(sheet, row, col) {
        let range = app.wb.sheets[sheet].text_imports[index].range;
        if !app.may(Intent::EditCells(range), cx) {
            return;
        }
        if let Some(task) = refresh_queries(app, vec![(sheet, index)], t("Refresh", "Actualiser"), window, cx) {
            task.detach();
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

/// A query on a document at `address` taking `item`, still to be placed.
pub(super) fn web_query(address: String, web: WebQuery) -> TextImport {
    TextImport {
        name: String::new(),
        path: address,
        range: Rect::cell(0, 0),
        first_row: 1,
        spec: opensheet_engine::text_columns::Spec::new(opensheet_engine::text_columns::Layout::Delimited(Default::default())),
        web: Some(web),
        props: Default::default(),
    }
}

// ------------------------------------------------- refreshing by itself

/// What the queries of the open workbook need watched: the question
/// asked once when a file wants to refresh on its own, and the clock of
/// "Refresh every n minutes".
#[derive(Default)]
pub(super) struct Watch {
    /// Whether this workbook's own schedule was already considered.
    opened: bool,
    /// `Some(true)` after the user agreed, or turned the interval on
    /// themselves. `Some(false)` when they declined.
    consented: Option<bool>,
    /// Local files the user picked in this session. A web query may read
    /// one of these; a path that arrived only inside the workbook may not.
    chosen: Vec<PathBuf>,
    /// Minutes since each query that refreshes by itself last did.
    waited: Vec<(QueryRef, u32)>,
    clock: Option<Task<()>>,
}

#[cfg(test)]
impl Watch {
    pub(super) fn timer_running(&self) -> bool {
        self.clock.is_some()
    }
}

/// The user picked `address` (the file dialog, or From JSON / XML /
/// Workbook). Remember it for this workbook only.
pub(super) fn remember_chosen_file(app: &mut ExcelApp, address: &str) {
    let Some(path) = sources::local_path(address) else { return };
    let chosen = &mut app.ext_mut::<Watch>().chosen;
    if !chosen.contains(&path) {
        chosen.push(path);
    }
}

fn user_chose_file(app: &ExcelApp, address: &str) -> bool {
    let Some(path) = sources::local_path(address) else { return false };
    app.ext::<Watch>().is_some_and(|watch| watch.chosen.iter().any(|known| known == &path))
}

/// Connection Properties just turned "Refresh every" on, which is the
/// user's own request: the next draw may start the clock.
pub(super) fn consent_to_timer(app: &mut ExcelApp) {
    app.ext_mut::<Watch>().consented = Some(true);
}

fn refreshes_on_a_timer(app: &ExcelApp) -> bool {
    app.wb.sheets.iter().any(|sheet| sheet.text_imports.iter().any(|import| import.props.refresh_minutes > 0))
}

fn wants_a_refresh_on_open(app: &ExcelApp) -> bool {
    app.wb.sheets.iter().any(|sheet| sheet.text_imports.iter().any(|import| import.props.refresh_on_open))
}

/// Called whenever the window is drawn: cheap unless a query asks for
/// something. A workbook that wants to refresh on opening, or on its own
/// interval, is asked once. The clock starts only after an agreement;
/// declining leaves it stopped, and Refresh still works by hand.
pub(super) fn watch(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let first = app.ext::<Watch>().is_none_or(|watch| !watch.opened);
    let timed = refreshes_on_a_timer(app);
    if !first {
        // The user has since turned the interval on in Connection Properties.
        if timed && app.ext::<Watch>().and_then(|watch| watch.consented) == Some(true) {
            start_clock(app, window, cx);
        }
        return;
    }
    app.ext_mut::<Watch>().opened = true;
    if !wants_a_refresh_on_open(app) && !timed {
        return;
    }
    // Already agreed in this session (they edited the properties before
    // the first draw): start, and do not ask a second time.
    if app.ext::<Watch>().and_then(|watch| watch.consented) == Some(true) {
        start_clock(app, window, cx);
        return;
    }
    // Not while the window is being drawn.
    cx.spawn_in(window, async move |this, cx| {
        this.update_in(cx, |app, _, cx| ask_to_refresh(app, cx)).ok();
    })
    .detach();
}

fn start_clock(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    if !refreshes_on_a_timer(app) || app.ext::<Watch>().is_some_and(|watch| watch.clock.is_some()) {
        return;
    }
    if app.ext::<Watch>().and_then(|watch| watch.consented) != Some(true) {
        return;
    }
    let clock = cx.spawn_in(window, async move |this, cx| {
        loop {
            cx.background_executor().timer(std::time::Duration::from_secs(60)).await;
            if this.update_in(cx, minute_passed).is_err() {
                break;
            }
        }
    });
    app.ext_mut::<Watch>().clock = Some(clock);
}

/// A file that wants to refresh its queries by itself: Excel asks first,
/// since the addresses are the file's and not the user's. One question
/// covers both "when opening the file" and "every n minutes".
fn ask_to_refresh(app: &mut ExcelApp, cx: &mut Cx) {
    let message = format!(
        "{}\n\n{}",
        t("The operation connects to an external data source.", "L’opération se connecte à une source de données externe."),
        t(
            "External data sources are used to import data into Excel, but connections to external data sources can be used by queries to access confidential information available to other users, or to perform other harmful actions. If you trust the source of this file, click OK.",
            "Les sources de données externes servent à importer des données dans Excel, mais les connexions à des sources de données externes peuvent être utilisées par des requêtes pour accéder à des informations confidentielles disponibles pour d’autres utilisateurs ou pour effectuer d’autres opérations dangereuses. Si vous faites confiance à la source de ce fichier, cliquez sur OK.",
        )
    );
    let alert = Alert::new("OpenSheet", message)
        .warning()
        .button(t("OK", "OK"), |app, window, cx| {
            app.ext_mut::<Watch>().consented = Some(true);
            let queries = all_queries(app).into_iter().filter(|(sheet, index)| app.wb.sheets[*sheet].text_imports[*index].props.refresh_on_open).collect();
            if let Some(task) = refresh_queries(app, queries, t("Refresh", "Actualiser"), window, cx) {
                task.detach();
            }
            start_clock(app, window, cx);
        })
        .button(t("Cancel", "Annuler"), |app, _, _| {
            app.ext_mut::<Watch>().consented = Some(false);
        });
    app.alert(alert, cx);
}

/// A minute went by: the queries whose time has come are refreshed.
/// Nothing happens until the user has agreed to this workbook's schedule.
pub(super) fn minute_passed(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    if app.ext::<Watch>().and_then(|watch| watch.consented) != Some(true) {
        return;
    }
    let mut due = Vec::new();
    let mut waited = Vec::new();
    for (sheet, index) in all_queries(app) {
        let every = app.wb.sheets[sheet].text_imports[index].props.refresh_minutes;
        let Some(query) = QueryRef::of(app, sheet, index).filter(|_| every > 0) else { continue };
        let before = app.ext::<Watch>().and_then(|watch| watch.waited.iter().find(|(known, _)| *known == query)).map_or(0, |(_, minutes)| *minutes);
        if before + 1 >= every {
            due.push((sheet, index));
            waited.push((query, 0));
        } else {
            waited.push((query, before + 1));
        }
    }
    app.ext_mut::<Watch>().waited = waited;
    // Not over a dialog or a cell being typed: the next minute will do.
    if due.is_empty() || app.is_editing() || app.modal_open() {
        return;
    }
    if let Some(task) = refresh_queries(app, due, t("Refresh", "Actualiser"), window, cx) {
        task.detach();
    }
}
