//! The "Table Design" tab, shown while the active cell is in a table:
//! Properties (the Table Name box, Resize Table), Tools, External Table
//! Data, Table Style Options and Table Styles.

use super::actions::{self, NameProblem};
use super::{Cmd, Doing, TablesUi, active_table, refusal, styles};
use crate::app::ExcelApp;
use crate::features::{self, Command, Cx, Intent};
use crate::i18n::t;
use crate::ribbon::{group, row, rows};
use crate::shell::Alert;
use crate::text_input::{InputEvent, TextInput};
use crate::widgets::TextField;
use gpui::{AnyElement, Entity, Focusable, Subscription, Window, div, prelude::*, px};

/// The Table Name box: a text field of the ribbon.
pub(super) struct NameBox {
    input: Entity<TextInput>,
    /// The table whose name is being typed (sheet, id), from the moment
    /// the box takes the keyboard: the active cell may be elsewhere by the
    /// time the name is entered.
    target: Option<(usize, u32)>,
    _subscription: Subscription,
}

/// The field of the Table Name box, showing the name of the active table
/// unless one is being typed.
fn name_box(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Entity<TextInput> {
    let existing = app
        .ext::<TablesUi>()
        .and_then(|ui| ui.name_box.as_ref())
        .map(|name_box| name_box.input.clone());
    let input = existing.unwrap_or_else(|| {
        let input = cx.new(|cx| {
            let mut input = TextInput::new(cx);
            input.select_on_focus = true;
            input
        });
        let subscription = cx.subscribe_in(&input, window, |app, _, ev, window, cx| {
            on_name_event(app, ev, window, cx)
        });
        app.ext_mut::<TablesUi>().name_box = Some(NameBox {
            input: input.clone(),
            target: None,
            _subscription: subscription,
        });
        input
    });
    if !input.read(cx).focus_handle(cx).is_focused(window) {
        if let Some(name) = active_table(app).map(|table| table.name.clone()) {
            if input.read(cx).text() != name {
                input.update(cx, |input, cx| input.set_text(name, cx));
            }
        }
    }
    input
}

fn on_name_event(app: &mut ExcelApp, ev: &InputEvent, window: &mut Window, cx: &mut Cx) {
    match ev {
        InputEvent::Focused => {
            let target = active_table(app).map(|table| (app.sheet, table.id));
            if let Some(name_box) = &mut app.ext_mut::<TablesUi>().name_box {
                name_box.target = target;
            }
        }
        InputEvent::Enter { .. } | InputEvent::Tab { .. } => {
            if rename(app, true, cx) {
                window.focus(&app.grid_focus);
            }
        }
        // Leaving the box enters the name too, when it is a good one; a
        // bad one is dropped without a word.
        InputEvent::Blurred => {
            rename(app, false, cx);
            if let Some(name_box) = &mut app.ext_mut::<TablesUi>().name_box {
                name_box.target = None;
            }
            cx.notify();
        }
        InputEvent::Escape => {
            if let Some(name_box) = &mut app.ext_mut::<TablesUi>().name_box {
                name_box.target = None;
            }
            window.focus(&app.grid_focus);
            cx.notify();
        }
        _ => {}
    }
}

/// The selection moved while a name was being typed in the Table Name box
/// (a click on the sheet). The name is entered as when the box is left:
/// now, because the box may be gone, or showing another table, before it
/// learns that it lost the keyboard.
pub(super) fn selection_changed(app: &mut ExcelApp, cx: &mut Cx) {
    let typing = app
        .ext::<TablesUi>()
        .and_then(|ui| ui.name_box.as_ref())
        .is_some_and(|name_box| name_box.target.is_some());
    if typing {
        rename(app, false, cx);
        if let Some(name_box) = &mut app.ext_mut::<TablesUi>().name_box {
            name_box.target = None;
        }
    }
}

/// Give the table of the Table Name box the name typed there. `loud`:
/// say why when the name is refused. Returns true when the table now has
/// the name typed (or already had it).
fn rename(app: &mut ExcelApp, loud: bool, cx: &mut Cx) -> bool {
    let Some(name_box) = app.ext::<TablesUi>().and_then(|ui| ui.name_box.as_ref()) else {
        return false;
    };
    let input = name_box.input.clone();
    let name = input.read(cx).text().trim().to_string();
    let Some((sheet, id)) = name_box.target else {
        return true;
    };
    match app.wb.table(sheet, id) {
        Some(table) if table.name == name => return true,
        Some(_) => {}
        None => return false,
    }
    let allowed = if loud {
        app.may(Intent::ChangeTable, cx)
    } else {
        features::guard(app, &Intent::ChangeTable).is_none()
    };
    if !allowed {
        return false;
    }
    match app.wb.rename_table(sheet, id, &name) {
        Ok(()) => {
            app.commit(t("Rename Table", "Renommer le tableau"), cx);
            true
        }
        Err(error) => {
            if loud {
                let message = match actions::name_problem(&app.wb, error, &name) {
                    NameProblem::TableExists => Some(t(
                        "A table with that name already exists. Select a different name.",
                        "Un tableau portant ce nom existe déjà. Choisissez un autre nom.",
                    )),
                    NameProblem::Syntax => refusal(error, Doing::Other),
                };
                if let Some(message) = message {
                    // After the message the box has the keyboard again.
                    let handle = input.read(cx).focus_handle(cx);
                    app.alert(
                        Alert::new("OpenSheet", message)
                            .warning()
                            .button(t("OK", "OK"), move |_, window, _| window.focus(&handle)),
                        cx,
                    );
                }
            }
            false
        }
    }
}

/// The groups of the tab.
pub(super) fn groups(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
    let name = name_box(app, window, cx);
    // The filter buttons need the header row: without it the box is greyed.
    let filter_button = div()
        .when(!Cmd::FilterButton.enabled(app), |d| d.opacity(0.45))
        .child(app.cmd_check("td-filter", Cmd::FilterButton, cx));
    vec![
        group(
            t("Properties", "Propriétés"),
            rows(vec![
                div()
                    .px(px(4.))
                    .child(t("Table Name:", "Nom du tableau :"))
                    .into_any_element(),
                div()
                    .px(px(4.))
                    .child(TextField::new(&name).width(132.0))
                    .into_any_element(),
                app.cmd_labeled("td-resize", Cmd::Resize, false, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Tools", "Outils"),
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(
                    rows(vec![
                        app.cmd_labeled("td-pivot", Cmd::SummarizeWithPivot, false, cx),
                        app.cmd_labeled("td-duplicates", Cmd::RemoveDuplicates, false, cx),
                        app.cmd_labeled("td-range", Cmd::ConvertToRange, false, cx),
                    ])
                    .items_start(),
                )
                .child(app.cmd_large("td-slicer", crate::features::slicers::Cmd::InsertSlicer, false, cx)),
        )
        .into_any_element(),
        group(
            t("External Table Data", "Données de tableau externe"),
            row(vec![
                app.cmd_large("td-export", Cmd::Export, true, cx),
                app.cmd_large("td-refresh", Cmd::Refresh, true, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Table Style Options", "Options de style de tableau"),
            div()
                .flex()
                .gap(px(8.))
                .child(rows(vec![
                    app.cmd_check("td-header", Cmd::HeaderRow, cx),
                    app.cmd_check("td-total", Cmd::TotalRow, cx),
                    app.cmd_check("td-banded-rows", Cmd::BandedRows, cx),
                ]))
                .child(rows(vec![
                    app.cmd_check("td-first", Cmd::FirstColumn, cx),
                    app.cmd_check("td-last", Cmd::LastColumn, cx),
                    app.cmd_check("td-banded-cols", Cmd::BandedColumns, cx),
                ]))
                .child(rows(vec![filter_button.into_any_element()])),
        )
        .into_any_element(),
        // The last group takes the room left, for as many styles as fit.
        group(t("Table Styles", "Styles de tableau"), styles::strip(app, cx))
            .flex_1()
            .min_w(px(80.))
            .border_r_0()
            .into_any_element(),
    ]
}
