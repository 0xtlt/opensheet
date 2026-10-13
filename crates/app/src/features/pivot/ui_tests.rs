//! Calculated fields and items and PivotCharts driven in GPUI's headless
//! window: the dialogs are opened, filled and confirmed, and the workbook
//! is checked. What these tests cannot tell is how things look.

use super::calculated::{CalcDialog, Kind, SolveOrder};
use super::*;
use crate::i18n::{Lang, set_lang};
use gpui::{Entity, Modifiers, Render, TestAppContext, VisualTestContext, point, px};
use opensheet_engine::Rect;
use opensheet_engine::pivot::{Area, PivotDestination, PivotSource};

const SALES: &[[&str; 4]] = &[
    ["Region", "Product", "Sales", "Units"],
    ["East", "Apples", "100", "10"],
    ["East", "Pears", "150", "5"],
    ["West", "Apples", "200", "20"],
    ["West", "Pears", "120", "9"],
];

/// Draw the window as it is now (a pointer move far from everything).
pub(super) fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::none());
    cx.run_until_parked();
}

/// A window on the sales with a PivotTable of sales by region (and by
/// product across) on a second sheet, shown, its first label active.
pub(super) fn open(cx: &mut TestAppContext) -> (Entity<ExcelApp>, &mut VisualTestContext, u32) {
    set_lang(Lang::En);
    cx.update(crate::bind_keys);
    let (app, cx) = cx.add_window_view(ExcelApp::new);
    let pivot = app.update_in(cx, |app, window, cx| {
        for (r, row) in SALES.iter().enumerate() {
            for (c, text) in row.iter().enumerate() {
                let _ = app.wb.set_cell_text(0, r as u32, c as u32, text);
            }
        }
        let source = PivotSource::Range {
            sheet: app.wb.sheets[0].name.clone(),
            range: Rect::new(0, 0, 4, 3),
        };
        let id = app.wb.pivot_create(source, PivotDestination::NewSheet { index: 1 }, false).unwrap();
        let locale = app.wb.locale;
        app.wb
            .pivot_update(id, false, |p| {
                p.add_field(0, Area::Rows, None, locale);
                p.add_field(1, Area::Columns, None, locale);
                p.add_field(2, Area::Values, None, locale);
            })
            .unwrap();
        app.commit("setup", cx);
        app.switch_sheet(1, window, cx);
        app.select_cell(4, 0, cx);
        id
    });
    redraw(cx);
    (app, cx, pivot)
}

