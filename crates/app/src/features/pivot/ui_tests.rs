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

#[gpui::test]
fn a_pivot_chart_is_inserted_follows_its_table_and_keeps_to_its_types(cx: &mut TestAppContext) {
    use crate::features::{charts, slicers};
    use opensheet_engine::chart::ChartKind;
    let (app, cx, pivot) = open(cx);
    app.update_in(cx, |app, window, cx| Cmd::PivotChart.run(app, window, cx));
    redraw(cx);
    let chart = app.update(cx, |app, _| {
        let chart = app.selected_chart().expect("the PivotChart is selected");
        assert_eq!(chart.kind, ChartKind::Column);
        assert_eq!(chart.series.len(), 2, "one series per product");
        assert_eq!(app.wb.pivot_of_chart(chart.id).map(|(_, p, _)| p.id), Some(pivot));
        assert_eq!(app.wb.undo_label(), Some("Insert PivotChart"));
        assert_eq!(app.context_tab, Some(chart::TAB));
        // Its tabs: PivotChart Analyze, not those of the PivotTable.
        let tabs: Vec<&str> = Pivots.context_tabs(app).iter().map(|tab| tab.title).collect();
        assert_eq!(tabs, ["PivotChart Analyze"]);
        assert!(Cmd::ChartFieldButtons.checked(app) && Cmd::ChartRefresh.enabled(app));
        // The slicer commands work from the chart.
        assert!(slicers::Cmd::InsertSlicer.enabled(app) && slicers::Cmd::FilterConnections.enabled(app));
        chart.id
    });
    // The buttons Excel shows: axis at the bottom left, legend at the top right, values at the top left.
    app.update(cx, |app, _| {
        let fields = app.wb.pivot(pivot).unwrap().1.chart_fields();
        let buttons = chart::field_buttons(&fields, [true; 4], (480.0, 288.0), &|caption| caption.len() as f32 * 6.0);
        let captions: Vec<&str> = buttons.iter().map(|b| b.caption.as_str()).collect();
        assert_eq!(captions, ["Sum of Sales", "Product", "Region"]);
        assert_eq!((buttons[0].rect.0, buttons[0].rect.1), (4.0, 4.0));
        assert_eq!(buttons[1].rect.2, 476.0);
        assert_eq!(buttons[2].rect.3, 284.0);
    });
    // A change to the PivotTable is a change to the chart.
    app.update_in(cx, |app, window, cx| {
        update(app, pivot, "filter", window, cx, |p| p.set_item_hidden(0, 0, true));
        let chart = app.wb.chart(1, chart).unwrap();
        assert_eq!(app.wb.chart_data(1, chart).categories, ["West"]);
    });
    // Field Buttons, Clear All and Refresh of the tab.
    app.update_in(cx, |app, window, cx| {
        app.select_object(Some(crate::features::object_id(&charts::Charts, chart)), cx);
        // Field Buttons lists the four kinds, ticked, then Hide All.
        let entries = chart::buttons_menu_entries(app);
        let labels: Vec<&str> = entries.iter().map(|e| e.label.as_ref()).collect();
        assert_eq!(
            labels,
            ["Show Report Filter Field Buttons", "Show Legend Field Buttons", "Show Axis Field Buttons", "Show Value Field Buttons", "", "Hide All"]
        );
        assert!(entries[..4].iter().all(|e| e.checked));
        (entries[1].action.as_ref().unwrap())(app, window, cx);
        assert_eq!(app.wb.pivot_of_chart(chart).unwrap().2.shown, [true, false, true, true]);
        assert_eq!(app.wb.undo_label(), Some("Field Buttons"));
        assert!(!chart::buttons_menu_entries(app)[1].checked);
        (entries[5].action.as_ref().unwrap())(app, window, cx);
        assert!(!Cmd::ChartFieldButtons.checked(app));
        assert!(chart::buttons_menu_entries(app)[5].disabled, "nothing left to hide");
        app.wb.undo();
        app.wb.undo();
        assert!(Cmd::ChartFieldButtons.checked(app));
        Cmd::ChartRefresh.run(app, window, cx);
        Cmd::ChartClear.run(app, window, cx);
        assert!(app.wb.pivot(pivot).unwrap().1.data_fields.is_empty());
        assert!(app.wb.chart(1, chart).unwrap().series.is_empty());
        app.wb.undo();
    });
    redraw(cx);
    // Scatter is refused with Excel's message; a line chart is not.
    app.update_in(cx, |app, window, cx| {
        let mut wanted = app.selected_chart().unwrap().clone();
        wanted.set_kind(ChartKind::Scatter);
        assert!(refuses_chart_type(app, &wanted, cx));
        assert!(app.modal_open());
        app.close_modal(window, cx);
        wanted.set_kind(ChartKind::Line);
        assert!(!refuses_chart_type(app, &wanted, cx));
    });
    // Outside a PivotTable the command first makes one.
    app.update_in(cx, |app, window, cx| {
        app.deselect_object(cx);
        app.switch_sheet(0, window, cx);
        app.select_cell(1, 1, cx);
        Cmd::PivotChart.run(app, window, cx);
        assert!(app.modal_open(), "Create PivotTable");
    });
}

