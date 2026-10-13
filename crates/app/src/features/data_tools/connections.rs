//! The dialogs that manage queries, as Excel for Mac has them: Workbook
//! Queries & Connections (the list: Refresh, Properties, Remove, Change
//! File Path), Connection Properties (name, description, when the query
//! refreshes, its address and Edit Query) and External Data Properties
//! (the name of the range, what a refresh does to formats and widths).

use super::import::{self, QueryRef};
use super::{navigator, sources, warn};
use crate::app::ExcelApp;
use crate::features::Cx;
use crate::i18n::t;
use crate::shell::{Alert, Modal, RefStyle};
use crate::text_input::TextInput;
use crate::theme::{self, c};
use crate::widgets::{self, Button, Checkbox, ListBox, NumberField, TabStrip, TextField, dialog, label, labeled, ok_cancel};
use gpui::{Context, Entity, PathPromptOptions, SharedString, Window, div, prelude::*, px};
use opensheet_engine::external::QueryProps;
use opensheet_engine::text_columns::TextImport;

/// The queries of the workbook, as the dialogs name them.
fn queries(app: &ExcelApp) -> Vec<QueryRef> {
    import::all_queries(app).into_iter().filter_map(|(sheet, index)| QueryRef::of(app, sheet, index)).collect()
}

/// Whether a query reads a file of this machine, whose path can change.
fn reads_a_file(import: &TextImport) -> bool {
    sources::local_path(&import.path).is_some()
}

// ---------------------------------------------- Queries & Connections

/// Data ▸ Queries & Connections.
pub(super) fn open(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Entity<ConnectionsDialog> {
    let list = queries(app);
    let selected = (!list.is_empty()).then_some(0);
    app.open_modal(window, cx, move |modal, _, _| ConnectionsDialog {
        modal,
        list,
        selected,
        locations: false,
    })
}

pub(super) struct ConnectionsDialog {
    modal: Modal,
    list: Vec<QueryRef>,
    selected: Option<usize>,
    /// Whether "where the selected connections are used" was asked.
    locations: bool,
}

impl ConnectionsDialog {
    pub(super) fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = Some(index);
        self.locations = false;
        cx.notify();
    }

    fn query(&self) -> Option<QueryRef> {
        self.list.get(self.selected?).cloned()
    }

    /// The list as the workbook has it now: a query may have been
    /// renamed, removed or brought back since it was last drawn.
    fn reload(&mut self, cx: &Context<Self>) {
        self.list = self.modal.read_app(cx, queries).unwrap_or_default();
        self.selected = match self.list.len() {
            0 => None,
            count => Some(self.selected.unwrap_or(0).min(count - 1)),
        };
    }

    pub(super) fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(query) = self.query() else { return };
        self.modal.update_app(cx, |app, cx| {
            if let Some(found) = query.find(app) {
                if let Some(task) = import::refresh_queries(app, vec![found], t("Refresh", "Actualiser"), window, cx) {
                    task.detach();
                }
            }
        });
    }

    pub(super) fn properties(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(query) = self.query() else { return };
        self.modal.update_app(cx, |app, cx| {
            open_properties(app, &query, window, cx);
        });
    }

    /// Remove: Excel's warning first, since the data can no longer be
    /// refreshed afterwards.
    pub(super) fn remove(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(query) = self.query() else { return };
        let alert = Alert::new(
            "OpenSheet",
            t(
                "Removing connection(s) will separate this workbook from its data source(s), and data refresh operations in the workbook will no longer succeed. Click Cancel to keep connection(s), or click OK to proceed with removal.",
                "La suppression de la ou des connexions séparera ce classeur de sa ou ses sources de données, et les opérations d'actualisation des données de ce classeur ne fonctionneront plus. Cliquez sur Annuler pour conserver la ou les connexions, ou sur OK pour poursuivre la suppression.",
            ),
        )
        .warning()
        .button(t("OK", "OK"), move |app, _, cx| {
            if let Some((sheet, index)) = query.find(app) {
                app.wb.remove_import(sheet, index);
                app.commit(t("Delete Query", "Supprimer la requête"), cx);
            }
        })
        .close_button(t("Cancel", "Annuler"));
        self.modal.update_app(cx, |app, cx| app.alert(alert, cx));
    }

    /// Change File Path: the system's dialog, then the query reads the
    /// file chosen.
    fn change_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(query) = self.query() else { return };
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        let modal = self.modal.clone();
        cx.spawn_in(window, async move |_, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                if let Some(path) = paths.into_iter().next() {
                    cx.update(|_, cx| {
                        modal.update_app(cx, |app, cx| set_path(app, &query, &path, cx));
                    })
                    .ok();
                }
            }
        })
        .detach();
    }

    fn selected_import(&self, cx: &Context<Self>) -> Option<(TextImport, String)> {
        let query = self.query()?;
        self.modal.read_app(cx, |app| {
            let (sheet, index) = query.find(app)?;
            let on = &app.wb.sheets[sheet];
            let import = on.text_imports[index].clone();
            let place = RefStyle {
                absolute: true,
                sheet: true,
                equals: false,
            }
            .format(import.range, &on.name);
            Some((import, place))
        })?
    }
}

