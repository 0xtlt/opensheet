//! Slicers and timelines driven in GPUI's headless window: the dialogs,
//! clicks and drags on the buttons, the keyboard, the two tabs. What
//! these tests cannot tell is how things look.

use super::dialogs::{ConnectionsDialog, InsertDialog, SettingsDialog};
use super::tabs::{FieldKind, field_value, set_field, set_style};
use super::*;
use crate::i18n::{Lang, set_lang};
use gpui::{Entity, Modifiers, Pixels, Point, Render, TestAppContext, VisualTestContext, point, px};
use opensheet_engine::Rect;
use opensheet_engine::pivot::{Area, PivotDestination, PivotSource};
use opensheet_engine::protect::SheetProtection;

const SALES: &[[&str; 4]] = &[
    ["Region", "Product", "Date", "Sales"],
    ["East", "Apples", "2023-01-15", "100"],
    ["East", "Pears", "2023-03-02", "150"],
    ["East", "Apples", "2024-02-10", "120"],
    ["West", "Apples", "2023-03-20", "200"],
    ["West", "Pears", "2024-06-01", "50"],
    ["North", "Plums", "2024-06-30", "70"],
];

/// Draw the window as it is now (a pointer move far from everything).
fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::none());
    cx.run_until_parked();
}

/// A window on the sales, with a PivotTable of sales by region on a second
/// sheet, shown, its first label active. Returns the id of the PivotTable.
fn open(cx: &mut TestAppContext) -> (Entity<ExcelApp>, &mut VisualTestContext, u32) {
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
            range: Rect::new(0, 0, 6, 3),
        };
        let id = app.wb.pivot_create(source, PivotDestination::NewSheet { index: 1 }, false).unwrap();
        let locale = app.wb.locale;
        app.wb
            .pivot_update(id, false, |p| {
                p.add_field(0, Area::Rows, None, locale);
                p.add_field(3, Area::Values, None, locale);
            })
            .unwrap();
        app.commit("setup", cx);
        app.switch_sheet(1, window, cx);
        app.select_cell(3, 0, cx);
        id
    });
    redraw(cx);
    (app, cx, pivot)
}

fn run(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cmd: impl Command) {
    app.update_in(cx, |app, window, cx| app.run_command(cmd, window, cx));
    redraw(cx);
}

