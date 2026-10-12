//! The two dialogs of tables: Create Table ("Where is the data for your
//! table?") and Resize Table. While one is open, the range in its field is
//! outlined on the sheet (see `sheet::paint`), and cells picked on the
//! sheet are written into the field.

use super::actions::parse_range;
use super::{RANGE_FIELD, TablesUi, active_table, create, invalid_reference, resize};
use crate::app::ExcelApp;
use crate::features::{Cx, Intent};
use crate::i18n::t;
use crate::shell::{Alert, Modal};
use crate::text_input::TextInput;
use crate::theme::{self, c};
use crate::widgets::{Checkbox, RefEdit, dialog, label, ok_cancel};
use gpui::{Context, Entity, Focusable, Window, div, prelude::*};
use opensheet_engine::Rect;
use std::cell::Cell;
use std::rc::Rc;

/// The range field of a dialog, and the outline of its range on the sheet.
struct RangeField {
    modal: Modal,
    input: Entity<TextInput>,
    sheet: usize,
    sheet_name: String,
    /// What the sheet outlines (shared with `TablesUi::shown`): gone with
    /// the dialog.
    shown: Rc<Cell<Option<(usize, Rect)>>>,
}

impl RangeField {
    /// To call from the constructor of the dialog `V`, which owns it as
    /// its field `range`.
    fn new<V: 'static>(
        modal: &Modal,
        sheet: usize,
        sheet_name: String,
        range: Rect,
        shown: Rc<Cell<Option<(usize, Rect)>>>,
        cx: &mut Context<V>,
    ) -> RangeField {
        RangeField {
            modal: modal.clone(),
            input: modal.ref_input(RANGE_FIELD, RANGE_FIELD.format(range, &sheet_name), cx),
            sheet,
            sheet_name,
            shown,
        }
    }

    /// The range typed, if it is one of the dialog's sheet.
    fn range(&self, cx: &gpui::App) -> Option<Rect> {
        parse_range(self.input.read(cx).text(), &self.sheet_name)
    }

    /// Bring the outline on the sheet in line with the field.
    fn show(&self, cx: &mut gpui::App) {
        let shown = self.range(cx).map(|range| (self.sheet, range));
        if self.shown.get() != shown {
            self.shown.set(shown);
            self.modal.update_app(cx, |_, cx| cx.notify());
        }
    }

    /// The range to act on when OK is pressed. Without one, Excel's
    /// message is shown and the field takes the keyboard back.
    fn accept(&self, cx: &mut gpui::App) -> Option<Rect> {
        let range = self.range(cx);
        if range.is_none() {
            let (input, handle) = (self.input.clone(), self.input.read(cx).focus_handle(cx));
            self.modal.update_app(cx, |app, cx| {
                app.alert(
                    Alert::new("OpenSheet", invalid_reference()).warning().button(
                        t("OK", "OK"),
                        move |_, window, cx| {
                            window.focus(&handle);
                            input.update(cx, |input, cx| input.select_all_text(cx));
                        },
                    ),
                    cx,
                )
            });
        }
        range
    }
}

/// Start showing a range on the sheet for a dialog about to open.
fn show_on_sheet(app: &mut ExcelApp, range: Rect) -> Rc<Cell<Option<(usize, Rect)>>> {
    let shown = Rc::new(Cell::new(Some((app.sheet, range))));
    app.ext_mut::<TablesUi>().shown = Rc::downgrade(&shown);
    shown
}

// ------------------------------------------------------------- Create Table

struct CreateTable {
    modal: Modal,
    range: RangeField,
    headers: bool,
    /// The style picked in the Format as Table gallery, if the dialog
    /// comes from there.
    style: Option<Option<&'static str>>,
}