/// The query now reads the file at `path`, written the way its address
/// was (a path for text, a `file://` URL for documents).
pub(super) fn set_path(app: &mut ExcelApp, query: &QueryRef, path: &std::path::Path, cx: &mut Cx) {
    let Some((sheet, index)) = query.find(app) else { return };
    let address = match app.wb.sheets[sheet].text_imports[index].web {
        Some(_) => sources::file_url(path),
        None => path.to_string_lossy().into_owned(),
    };
    app.wb.update_import(sheet, index, |import| import.path = address);
    app.commit(t("Change File Path", "Modifier le chemin d’accès au fichier"), cx);
}

impl Render for ConnectionsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.reload(cx);
        let names: Vec<SharedString> = self.list.iter().map(|query| query.name.clone().into()).collect();
        let chosen = self.selected_import(cx);
        let none = chosen.is_none();
        let file = chosen.as_ref().is_some_and(|(import, _)| reads_a_file(import));
        // Wide enough for the longest label in the five languages.
        let side = |button: Button| button.width(240.0);
        let modal = self.modal.clone();
        let locations = match (&chosen, self.locations) {
            (Some((import, place)), true) => div().child(SharedString::from(format!("{}    {}", import.name, place))),
            _ => div().text_color(c(theme::ACCENT)).child(
                div()
                    .id("connections-where")
                    .cursor_pointer()
                    .child(t(
                        "Click here to see where the selected connections are used",
                        "Cliquez ici pour savoir où les connexions sélectionnées sont utilisées",
                    ))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.locations = true;
                        cx.notify();
                    })),
            ),
        };
        dialog(&self.modal, t("Workbook Queries & Connections", "Requêtes et connexions du classeur"))
            .width(600.0)
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(
                        ListBox::new("connections", names, self.selected)
                            .width(320.0)
                            .height(widgets::ROW_H * 8.0)
                            .on_change(cx.listener(|this, index: &usize, _, cx| this.select(*index, cx)))
                            .on_activate(cx.listener(|this, index: &usize, window, cx| {
                                this.select(*index, cx);
                                this.properties(window, cx);
                            })),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(side(Button::new("connections-refresh", t("Refresh", "Actualiser"))).disabled(none).on_click(cx.listener(|this, _, window, cx| this.refresh(window, cx))))
                            .child(
                                side(Button::new("connections-properties", t("Properties...", "Propriétés...")))
                                    .disabled(none)
                                    .on_click(cx.listener(|this, _, window, cx| this.properties(window, cx))),
                            )
                            .child(side(Button::new("connections-remove", t("Remove", "Supprimer"))).disabled(none).on_click(cx.listener(|this, _, window, cx| this.remove(window, cx))))
                            .child(
                                side(Button::new("connections-path", t("Change File Path...", "Modifier le chemin d’accès au fichier...")))
                                    .disabled(!file)
                                    .on_click(cx.listener(|this, _, window, cx| this.change_path(window, cx))),
                            )
                            // No source of this program asks for a sign-in,
                            // so there is never a permission to clear.
                            .child(side(Button::new("connections-permissions", t("Clear Permissions", "Effacer les autorisations"))).disabled(true)),
                    ),
            )
            .child(label(t(
                "Locations where connections are used in this workbook",
                "Emplacements dans ce classeur où des connexions sont utilisées",
            )))
            .child(
                div()
                    .h(px(44.))
                    .px(px(6.))
                    .py(px(4.))
                    .border_1()
                    .border_color(c(theme::BORDER))
                    .bg(c(0xFFFFFF))
                    .when(!none, |d| d.child(locations)),
            )
            .footer(Button::new("connections-close", t("Close", "Fermer")).primary().on_click(move |_, window, cx| modal.close(window, cx)))
    }
}

// ------------------------------------------------ Connection Properties

fn kind_name(import: &TextImport) -> &'static str {
    match import.web {
        Some(_) => t("Web Query", "Requête sur le web"),
        None => t("Text", "Texte"),
    }
}