/// The dialog in front.
fn dialog<V: Render>(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Entity<V> {
    app.update(cx, |app, _| app.front_modal_view().expect("a dialog is open").downcast::<V>().ok().expect("the dialog"))
}

/// Insert slicers for the fields named, through the dialog; the id of the
/// last one.
fn insert(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, fields: &[&str]) -> u32 {
    run(app, cx, Cmd::InsertSlicer);
    let view = dialog::<InsertDialog>(app, cx);
    view.update(cx, |d, _| {
        for field in d.fields.iter_mut() {
            field.2 = fields.contains(&field.1.as_ref());
        }
    });
    cx.simulate_keystrokes("enter");
    redraw(cx);
    app.update(cx, |app, _| selected_slicer(app).expect("the new slicer is selected").id)
}

/// The middle of the button of the item labelled `label`, in the window.
fn button(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32, label: &str) -> Point<Pixels> {
    app.update(cx, |app, _| {
        let zone = painted_slicer(app, id).expect("the slicer was painted");
        let index = zone.items.iter().position(|i| i.label == label).unwrap_or_else(|| panic!("no item {label}"));
        let (_, b) = zone.layout.buttons.iter().find(|(k, _)| *k == index).expect("the button is in view");
        let layout = app.current_layout().expect("the grid was drawn");
        layout.to_window(zone.origin.0 + (b.0 + b.2) / 2.0 * zone.zoom, zone.origin.1 + (b.1 + b.3) / 2.0 * zone.zoom)
    })
}

/// A point of a box of the slicer's layout, in the window.
fn at_box(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32, pick: impl Fn(&SlicerZone) -> logic::Box4) -> Point<Pixels> {
    app.update(cx, |app, _| {
        let zone = painted_slicer(app, id).expect("the slicer was painted");
        let b = pick(zone);
        let layout = app.current_layout().expect("the grid was drawn");
        layout.to_window(zone.origin.0 + (b.0 + b.2) / 2.0 * zone.zoom, zone.origin.1 + (b.1 + b.3) / 2.0 * zone.zoom)
    })
}

fn click(cx: &mut VisualTestContext, at: Point<Pixels>, modifiers: Modifiers) {
    cx.simulate_click(at, modifiers);
    redraw(cx);
}

/// The labels of the report, from its first label down.
fn labels(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, col: u32, count: u32) -> Vec<String> {
    app.update(cx, |app, _| (3..3 + count).map(|r| app.wb.display_text(1, r, col).text).collect())
}

fn selected(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32) -> Vec<String> {
    app.update(cx, |app, _| app.wb.slicer_items(id).into_iter().filter(|i| i.selected).map(|i| i.label).collect())
}

#[gpui::test]
fn a_slicer_is_inserted_from_its_dialog_and_filters_with_clicks(cx: &mut TestAppContext) {
    let (app, cx, _) = open(cx);
    assert!(app.update(cx, |app, _| Cmd::InsertSlicer.enabled(app) && Cmd::InsertTimeline.enabled(app)));
    run(&app, cx, Cmd::InsertSlicer);
    let view = dialog::<InsertDialog>(&app, cx);
    let names: Vec<String> = view.update(cx, |d, _| d.fields.iter().map(|f| f.1.to_string()).collect());
    assert_eq!(names, ["Region", "Product", "Date", "Sales"]);
    // OK with nothing ticked inserts nothing.
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(app.update(cx, |app, _| !app.modal_open() && app.wb.slicers().next().is_none()));

    let id = insert(&app, cx, &["Region"]);
    app.update(cx, |app, _| {
        assert_eq!(app.context_tab, Some(SLICER_TAB));
        assert_eq!(app.wb.undo_label(), Some("Insert Slicer"));
        assert_eq!(app.wb.sheets[1].slicers.len(), 1);
    });
    assert_eq!(selected(&app, cx, id), ["East", "North", "West"]);

    // A click keeps one item; ⌘-click adds another; one undo step each.
    let west = button(&app, cx, id, "West");
    click(cx, west, Modifiers::none());
    assert_eq!(labels(&app, cx, 0, 2), ["West", "Grand Total"]);
    let east = button(&app, cx, id, "East");
    click(cx, east, Modifiers::command());
    assert_eq!(selected(&app, cx, id), ["East", "West"]);
    cx.simulate_keystrokes("cmd-z");
    redraw(cx);
    assert_eq!(selected(&app, cx, id), ["West"]);

    // "Multi-Select" makes a plain click add to the selection.
    let multi = at_box(&app, cx, id, |z| z.layout.multi.unwrap());
    click(cx, multi, Modifiers::none());
    assert!(app.update(cx, |app, _| Cmd::MultiSelect.checked(app)));
    let north = button(&app, cx, id, "North");
    click(cx, north, Modifiers::none());
    assert_eq!(selected(&app, cx, id), ["North", "West"]);
    click(cx, multi, Modifiers::none());

    // "Clear Filter" shows everything again.
    let clear = at_box(&app, cx, id, |z| z.layout.clear.unwrap());
    click(cx, clear, Modifiers::none());
    assert_eq!(labels(&app, cx, 0, 4), ["East", "North", "West", "Grand Total"]);
    assert!(app.update(cx, |app, _| !app.wb.slicer_filtered(id)));

    // A drag over two buttons selects both, in one step.
    let (from, to) = (button(&app, cx, id, "East"), button(&app, cx, id, "North"));
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    assert!(app.update(cx, |app, _| app.wb.slicer_items(id).iter().all(|i| i.selected)), "applied on release only");
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    redraw(cx);
    assert_eq!(selected(&app, cx, id), ["East", "North"]);
    // Shift-click extends from the item last clicked.
    let north = button(&app, cx, id, "North");
    click(cx, north, Modifiers::none());
    let west = button(&app, cx, id, "West");
    click(cx, west, Modifiers::shift());
    assert_eq!(selected(&app, cx, id), ["North", "West"]);

    // Delete removes the slicer and leaves its filter.
    cx.simulate_keystrokes("delete");
    redraw(cx);
    assert!(app.update(cx, |app, _| app.wb.slicers().next().is_none()));
    assert_eq!(labels(&app, cx, 0, 3), ["North", "West", "Grand Total"]);
    cx.simulate_keystrokes("cmd-z");
    redraw(cx);
    assert!(app.update(cx, |app, _| app.wb.slicer(id).is_some()));
}

#[gpui::test]
fn the_settings_of_a_slicer(cx: &mut TestAppContext) {
    let (app, cx, _) = open(cx);
    let product = insert(&app, cx, &["Product"]);
    app.update_in(cx, |app, _, cx| app.select_cell(3, 0, cx));
    let region = insert(&app, cx, &["Region"]);
    assert_ne!(product, region);
    run(&app, cx, Cmd::SlicerSettings);
    let view = dialog::<SettingsDialog>(&app, cx);
    // A name another slicer has is refused, and the dialog stays.
    view.update(cx, |d, cx| d.name.update(cx, |input, cx| input.set_text("product", cx)));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(app.update(cx, |app, _| app.modals.len()), 2, "the dialog and the message");
    app.update_in(cx, |app, window, cx| app.close_modal(window, cx));
    view.update(cx, |d, cx| {
        d.name.update(cx, |input, cx| input.set_text("Zones", cx));
        d.caption.update(cx, |input, cx| input.set_text("Where", cx));
        d.show_header = false;
        d.descending = true;
        d.hide_no_data = true;
    });
    cx.simulate_keystrokes("enter");
    redraw(cx);
    app.update(cx, |app, _| {
        assert!(!app.modal_open());
        let slicer = app.wb.slicer(region).unwrap().1;
        assert_eq!((slicer.name.as_str(), slicer.caption.as_str()), ("Zones", "Where"));
        assert!(!slicer.show_header && slicer.descending && slicer.hide_no_data);
        assert_eq!(app.wb.undo_label(), Some("Slicer Settings"));
        let order: Vec<String> = app.wb.slicer_items(region).into_iter().map(|i| i.label).collect();
        assert_eq!(order, ["West", "North", "East"]);
        // Without its header the slicer has no header buttons.
        assert!(painted_slicer(app, region).is_some_and(|z| z.layout.clear.is_none()));
    });
    cx.simulate_keystrokes("cmd-z");
    redraw(cx);
    assert_eq!(app.update(cx, |app, _| app.wb.slicer(region).unwrap().1.name.clone()), "Region");
}

#[gpui::test]
fn the_slicer_tab_sets_caption_columns_sizes_and_style(cx: &mut TestAppContext) {
    let (app, cx, _) = open(cx);
    let id = insert(&app, cx, &["Region"]);
    app.update_in(cx, |app, _, cx| {
        assert_eq!(field_value(app, FieldKind::Caption).as_deref(), Some("Region"));
        assert_eq!(field_value(app, FieldKind::Columns).as_deref(), Some("1"));
        assert_eq!(field_value(app, FieldKind::Height).as_deref(), Some("2.63"));
        assert_eq!(field_value(app, FieldKind::Width).as_deref(), Some("2"));
        assert_eq!(field_value(app, FieldKind::ButtonHeight).as_deref(), Some("0.26"));
        set_field(app, FieldKind::Caption, "Zone", cx);
        set_field(app, FieldKind::Columns, "2", cx);
        set_field(app, FieldKind::ButtonHeight, "0.5", cx);
        set_field(app, FieldKind::Height, "3", cx);
        set_field(app, FieldKind::Width, "2.5", cx);
        let slicer = app.wb.slicer(id).unwrap().1;
        assert_eq!((slicer.caption.as_str(), slicer.columns), ("Zone", 2));
        assert_eq!((slicer.button_height, slicer.size), (48.0, (240.0, 288.0)));
        // Two columns of buttons, 1" wide each: the slicer follows.
        set_field(app, FieldKind::ButtonWidth, "1", cx);
        let slicer = app.wb.slicer(id).unwrap().1;
        assert_eq!(slicer.size.0, 2.0 * 96.0 + logic::COLUMN_GAP + 2.0 * logic::PAD);
        assert_eq!(field_value(app, FieldKind::ButtonWidth).as_deref(), Some("1"));
        // Nonsense is not stored.
        set_field(app, FieldKind::Columns, "0", cx);
        set_field(app, FieldKind::Height, "abc", cx);
        assert_eq!(app.wb.slicer(id).unwrap().1.columns, 2);
        set_style(app, "SlicerStyleDark4", cx);
        assert_eq!(app.wb.slicer(id).unwrap().1.style, "SlicerStyleDark4");
        assert_eq!(app.wb.undo_label(), Some("Slicer Style"));
    });
    redraw(cx);
    // The tab draws, with its gallery open too.
    app.update_in(cx, |app, _, cx| {
        app.context_tab = Some(SLICER_TAB);
        cx.notify();
    });
    redraw(cx);
    run(&app, cx, Cmd::Styles);
    assert!(app.update(cx, |app, _| app.popup.is_some()));
}

#[gpui::test]
fn report_connections_and_filter_connections(cx: &mut TestAppContext) {
    let (app, cx, first) = open(cx);
    let second = app.update_in(cx, |app, _, cx| {
        let source = app.wb.pivot(first).unwrap().1.source.clone();
        let id = app.wb.pivot_create(source, PivotDestination::Existing { sheet: 1, row: 2, col: 10 }, false).unwrap();
        let locale = app.wb.locale;
        app.wb
            .pivot_update(id, false, |p| {
                p.add_field(1, Area::Rows, None, locale);
                p.add_field(3, Area::Values, None, locale);
            })
            .unwrap();
        app.commit("second", cx);
        id
    });
    let id = insert(&app, cx, &["Region"]);
    let east = button(&app, cx, id, "East");
    click(cx, east, Modifiers::none());
    assert_eq!(labels(&app, cx, 10, 4), ["Apples", "Pears", "Plums", "Grand Total"]);

    run(&app, cx, Cmd::ReportConnections);
    let view = dialog::<ConnectionsDialog>(&app, cx);
    let listed: Vec<(String, bool)> = view.update(cx, |d, _| d.links.iter().map(|l| (l.name.to_string(), l.on)).collect());
    assert_eq!(listed, [("PivotTable1".to_string(), true), ("PivotTable2".to_string(), false)]);
    view.update(cx, |d, _| d.links[1].on = true);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    app.update(cx, |app, _| {
        assert_eq!(app.wb.slicer(id).unwrap().1.pivots(), [first, second]);
        assert_eq!(app.wb.undo_label(), Some("Report Connections"));
    });
    assert_eq!(labels(&app, cx, 10, 3), ["Apples", "Pears", "Grand Total"]);

    // From the PivotTable: untick the slicer again.
    app.update_in(cx, |app, _, cx| {
        app.deselect_object(cx);
        app.select_cell(3, 10, cx);
        assert!(Cmd::FilterConnections.enabled(app));
    });
    run(&app, cx, Cmd::FilterConnections);
    let view = dialog::<ConnectionsDialog>(&app, cx);
    let listed: Vec<(String, bool)> = view.update(cx, |d, _| d.links.iter().map(|l| (l.name.to_string(), l.on)).collect());
    assert_eq!(listed, [("Region".to_string(), true)]);
    view.update(cx, |d, _| d.links[0].on = false);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    app.update(cx, |app, _| {
        assert_eq!(app.wb.slicer(id).unwrap().1.pivots(), [first]);
        assert_eq!(app.wb.undo_label(), Some("Filter Connections"));
    });
}

#[gpui::test]
fn a_slicer_of_a_table_filters_its_rows(cx: &mut TestAppContext) {
    let (app, cx, _) = open(cx);
    app.update_in(cx, |app, window, cx| {
        app.switch_sheet(0, window, cx);
        app.wb.add_table(0, Rect::new(0, 0, 6, 3), true).unwrap();
        app.commit("table", cx);
        app.select_cell(2, 1, cx);
        assert!(Cmd::InsertSlicer.enabled(app) && !Cmd::InsertTimeline.enabled(app));
    });
    redraw(cx);
    let id = insert(&app, cx, &["Product"]);
    assert!(app.update(cx, |app, _| !Cmd::ReportConnections.enabled(app)));
    let pears = button(&app, cx, id, "Pears");
    click(cx, pears, Modifiers::none());
    app.update(cx, |app, _| {
        assert_eq!(app.wb.sheets[0].filtered_rows.iter().copied().collect::<Vec<_>>(), [1, 3, 4, 6]);
        assert_eq!(app.wb.undo_label(), Some("Slicer Selection"));
    });
    // A protected sheet that does not allow filtering keeps its rows.
    app.update_in(cx, |app, _, cx| {
        app.wb.sheets[0].protection = Some(SheetProtection::default());
        cx.notify();
    });
    redraw(cx);
    let apples = button(&app, cx, id, "Apples");
    click(cx, apples, Modifiers::none());
    app.update_in(cx, |app, window, cx| {
        assert_eq!(app.wb.sheets[0].filtered_rows.len(), 4);
        assert!(app.modal_open(), "the refusal is said");
        app.close_modal(window, cx);
    });
}

/// The middle of period `index` of the bar of a timeline, in the window.
fn period(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32, index: usize) -> Point<Pixels> {
    app.update(cx, |app, _| {
        let zone = painted_timeline(app, id).expect("the timeline was painted");
        let x = zone.layout.period_x(index) + zone.layout.tile / 2.0;
        let y = (zone.layout.bar.0 + zone.layout.bar.1) / 2.0;
        let layout = app.current_layout().expect("the grid was drawn");
        layout.to_window(zone.origin.0 + x * zone.zoom, zone.origin.1 + y * zone.zoom)
    })
}

#[gpui::test]
fn a_timeline_is_inserted_and_selects_periods(cx: &mut TestAppContext) {
    let (app, cx, pivot) = open(cx);
    run(&app, cx, Cmd::InsertTimeline);
    let view = dialog::<InsertDialog>(&app, cx);
    // Only the date field is offered.
    view.update(cx, |d, _| {
        assert_eq!(d.fields.iter().map(|f| f.1.to_string()).collect::<Vec<_>>(), ["Date"]);
        d.fields[0].2 = true;
    });
    cx.simulate_keystrokes("enter");
    redraw(cx);
    let id = app.update(cx, |app, _| {
        assert_eq!(app.context_tab, Some(TIMELINE_TAB));
        assert_eq!(app.wb.undo_label(), Some("Insert Timeline"));
        selected_timeline(app).expect("the new timeline is selected").id
    });

    // March 2023 is the third month of the band.
    let march = period(&app, cx, id, 2);
    click(cx, march, Modifiers::none());
    assert_eq!(labels(&app, cx, 0, 3), ["East", "West", "Grand Total"]);
    app.update(cx, |app, _| {
        let (start, end) = app.wb.timeline_selection(id).unwrap();
        assert_eq!(opensheet_engine::datetime::serial_to_ymd(start), (2023, 3, 1));
        assert_eq!(opensheet_engine::datetime::serial_to_ymd(end), (2023, 3, 31));
        assert_eq!(app.wb.undo_label(), Some("Timeline Selection"));
        assert_eq!(painted_timeline(app, id).unwrap().selected, Some((2, 2)));
    });
    // Dragging the right handle of the span extends it to May.
    let (handle, may) = app.update(cx, |app, _| {
        let zone = painted_timeline(app, id).unwrap();
        let layout = app.current_layout().unwrap();
        let y = zone.origin.1 + (zone.layout.bar.0 + zone.layout.bar.1) / 2.0 * zone.zoom;
        let x = |index: usize, inset: f32| zone.origin.0 + (zone.layout.period_x(index) + inset) * zone.zoom;
        (layout.to_window(x(3, -1.0), y), layout.to_window(x(4, 20.0), y))
    });
    cx.simulate_mouse_down(handle, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(may, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(may, MouseButton::Left, Modifiers::none());
    redraw(cx);
    app.update(cx, |app, _| {
        let (start, end) = app.wb.timeline_selection(id).unwrap();
        assert_eq!(opensheet_engine::datetime::serial_to_ymd(start), (2023, 3, 1));
        assert_eq!(opensheet_engine::datetime::serial_to_ymd(end), (2023, 5, 31));
    });

    // The level: the selection shows on the quarters it touches.
    app.update_in(cx, |app, _, cx| set_level(app, id, TimeLevel::Quarters, cx));
    redraw(cx);
    app.update(cx, |app, _| {
        assert_eq!(app.wb.timeline(id).unwrap().1.level, TimeLevel::Quarters);
        assert_eq!(painted_timeline(app, id).unwrap().selected, Some((0, 1)));
    });
    // "Clear Filter", then the check boxes of the tab.
    let clear = app.update(cx, |app, _| {
        let zone = painted_timeline(app, id).unwrap();
        let b = zone.layout.clear.unwrap();
        app.current_layout().unwrap().to_window(zone.origin.0 + (b.0 + b.2) / 2.0 * zone.zoom, zone.origin.1 + (b.1 + b.3) / 2.0 * zone.zoom)
    });
    click(cx, clear, Modifiers::none());
    assert!(app.update(cx, |app, _| app.wb.timeline_selection(id).is_none()));
    assert_eq!(labels(&app, cx, 0, 4), ["East", "North", "West", "Grand Total"]);
    run(&app, cx, Cmd::ShowHeader);
    run(&app, cx, Cmd::ShowTimeLevel);
    app.update_in(cx, |app, _, cx| {
        let timeline = app.wb.timeline(id).unwrap().1;
        assert!(!timeline.show_header && !timeline.show_time_level && timeline.show_scrollbar);
        assert!(!Cmd::ShowHeader.checked(app) && Cmd::ShowScrollbar.checked(app));
        let zone = painted_timeline(app, id).unwrap();
        assert!(zone.layout.clear.is_none() && zone.layout.level.is_none());
        set_field(app, FieldKind::Caption, "When", cx);
        set_style(app, "TimeSlicerStyleDark1", cx);
        let timeline = app.wb.timeline(id).unwrap().1;
        assert_eq!((timeline.caption.as_str(), timeline.style.as_str()), ("When", "TimeSlicerStyleDark1"));
        assert_eq!(timeline.pivots, [pivot]);
    });
    // The Timeline tab draws.
    app.update_in(cx, |app, _, cx| {
        app.context_tab = Some(TIMELINE_TAB);
        cx.notify();
    });
    redraw(cx);
    // Delete removes it.
    cx.simulate_keystrokes("delete");
    redraw(cx);
    assert!(app.update(cx, |app, _| app.wb.timelines().next().is_none()));
}

#[gpui::test]
fn a_pivot_without_dates_gets_no_timeline_and_objects_copy(cx: &mut TestAppContext) {
    let (app, cx, _) = open(cx);
    // Text in the date column: no field of dates is left.
    app.update_in(cx, |app, _, cx| {
        let _ = app.wb.set_cell_text(0, 1, 2, "soon");
        app.wb.pivot_refresh_all(true).unwrap();
        app.commit("refresh", cx);
    });
    run(&app, cx, Cmd::InsertTimeline);
    app.update_in(cx, |app, window, cx| {
        assert!(app.modal_open() && app.wb.timelines().next().is_none());
        app.close_modal(window, cx);
    });
    let id = insert(&app, cx, &["Region"]);
    cx.simulate_keystrokes("cmd-c cmd-v");
    redraw(cx);
    app.update(cx, |app, _| {
        let names: Vec<String> = app.wb.sheets[1].slicers.iter().map(|s| s.name.clone()).collect();
        assert_eq!(names, ["Region", "Region 1"]);
        assert_ne!(selected_slicer(app).unwrap().id, id);
    });
    // The context menu of a slicer.
    let entries: Vec<String> = app.update(cx, |app, _| object_menu(app, id).into_iter().map(|e| e.label.to_string()).collect());
    assert_eq!(
        entries,
        [
            "Sort Ascending",
            "Sort Descending",
            "Clear Filter From \"Region\"",
            "Multi-Select \"Region\"",
            "Report Connections",
            "Remove \"Region\"",
            "Slicer Settings..."
        ]
    );
}