/// Insert > Table, and what follows the choice of a style in Format as
/// Table: ask where the data is. The range offered is the selection, or
/// the block of data around the active cell.
pub(super) fn open_create(app: &mut ExcelApp, style: Option<Option<&'static str>>, window: &mut Window, cx: &mut Cx) {
    if app.sel().ranges.len() > 1 {
        app.alert(
            Alert::new(
                "OpenSheet",
                t(
                    "This action won't work on multiple selections.",
                    "Cette action ne peut pas être effectuée sur des sélections multiples.",
                ),
            )
            .warning(),
            cx,
        );
        return;
    }
    if !app.may(Intent::ChangeTable, cx) {
        return;
    }
    let sheet = app.sheet;
    let (range, headers) = app.wb.suggest_table_range(sheet, app.sel().current());
    let sheet_name = app.wb.sheets[sheet].name.clone();
    let shown = show_on_sheet(app, range);
    app.open_modal(window, cx, move |modal, _, cx| {
        let range = RangeField::new(&modal, sheet, sheet_name, range, shown, cx);
        cx.observe(&range.input, |this: &mut CreateTable, _, cx| this.range.show(cx))
            .detach();
        CreateTable {
            modal,
            range,
            headers,
            style,
        }
    });
}

impl CreateTable {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let Some(range) = self.range.accept(cx) else {
            return;
        };
        let (sheet, headers, style) = (self.range.sheet, self.headers, self.style);
        self.modal.close(window, cx);
        self.modal
            .update_app(cx, |app, cx| create(app, sheet, range, headers, style, cx));
    }
}

impl Render for CreateTable {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        dialog(&self.modal, t("Create Table", "Créer un tableau"))
            .width(330.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(t(
                "Where is the data for your table?",
                "Où se trouvent les données de votre tableau ?",
            )))
            .child(div().flex().child(RefEdit::new(&self.range.input, &self.modal)))
            .child(
                Checkbox::new(
                    "headers",
                    t("My table has headers", "Mon tableau comporte des en-têtes"),
                    self.headers,
                )
                .on_change(cx.listener(|this, on: &bool, _, cx| {
                    this.headers = *on;
                    cx.notify();
                })),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// ------------------------------------------------------------- Resize Table

struct ResizeTable {
    modal: Modal,
    range: RangeField,
    table: u32,
    /// The table shows its header row (the note of the dialog says which
    /// row must stay).
    headers: bool,
}

/// Table Design > Resize Table.
pub(super) fn open_resize(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some((table, range, headers)) = active_table(app).map(|table| (table.id, table.range, table.header_row)) else {
        return;
    };
    if !app.may(Intent::ChangeTable, cx) {
        return;
    }
    let sheet = app.sheet;
    let sheet_name = app.wb.sheets[sheet].name.clone();
    let shown = show_on_sheet(app, range);
    app.open_modal(window, cx, move |modal, _, cx| {
        let range = RangeField::new(&modal, sheet, sheet_name, range, shown, cx);
        cx.observe(&range.input, |this: &mut ResizeTable, _, cx| this.range.show(cx))
            .detach();
        ResizeTable {
            modal,
            range,
            table,
            headers,
        }
    });
}

impl ResizeTable {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let Some(range) = self.range.accept(cx) else {
            return;
        };
        let (sheet, table) = (self.range.sheet, self.table);
        self.modal.close(window, cx);
        self.modal
            .update_app(cx, |app, cx| resize(app, sheet, table, range, cx));
    }
}

impl Render for ResizeTable {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let note = if self.headers {
            t(
                "Note: The headers must remain in the same row, and the resulting table range must overlap the original table range.",
                "Remarque : les en-têtes doivent rester sur la même ligne et la plage de données générée doit chevaucher la plage du tableau d’origine.",
            )
        } else {
            t(
                "Note: The first row in the table must remain in the same row, and the resulting table range must overlap the original table range",
                "Remarque : la première ligne du tableau doit rester sur la même ligne et la plage de données générée doit chevaucher la plage du tableau d’origine",
            )
        };
        dialog(&self.modal, t("Resize Table", "Redimensionner le tableau"))
            .width(360.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(t(
                "Select the new data range for your table:",
                "Sélectionnez la nouvelle plage de données pour votre tableau :",
            )))
            .child(div().flex().child(RefEdit::new(&self.range.input, &self.modal)))
            .child(div().w_full().text_color(c(theme::TEXT_MUTED)).child(note))
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}
