//! Get Data ▸ From Web and the Navigator.
//!
//! "From Web" asks for an address. The Navigator then lists what the
//! document there holds (the tables of a page, the sheets, tables and
//! names of a workbook, the lists of a JSON document) with a preview of
//! the one selected; Load puts it on a new sheet as Excel does, Load To
//! asks where. The same two dialogs edit a query: its address, then the
//! item it takes.

use super::import::{self, Payload, QueryRef};
use super::{sources, warn};
use crate::app::ExcelApp;
use crate::features::{ChangeCause, Cx, Intent};
use crate::i18n::t;
use crate::shell::Modal;
use crate::text_input::TextInput;
use crate::theme::{self, c};
use crate::widgets::{self, Button, ListBox, TextField, dialog, label, ok_cancel};
use gpui::{Context, Entity, SharedString, Window, div, prelude::*, px};
use opensheet_engine::external::{SourceTable, WebQuery};

/// Rows and columns of a table the preview shows.
const PREVIEW_ROWS: usize = 14;
const PREVIEW_COLS: usize = 8;

/// Data ▸ Get Data ▸ From Web, or Edit Query on a web query (`replaces`,
/// with its address).
pub(super) fn open_web(app: &mut ExcelApp, address: &str, replaces: Option<QueryRef>, window: &mut Window, cx: &mut Cx) -> Entity<WebDialog> {
    let address = address.to_string();
    app.open_modal(window, cx, move |modal, _, cx| WebDialog {
        address: widgets::input(cx, address),
        modal,
        replaces,
    })
}

pub(super) struct WebDialog {
    modal: Modal,
    address: Entity<TextInput>,
    replaces: Option<QueryRef>,
}

impl WebDialog {
    #[cfg(test)]
    pub(super) fn set_address(&mut self, text: &str, cx: &mut Context<Self>) {
        self.address.update(cx, |field, cx| field.set_text(text, cx));
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let address = sources::normalize(self.address.read(cx).text());
        if address.is_empty() {
            return;
        }
        let replaces = self.replaces.clone();
        // The dialog goes away first: what the address gives, or why it
        // gave nothing, is shown over the sheet.
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| import::open_address(app, address, replaces, window, cx));
    }
}

impl Render for WebDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        dialog(&self.modal, t("From Web", "À partir du web"))
            .width(460.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(t("URL", "URL")))
            .child(TextField::new(&self.address).width(430.0))
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

/// Show what a document holds.
pub(super) fn open(app: &mut ExcelApp, address: String, tables: Vec<SourceTable>, replaces: Option<QueryRef>, window: &mut Window, cx: &mut Cx) -> Entity<Navigator> {
    // Editing a query opens on the item it takes now.
    let current = replaces.as_ref().and_then(|query| query.find(app)).and_then(|(sheet, index)| app.wb.sheets[sheet].text_imports[index].web.clone());
    let selected = current.and_then(|web| tables.iter().position(|table| table.item == web.item)).unwrap_or(0);
    app.open_modal(window, cx, move |modal, _, _| Navigator {
        modal,
        address,
        tables,
        selected,
        replaces,
    })
}

pub(super) struct Navigator {
    modal: Modal,
    address: String,
    tables: Vec<SourceTable>,
    selected: usize,
    /// The query being edited rather than a new one being made.
    replaces: Option<QueryRef>,
}