pub(super) fn dialog<V: Render>(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Entity<V> {
    app.update(cx, |app, _| app.front_modal_view().expect("a dialog is open").downcast::<V>().ok().expect("the dialog"))
}

fn text(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, row: u32, col: u32) -> String {
    app.update(cx, |app, _| app.wb.display_text(1, row, col).text)
}

#[gpui::test]
fn a_calculated_field_is_added_and_modified_in_its_dialog(cx: &mut TestAppContext) {
    let (app, cx, pivot) = open(cx);
    app.update_in(cx, |app, window, cx| calculated::open_field(app, window, cx));
    let view = dialog::<CalcDialog>(&app, cx);
    view.update_in(cx, |d, window, cx| {
        assert_eq!(d.kind, Kind::Field);
        assert_eq!(d.name.read(cx).text(), "Field1");
        assert_eq!(d.fields.iter().map(|f| f.to_string()).collect::<Vec<_>>(), ["Region", "Product", "Sales", "Units"]);
        d.name.update(cx, |input, cx| input.set_text("Price", cx));
        // "Insert Field" writes over the formula still as proposed.
        d.insert("Sales", window, cx);
        assert_eq!(d.formula.read(cx).text(), "=Sales");
        d.formula.update(cx, |input, cx| input.set_text("=Sales/Units", cx));
        assert!(d.add(window, cx));
        assert_eq!(d.existing.len(), 1);
    });
    redraw(cx);
    app.update(cx, |app, _| {
        let (_, p) = app.wb.pivot(pivot).unwrap();
        assert_eq!(p.cache.fields[4].formula.as_deref(), Some("Sales/Units"));
        assert_eq!(p.data_fields.len(), 2);
        assert_eq!(app.wb.undo_label(), Some("Calculated Field"));
        assert!(app.modal_open(), "Add leaves the dialog open");
    });
    // A formula Excel refuses is said, and nothing changes.
    view.update_in(cx, |d, window, cx| {
        d.formula.update(cx, |input, cx| input.set_text("=A1+1", cx));
        assert!(!d.add(window, cx));
    });
    redraw(cx);
    assert_eq!(app.update(cx, |app, _| app.modals.len()), 2);
    app.update_in(cx, |app, window, cx| app.close_modal(window, cx));
    // Modify through OK, which closes.
    view.update(cx, |d, cx| d.formula.update(cx, |input, cx| input.set_text("=Sales*2", cx)));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    app.update(cx, |app, _| {
        assert!(!app.modal_open());
        assert_eq!(app.wb.pivot(pivot).unwrap().1.cache.fields[4].formula.as_deref(), Some("Sales*2"));
    });
    cx.simulate_keystrokes("cmd-z");
    redraw(cx);
    assert_eq!(app.update(cx, |app, _| app.wb.pivot(pivot).unwrap().1.cache.fields[4].formula.clone()).as_deref(), Some("Sales/Units"));
    // The menu of the ribbon button, as Excel lists it.
    app.update_in(cx, |app, window, cx| {
        app.select_cell(4, 0, cx);
        Cmd::CalculatedFields.run(app, window, cx);
    });
    let entries: Vec<(String, bool)> = app.update(cx, |app, _| match &app.popup.as_ref().expect("the menu").kind {
        PopupKind::Menu(entries) => entries.iter().filter(|e| !e.separator).map(|e| (e.label.to_string(), e.disabled)).collect(),
        _ => Vec::new(),
    });
    assert_eq!(
        entries,
        [
            ("Calculated Field...".to_string(), false),
            ("Calculated Item...".to_string(), false),
            ("Solve Order...".to_string(), true),
            ("List Formulas".to_string(), false),
            ("Create Set Based on Row Items...".to_string(), true),
            ("Create Set Based on Column Items...".to_string(), true),
            ("Manage Sets...".to_string(), true),
        ]
    );
}

#[gpui::test]
fn calculated_items_solve_order_and_list_formulas(cx: &mut TestAppContext) {
    let (app, cx, pivot) = open(cx);
    // On a value there is no field to add an item to.
    app.update_in(cx, |app, _, cx| {
        app.select_cell(5, 1, cx);
        assert_eq!(calculated::item_field(app), None);
        app.select_cell(4, 0, cx);
        assert_eq!(calculated::item_field(app), Some(0));
    });
    app.update_in(cx, |app, window, cx| calculated::open_item(app, window, cx));
    let view = dialog::<CalcDialog>(&app, cx);
    view.update_in(cx, |d, window, cx| {
        assert_eq!(d.kind, Kind::Item(0));
        assert_eq!(d.items.iter().map(|i| i.to_string()).collect::<Vec<_>>(), ["East", "West"]);
        d.name.update(cx, |input, cx| input.set_text("Both", cx));
        d.formula.update(cx, |input, cx| input.set_text("=East+West", cx));
        assert!(d.add(window, cx));
        d.name.update(cx, |input, cx| input.set_text("Gap", cx));
        d.formula.update(cx, |input, cx| input.set_text("=West-East", cx));
        assert!(d.add(window, cx));
    });
    app.update_in(cx, |app, window, cx| app.close_modal(window, cx));
    redraw(cx);
    // Rows: East, West, Both, Gap; Apples in column B.
    assert_eq!((text(&app, cx, 6, 0), text(&app, cx, 6, 1)), ("Both".to_string(), "300".to_string()));
    assert_eq!(text(&app, cx, 7, 1), "100");

    app.update_in(cx, |app, window, cx| calculated::open_solve_order(app, window, cx));
    let order = dialog::<SolveOrder>(&app, cx);
    order.update_in(cx, |d, window, cx| {
        assert_eq!(d.lines.iter().map(|l| l.to_string()).collect::<Vec<_>>(), ["Both = East+West", "Gap = West-East"]);
        d.shift(true, window, cx);
        assert_eq!((d.lines[0].to_string(), d.selected), ("Gap = West-East".to_string(), 1));
    });
    app.update_in(cx, |app, window, cx| {
        assert_eq!(app.wb.undo_label(), Some("Solve Order"));
        assert_eq!(app.wb.pivot(pivot).unwrap().1.cache.calc_items[0].name, "Gap");
        app.close_modal(window, cx);
        calculated::list_formulas(app, window, cx);
    });
    redraw(cx);
    app.update(cx, |app, _| {
        assert_eq!(app.wb.sheets.len(), 3);
        assert_eq!(app.sheet, 1, "the list is shown");
        assert_eq!(app.wb.display_text(1, 0, 0).text, "Calculated Field");
        assert_eq!(app.wb.display_text(1, 5, 2).text, "=West-East");
        assert_eq!(app.wb.undo_label(), Some("List Formulas"));
    });
    // An average in the values: Excel refuses calculated items.
    app.update_in(cx, |app, window, cx| {
        app.switch_sheet(2, window, cx);
        app.select_cell(4, 0, cx);
        let locale = app.wb.locale;
        // The items are deleted first, or the average itself is what is wrong.
        app.wb.pivot_delete_calculated_item(pivot, 0, 3, false).unwrap();
        app.wb.pivot_delete_calculated_item(pivot, 0, 2, false).unwrap();
        app.wb.pivot_update(pivot, false, |p| p.set_function(0, opensheet_engine::pivot::Aggregation::Average, locale)).unwrap();
        app.commit("average", cx);
        calculated::open_item(app, window, cx);
        assert!(app.modal_open() && app.front_modal_view().unwrap().downcast::<CalcDialog>().is_err(), "the refusal, not the dialog");
    });
}