/// Connection Properties of a query; `None` when it is gone.
pub(super) fn open_properties(app: &mut ExcelApp, query: &QueryRef, window: &mut Window, cx: &mut Cx) -> Option<Entity<PropertiesDialog>> {
    let (sheet, index) = query.find(app)?;
    let import = app.wb.sheets[sheet].text_imports[index].clone();
    let query = query.clone();
    Some(app.open_modal(window, cx, move |modal, _, cx| PropertiesDialog {
        name: widgets::input(cx, import.name.clone()),
        description: widgets::input(cx, import.props.description.clone()),
        // Excel proposes an hour.
        minutes: widgets::number_input(cx, Some(if import.props.refresh_minutes == 0 { 60.0 } else { f64::from(import.props.refresh_minutes) }), 1.0, 32767.0, 1.0),
        address: widgets::input(cx, import.path.clone()),
        timed: import.props.refresh_minutes > 0,
        props: import.props.clone(),
        import,
        query,
        tab: 0,
        modal,
    }))
}

pub(super) struct PropertiesDialog {
    modal: Modal,
    query: QueryRef,
    /// The query as it was when the dialog opened.
    import: TextImport,
    name: Entity<TextInput>,
    description: Entity<TextInput>,
    props: QueryProps,
    /// "Refresh every" is ticked.
    timed: bool,
    minutes: Entity<TextInput>,
    address: Entity<TextInput>,
    tab: usize,
}

impl PropertiesDialog {
    #[cfg(test)]
    pub(super) fn set(&mut self, name: &str, description: &str, every: Option<u32>, on_open: bool, address: &str, cx: &mut Context<Self>) {
        self.name.update(cx, |field, cx| field.set_text(name, cx));
        self.description.update(cx, |field, cx| field.set_text(description, cx));
        self.timed = every.is_some();
        if let Some(minutes) = every {
            self.minutes.update(cx, |field, cx| field.set_text(minutes.to_string(), cx));
        }
        self.props.refresh_on_open = on_open;
        self.address.update(cx, |field, cx| field.set_text(address, cx));
        cx.notify();
    }

    /// The query as the dialog now describes it; the error to show when
    /// it cannot be so.
    fn edited(&self, app: &ExcelApp, cx: &Context<Self>) -> Result<TextImport, &'static str> {
        let mut import = self.import.clone();
        import.name = self.name.read(cx).text().trim().to_string();
        let at = self.query.find(app).ok_or(t("The original external data range is no longer valid.", "La plage de données externes d'origine n'est plus valide."))?;
        if !app.wb.import_name_free(&import.name, at) {
            return Err(t("The name that you entered is not valid.", "Le nom que vous avez entré n’est pas valide."));
        }
        import.props = self.props.clone();
        import.props.description = self.description.read(cx).text().trim().to_string();
        import.props.refresh_minutes = match (self.timed, self.minutes.read(cx).number()) {
            (false, _) => 0,
            (true, Some(minutes)) if (1.0..=32767.0).contains(&minutes) => minutes as u32,
            (true, _) => return Err(t("Integer is not valid.", "Nombre entier non valide.")),
        };
        let address = self.address.read(cx).text().trim().to_string();
        if address.is_empty() {
            return Err(t("Invalid Web Query", "Requête sur le web non valide"));
        }
        import.path = address;
        Ok(import)
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let query = self.query.clone();
        let outcome = self.modal.read_app(cx, |app| self.edited(app, cx));
        let done = match outcome {
            Some(Ok(import)) => self.modal.update_app(cx, |app, cx| apply(app, &query, import, cx)).is_some(),
            Some(Err(message)) => {
                self.modal.update_app(cx, |app, cx| warn(app, message, cx));
                false
            }
            None => true,
        };
        if done {
            self.modal.close(window, cx);
        }
    }

    /// Edit Query: the address, then the Navigator on what it holds.
    fn edit_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (query, address) = (self.query.clone(), self.address.read(cx).text().to_string());
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            navigator::open_web(app, &address, Some(query), window, cx);
        });
    }
}

/// Give a query its edited name and properties, in one undo step.
fn apply(app: &mut ExcelApp, query: &QueryRef, edited: TextImport, cx: &mut Cx) {
    let Some((sheet, index)) = query.find(app) else { return };
    if app.wb.sheets[sheet].text_imports[index] == edited {
        return;
    }
    app.wb.update_import(sheet, index, |import| {
        // Where the data is may have moved while the dialog was open.
        let range = import.range;
        *import = edited;
        import.range = range;
    });
    app.commit(t("Connection Properties", "Propriétés de connexion"), cx);
}