impl Navigator {
    pub(super) fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.tables.len() {
            self.selected = index;
            cx.notify();
        }
    }

    #[cfg(test)]
    pub(super) fn names(&self) -> Vec<String> {
        self.tables.iter().map(|table| table.name.clone()).collect()
    }

    /// Load (on a new sheet) or Load To (asking where).
    pub(super) fn load(&mut self, ask: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(table) = self.tables.get(self.selected).cloned() else {
            return;
        };
        let web = WebQuery { item: table.item.clone() };
        let (address, replaces) = (self.address.clone(), self.replaces.clone());
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| match replaces {
            Some(query) => replace(app, &query, address, web, &table, window, cx),
            None => {
                // Named after the document, whatever its address looks like.
                let name = app.wb.text_import_name(&sources::document_name(&address));
                let mut import = import::web_query(address, web);
                import.name = name;
                if ask {
                    import::open_placement(app, import, Payload::Table(table), window, cx);
                } else {
                    import::place(app, &import, &Payload::Table(table), true, "", window, cx);
                }
            }
        });
    }

    fn preview(&self) -> impl IntoElement + use<> {
        let Some(table) = self.tables.get(self.selected) else {
            return div();
        };
        let columns = table.columns().min(PREVIEW_COLS);
        let rows = table.rows.iter().take(PREVIEW_ROWS).enumerate().map(|(i, row)| {
            div().flex().when(i == 0, |d| d.bg(c(theme::HEADER_BG)).font_weight(gpui::FontWeight::SEMIBOLD)).children((0..columns).map(|col| {
                let text = row.get(col).map(|datum| datum.preview()).unwrap_or_default();
                div()
                    .w(px(96.))
                    .h(px(20.))
                    .flex_none()
                    .px(px(4.))
                    .border_r_1()
                    .border_b_1()
                    .border_color(c(theme::BORDER))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(SharedString::from(text))
            }))
        });
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(div().font_weight(gpui::FontWeight::SEMIBOLD).child(SharedString::from(table.name.clone())))
            .child(
                div()
                    .w(px(420.))
                    .h(px(20. * PREVIEW_ROWS as f32 + 2.))
                    .border_1()
                    .border_color(c(theme::BORDER))
                    .bg(c(0xFFFFFF))
                    .overflow_hidden()
                    .text_size(px(11.5))
                    .children(rows),
            )
    }
}

/// Edit Query: the query now reads `address` and takes `web`; its data
/// is replaced by `table`, in one undo step.
fn replace(app: &mut ExcelApp, query: &QueryRef, address: String, web: WebQuery, table: &SourceTable, window: &mut Window, cx: &mut Cx) {
    let Some((sheet, index)) = query.find(app) else {
        return;
    };
    let range = app.wb.sheets[sheet].text_imports[index].range;
    if !app.may(Intent::EditCells(range), cx) {
        return;
    }
    app.wb.update_import(sheet, index, |import| {
        import.path = address;
        import.web = Some(web);
    });
    match app.wb.refresh_source_table(sheet, index, table) {
        Ok(written) => {
            if app.wb.sheets[sheet].text_imports[index].props.adjust_width {
                import::fit_columns(app, sheet, written, window);
            }
            app.commit_cells(t("Edit Query", "Modifier la requête"), &[written], ChangeCause::Fill, cx);
        }
        Err(_) => {
            app.wb.rollback();
            warn(app, t("Destination reference is not valid.", "Référence de destination non valide."), cx);
        }
    }
}

impl Render for Navigator {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let names: Vec<SharedString> = self.tables.iter().map(|table| table.name.clone().into()).collect();
        let modal = self.modal.clone();
        dialog(&self.modal, t("Navigator", "Navigateur"))
            .width(680.0)
            .on_ok(cx.listener(|this, _, window, cx| this.load(false, window, cx)))
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(
                        ListBox::new("navigator-items", names, Some(self.selected))
                            .width(210.0)
                            .height(20. * PREVIEW_ROWS as f32 + 28.)
                            .on_change(cx.listener(|this, index: &usize, _, cx| this.select(*index, cx)))
                            .on_activate(cx.listener(|this, index: &usize, window, cx| {
                                this.select(*index, cx);
                                this.load(false, window, cx);
                            })),
                    )
                    .child(self.preview()),
            )
            .footer(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(Button::new("navigator-load", t("Load", "Charger")).primary().on_click(cx.listener(|this, _, window, cx| this.load(false, window, cx))))
                    .when(self.replaces.is_none(), |d| {
                        d.child(Button::new("navigator-load-to", t("Load To...", "Charger dans...")).on_click(cx.listener(|this, _, window, cx| this.load(true, window, cx))))
                    })
                    .child(Button::new("navigator-cancel", t("Cancel", "Annuler")).on_click(move |_, window, cx| modal.close(window, cx))),
            )
    }
}