/// Where the field button reading `caption` was painted, in the window.
fn chart_button(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, caption: &str) -> gpui::Point<gpui::Pixels> {
    redraw(cx);
    app.update(cx, |app, _| {
        let pivot = chart::chart_pivot(app).expect("a PivotChart is selected");
        let chart = app.selected_chart().unwrap();
        let link = app.wb.pivot_of_chart(chart.id).unwrap().2;
        let buttons = chart::field_buttons(&pivot.chart_fields(), link.shown, chart.size, &|_| 40.0);
        let rank = buttons.iter().position(|b| b.caption == caption).unwrap_or_else(|| panic!("no button {caption}"));
        let zone = app.zones.iter().filter(|zone| chart::is_button_zone(zone)).nth(rank).expect("the button was painted");
        let b = zone.bounds;
        app.current_layout().unwrap().to_window((b.0 + b.2) / 2.0, (b.1 + b.3) / 2.0)
    })
}

#[gpui::test]
fn the_field_buttons_of_a_pivot_chart_open_their_lists(cx: &mut TestAppContext) {
    use crate::app::PopupKind;
    use opensheet_engine::pivot::SortOrder;
    let (app, cx, pivot) = open(cx);
    app.update_in(cx, |app, window, cx| Cmd::PivotChart.run(app, window, cx));
    let chart = app.update(cx, |app, _| app.selected_chart().unwrap().id);
    let drop_down = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| -> Entity<filter::DropDown> {
        app.update(cx, |app, _| match app.popup.as_ref().map(|popup| &popup.kind) {
            Some(PopupKind::View(view)) => view.clone().downcast::<filter::DropDown>().expect("the field's drop-down"),
            _ => panic!("no drop-down is open"),
        })
    };
    // The axis button: the drop-down of the row field, acting on the table.
    let region = chart_button(&app, cx, "Region");
    cx.simulate_click(region, Modifiers::none());
    cx.run_until_parked();
    let list = drop_down(&app, cx);
    assert_eq!(list.read_with(cx, |list, _| list.shown_field()), 0);
    list.update_in(cx, |list, window, cx| list.keep_only(&["West"], window, cx));
    cx.run_until_parked();
    app.update(cx, |app, _| {
        assert!(app.popup.is_none());
        assert_eq!(app.wb.undo_label(), Some("Filter"));
        let c = app.wb.chart(1, chart).unwrap();
        assert_eq!(app.wb.chart_data(1, c).categories, ["West"], "the chart follows");
        assert_eq!(app.wb.display_text(1, 4, 0).text, "West", "and so does the table");
        assert!(app.is_object_selected(crate::features::object_id(&crate::features::charts::Charts, chart)), "the chart stays selected");
    });
    app.update_in(cx, |app, _, cx| {
        app.wb.undo();
        cx.notify();
    });
    // The legend button: the column field; sorting from it.
    let product = chart_button(&app, cx, "Product");
    cx.simulate_click(product, Modifiers::none());
    cx.run_until_parked();
    let list = drop_down(&app, cx);
    assert_eq!(list.read_with(cx, |list, _| list.shown_field()), 1);
    list.update_in(cx, |list, window, cx| list.sort_descending(window, cx));
    cx.run_until_parked();
    app.update(cx, |app, _| {
        assert_eq!(app.wb.pivot(pivot).unwrap().1.fields[1].sort.order, SortOrder::Descending);
        let c = app.wb.chart(1, chart).unwrap();
        let names: Vec<String> = app.wb.chart_data(1, c).series.iter().map(|s| s.name.clone()).collect();
        assert_eq!(names, ["Pears", "Apples"]);
    });
    // A second press on the button that is open closes its list.
    let product = chart_button(&app, cx, "Product");
    cx.simulate_click(product, Modifiers::none());
    cx.run_until_parked();
    assert!(app.update(cx, |app, _| app.popup.is_some()));
    cx.simulate_click(product, Modifiers::none());
    cx.run_until_parked();
    assert!(app.update(cx, |app, _| app.popup.is_none()), "the press that closed the list does not open it again");
    // The value field button: the menu of the value field.
    let values = chart_button(&app, cx, "Sum of Sales");
    cx.simulate_click(values, Modifiers::none());
    cx.run_until_parked();
    app.update_in(cx, |app, window, cx| {
        let Some(PopupKind::Menu(entries)) = app.popup.as_ref().map(|popup| &popup.kind) else {
            panic!("the menu of the value field");
        };
        let labels: Vec<String> = entries.iter().filter(|e| !e.separator).map(|e| e.label.to_string()).collect();
        assert!(labels.contains(&"Value Field Settings...".to_string()) && labels.contains(&"Move Up".to_string()), "{labels:?}");
        let remove = entries.iter().find(|e| e.label.as_ref() == "Remove Field").unwrap().action.clone().unwrap();
        app.popup = None;
        remove(app, window, cx);
        assert!(app.wb.pivot(pivot).unwrap().1.data_fields.is_empty());
        assert!(app.wb.chart(1, chart).unwrap().series.is_empty());
        app.wb.undo();
        assert_eq!(app.wb.chart(1, chart).unwrap().series.len(), 2);
    });
    // A protected sheet refuses the lists.
    app.update_in(cx, |app, _, cx| {
        app.wb.protect_sheet(1, Default::default(), "");
        app.commit("protect", cx);
    });
    let region = chart_button(&app, cx, "Region");
    cx.simulate_click(region, Modifiers::none());
    cx.run_until_parked();
    app.update(cx, |app, _| assert!(!matches!(app.popup.as_ref().map(|p| &p.kind), Some(PopupKind::View(_)))));
}