impl Render for PropertiesDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let web = self.import.web.is_some();
        let page = if self.tab == 0 {
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    Checkbox::new("connection-background", t("Enable background refresh", "Actualiser en arrière-plan"), self.props.background).on_change(cx.listener(
                        |this, on: &bool, _, cx| {
                            this.props.background = *on;
                            cx.notify();
                        },
                    )),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(Checkbox::new("connection-timed", t("Refresh every", "Actualiser toutes les"), self.timed).on_change(cx.listener(|this, on: &bool, _, cx| {
                            this.timed = *on;
                            cx.notify();
                        })))
                        .child(div().when(!self.timed, |d| d.opacity(0.5)).child(NumberField::new("connection-minutes", &self.minutes).width(70.0)))
                        .child(t("minutes", "minutes")),
                )
                .child(
                    Checkbox::new(
                        "connection-on-open",
                        t("Refresh data when opening the file", "Actualiser les données lors de l’ouverture du fichier"),
                        self.props.refresh_on_open,
                    )
                    .on_change(cx.listener(|this, on: &bool, _, cx| {
                        this.props.refresh_on_open = *on;
                        cx.notify();
                    })),
                )
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(labeled(t("Connection type:", "Type de connexion :"), div().child(kind_name(&self.import))))
                .child(label(t("Connection string:", "Chaîne de connexion :")))
                .child(TextField::new(&self.address).width(420.0))
                .child(
                    div().flex().child(
                        Button::new("connection-edit-query", t("Edit Query...", "Modifier la requête..."))
                            .disabled(!web)
                            .on_click(cx.listener(|this, _, window, cx| this.edit_query(window, cx))),
                    ),
                )
        };
        dialog(&self.modal, t("Connection Properties", "Propriétés de connexion"))
            .width(460.0)
            .on_ok(cx.listener(Self::ok))
            .child(labeled(t("Connection name:", "Nom de la connexion :"), TextField::new(&self.name).width(280.0)))
            .child(labeled(t("Description:", "Description :"), TextField::new(&self.description).width(280.0)))
            .child(TabStrip::new("connection-tabs", vec![t("Usage", "Utilisation").into(), t("Definition", "Définition").into()], self.tab).on_change(cx.listener(
                |this, tab: &usize, _, cx| {
                    this.tab = *tab;
                    cx.notify();
                },
            )))
            .child(div().min_h(px(120.)).child(page))
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// --------------------------------------------- External Data Properties

/// Data ▸ Properties: the external data range under the active cell.
pub(super) fn open_range_properties(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Option<Entity<RangeDialog>> {
    let (row, col) = app.active();
    let index = app.wb.text_import_at(app.sheet, row, col)?;
    let query = QueryRef::of(app, app.sheet, index)?;
    let import = app.wb.sheets[app.sheet].text_imports[index].clone();
    Some(app.open_modal(window, cx, move |modal, _, cx| RangeDialog {
        name: widgets::input(cx, import.name.clone()),
        props: import.props.clone(),
        import,
        query,
        modal,
    }))
}

pub(super) struct RangeDialog {
    modal: Modal,
    query: QueryRef,
    import: TextImport,
    name: Entity<TextInput>,
    props: QueryProps,
}

impl RangeDialog {
    #[cfg(test)]
    pub(super) fn set(&mut self, preserve_formatting: bool, adjust_width: bool, cx: &mut Context<Self>) {
        self.props.preserve_formatting = preserve_formatting;
        self.props.adjust_width = adjust_width;
        cx.notify();
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let mut edited = self.import.clone();
        edited.name = self.name.read(cx).text().trim().to_string();
        edited.props = self.props.clone();
        let query = self.query.clone();
        let free = self.modal.read_app(cx, |app| query.find(app).is_none_or(|at| app.wb.import_name_free(&edited.name, at)));
        if free == Some(false) {
            self.modal.update_app(cx, |app, cx| warn(app, t("The name that you entered is not valid.", "Le nom que vous avez entré n’est pas valide."), cx));
            return;
        }
        self.modal.update_app(cx, |app, cx| apply(app, &query, edited, cx));
        self.modal.close(window, cx);
    }

    /// The Connection Properties of the same query, over this dialog.
    fn connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.query.clone();
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            open_properties(app, &query, window, cx);
        });
    }
}

impl Render for RangeDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        dialog(&self.modal, t("External Data Properties", "Propriétés des données externes"))
            .width(420.0)
            .on_ok(cx.listener(Self::ok))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(labeled(t("Name:", "Nom :"), TextField::new(&self.name).width(200.0)))
                    .child(Button::new("range-connection", t("Connection Properties...", "Propriétés de connexion...")).on_click(cx.listener(|this, _, window, cx| this.connection(window, cx)))),
            )
            .child(label(t("Data formatting and layout", "Format et disposition des données")))
            .child(
                div()
                    .pl(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(
                        Checkbox::new("range-adjust", t("Adjust column width", "Ajustement de la largeur de la colonne"), self.props.adjust_width).on_change(cx.listener(
                            |this, on: &bool, _, cx| {
                                this.props.adjust_width = *on;
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        Checkbox::new("range-preserve", t("Preserve cell formatting", "Conserver la mise en forme des cellules"), self.props.preserve_formatting).on_change(cx.listener(
                            |this, on: &bool, _, cx| {
                                this.props.preserve_formatting = *on;
                                cx.notify();
                            },
                        )),
                    ),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}
