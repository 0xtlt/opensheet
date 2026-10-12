//! Charts driven in GPUI's headless window: clicks on their elements, the
//! keyboard, titles typed in place, the commands of the Chart Design tab
//! and the dialogs. What these tests cannot tell is how things look.

use super::*;
use crate::app::PopupKind;
use crate::features::chart_format;
use crate::i18n::{Lang, set_lang};
use gpui::{Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, Pixels, Point, TestAppContext, VisualTestContext};
use opensheet_engine::Rect;
use opensheet_engine::chart::{ChartTitle, LegendPos};
use opensheet_engine::chart_layout::AxisId;

/// A window on an empty workbook, with the key bindings of the application.
fn open(cx: &mut TestAppContext) -> (Entity<ExcelApp>, &mut VisualTestContext) {
    set_lang(Lang::En);
    cx.update(crate::bind_keys);
    let (app, cx) = cx.add_window_view(ExcelApp::new);
    redraw(cx);
    (app, cx)
}

/// Draw the window as it is now (a pointer move far from everything).
fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::none());
    cx.run_until_parked();
}

/// Three months of two regions, and a chart of them, selected.
fn with_chart(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, kind: ChartKind) -> u32 {
    let id = app.update_in(cx, |app, window, cx| {
        let rows = [["Month", "North", "South"], ["Jan", "12", "8"], ["Feb", "15", "11"], ["Mar", "9", "14"]];
        for (r, row) in rows.iter().enumerate() {
            for (c, text) in row.iter().enumerate() {
                let _ = app.wb.set_cell_text(0, r as u32, c as u32, text);
            }
        }
        app.commit("setup", cx);
        app.select_rect(Rect::new(0, 0, 3, 2), cx);
        app.insert_chart(kind, window, cx);
        app.selected_chart().expect("a chart was inserted").id
    });
    redraw(cx);
    id
}

fn chart(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32) -> Chart {
    app.update(cx, |app, _| app.wb.chart(app.sheet, id).cloned().expect("the chart"))
}

fn element(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> ChartElement {
    app.update(cx, |app, _| app.chart_element())
}

/// The middle of an element of a chart as it is painted, in the window.
fn middle(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32, element: ChartElement) -> Point<Pixels> {
    app.update(cx, |app, _| {
        let zone = app.painted_chart(id).expect("the chart was painted");
        let b = zone.layout.bounds(element).unwrap_or_else(|| panic!("the chart shows no {element:?}"));
        let layout = app.current_layout().expect("the grid was drawn");
        layout.to_window(zone.origin.0 + (b[0] + b[2]) / 2.0, zone.origin.1 + (b[1] + b[3]) / 2.0)
    })
}

fn click(cx: &mut VisualTestContext, at: Point<Pixels>) {
    cx.simulate_click(at, Modifiers::none());
    redraw(cx);
}

fn click_on(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32, element: ChartElement) {
    let at = middle(app, cx, id, element);
    click(cx, at);
}

fn double_click(cx: &mut VisualTestContext, at: Point<Pixels>) {
    for count in [1, 2] {
        cx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::none(),
            click_count: count,
            first_mouse: false,
        });
        cx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::none(),
            click_count: count,
        });
    }
    redraw(cx);
}

fn run(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cmd: impl Command) {
    app.update_in(cx, |app, window, cx| app.run_command(cmd, window, cx));
    redraw(cx);
}

fn close_dialog(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) {
    app.update_in(cx, |app, window, cx| app.close_modal(window, cx));
    redraw(cx);
}

#[gpui::test]
fn a_click_selects_the_element_under_it(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = with_chart(&app, cx, ChartKind::Column);
    assert_eq!(element(&app, cx), ChartElement::ChartArea);
    click_on(&app, cx, id, ChartElement::Title);
    assert_eq!(element(&app, cx), ChartElement::Title);
    // A bar: its series first, the bar itself on the next click.
    click_on(&app, cx, id, ChartElement::Point(0, 1));
    assert_eq!(element(&app, cx), ChartElement::Series(0));
    click_on(&app, cx, id, ChartElement::Point(0, 1));
    assert_eq!(element(&app, cx), ChartElement::Point(0, 1));
    // Another series: back to selecting series.
    click_on(&app, cx, id, ChartElement::Point(1, 2));
    assert_eq!(element(&app, cx), ChartElement::Series(1));
    click_on(&app, cx, id, ChartElement::LegendEntry(1));
    assert_eq!(element(&app, cx), ChartElement::Legend);
    click_on(&app, cx, id, ChartElement::LegendEntry(1));
    assert_eq!(element(&app, cx), ChartElement::LegendEntry(1));
    click_on(&app, cx, id, ChartElement::Axis(AxisId::Value));
    assert_eq!(element(&app, cx), ChartElement::Axis(AxisId::Value));
    click_on(&app, cx, id, ChartElement::Axis(AxisId::Category));
    assert_eq!(element(&app, cx), ChartElement::Axis(AxisId::Category));
    // The chart stays selected all along, and nothing was changed.
    app.update(cx, |app, _| {
        assert_eq!(app.selected_chart().map(|c| c.id), Some(id));
        assert_eq!(app.wb.undo_label(), Some("Insert Chart"));
    });
    // A corner of the chart is the chart area again.
    let corner = app.update(cx, |app, _| {
        let zone = app.painted_chart(id).unwrap();
        app.current_layout().unwrap().to_window(zone.origin.0 + 12.0, zone.origin.1 + 12.0)
    });
    click(cx, corner);
    assert_eq!(element(&app, cx), ChartElement::ChartArea);
    assert_eq!(app.update(cx, |app, _| app.selected_chart().map(|c| c.id)), Some(id));
}

#[gpui::test]
fn a_click_on_an_unselected_chart_selects_it_and_the_element(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = with_chart(&app, cx, ChartKind::Line);
    app.update(cx, |app, cx| app.deselect_object(cx));
    redraw(cx);
    assert!(app.update(cx, |app, _| app.selected_chart().is_none()));
    click_on(&app, cx, id, ChartElement::Legend);
    assert_eq!(app.update(cx, |app, _| app.selected_chart().map(|c| c.id)), Some(id));
    assert_eq!(element(&app, cx), ChartElement::Legend);
    // Deselecting the chart forgets the element.
    app.update(cx, |app, cx| app.deselect_object(cx));
    assert_eq!(element(&app, cx), ChartElement::ChartArea);
}

#[gpui::test]
fn delete_removes_the_selected_element_then_the_chart(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = with_chart(&app, cx, ChartKind::Column);
    click_on(&app, cx, id, ChartElement::Title);
    cx.simulate_keystrokes("backspace");
    redraw(cx);
    assert_eq!(chart(&app, cx, id).title, ChartTitle::None);
    assert_eq!(element(&app, cx), ChartElement::ChartArea);
    click_on(&app, cx, id, ChartElement::MajorGridlines(AxisId::Value));
    assert_eq!(element(&app, cx), ChartElement::MajorGridlines(AxisId::Value));
    cx.simulate_keystrokes("delete");
    redraw(cx);
    assert!(!chart(&app, cx, id).axes.value.major_gridlines);
    // A series goes with its cells left alone.
    click_on(&app, cx, id, ChartElement::Point(1, 0));
    cx.simulate_keystrokes("delete");
    redraw(cx);
    assert_eq!(chart(&app, cx, id).series.len(), 1);
    // One undo step each.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(chart(&app, cx, id).series.len(), 2);
    redraw(cx);
    // With the chart area selected, Delete removes the chart.
    assert_eq!(element(&app, cx), ChartElement::ChartArea);
    cx.simulate_keystrokes("delete");
    redraw(cx);
    assert!(app.update(cx, |app, _| app.wb.sheets[0].charts.is_empty()));
}

#[gpui::test]
fn arrows_walk_the_elements_and_escape_steps_back(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = with_chart(&app, cx, ChartKind::Column);
    click_on(&app, cx, id, ChartElement::Title);
    let all = app.update(cx, |app, _| {
        let chart = app.selected_chart().unwrap();
        elements::listed(chart, &app.wb.chart_data(app.sheet, chart))
    });
    let at = all.iter().position(|e| *e == ChartElement::Title).unwrap();
    cx.simulate_keystrokes("down");
    assert_eq!(element(&app, cx), all[(at + 1) % all.len()]);
    cx.simulate_keystrokes("up up");
    assert_eq!(element(&app, cx), all[(at + all.len() - 1) % all.len()]);
    // Left and right walk the points of a series.
    click_on(&app, cx, id, ChartElement::Point(0, 0));
    click_on(&app, cx, id, ChartElement::Point(0, 0));
    cx.simulate_keystrokes("right");
    assert_eq!(element(&app, cx), ChartElement::Point(0, 1));
    cx.simulate_keystrokes("left left");
    assert_eq!(element(&app, cx), ChartElement::Point(0, 2));
    // Escape: the chart area, then no chart.
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!(element(&app, cx), ChartElement::ChartArea);
    assert!(app.update(cx, |app, _| app.selected_chart().is_some()));
    cx.simulate_keystrokes("escape");
    assert!(app.update(cx, |app, _| app.selected_chart().is_none()));
    // The chart itself did not move while the arrows walked its elements.
    assert_eq!(app.update(cx, |app, _| app.wb.undo_label().map(str::to_string)).as_deref(), Some("Insert Chart"));
}

#[gpui::test]
fn titles_are_typed_in_place(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = with_chart(&app, cx, ChartKind::Column);
    let editing = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| {
        app.update(cx, |app, _| app.ext_mut::<ChartUi>().edit.as_ref().map(|edit| edit.element))
    };
    click_on(&app, cx, id, ChartElement::Title);
    assert_eq!(editing(&app, cx), None);
    // A second click goes into the text, all of it selected.
    click_on(&app, cx, id, ChartElement::Title);
    assert_eq!(editing(&app, cx), Some(ChartElement::Title));
    cx.simulate_input("Sales by region");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(editing(&app, cx), None);
    assert_eq!(chart(&app, cx, id).title, ChartTitle::Text("Sales by region".into()));
    assert_eq!(app.update(cx, |app, _| app.wb.undo_label().map(str::to_string)).as_deref(), Some("Chart Title"));
    // An axis title, and Escape leaves it as it was.
    app.update(cx, |app, cx| {
        app.edit_chart("setup", cx, |c| c.axes.value.title = Some("Units".into()));
    });
    redraw(cx);
    let axis_title = ChartElement::AxisTitle(AxisId::Value);
    click_on(&app, cx, id, axis_title);
    click_on(&app, cx, id, axis_title);
    assert_eq!(editing(&app, cx), Some(axis_title));
    cx.simulate_input("Pieces");
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!(chart(&app, cx, id).axes.value.title.as_deref(), Some("Units"));
    // The title is still selected: one click goes back into its text.
    assert_eq!(element(&app, cx), axis_title);
    click_on(&app, cx, id, axis_title);
    assert_eq!(editing(&app, cx), Some(axis_title));
    cx.simulate_input("Pieces");
    // A click elsewhere on the chart stores what was typed.
    click_on(&app, cx, id, ChartElement::Legend);
    assert_eq!(chart(&app, cx, id).axes.value.title.as_deref(), Some("Pieces"));
    assert_eq!(element(&app, cx), ChartElement::Legend);
}

#[gpui::test]
fn a_double_click_opens_the_format_pane_of_the_element(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = with_chart(&app, cx, ChartKind::Column);
    let title = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| {
        app.update(cx, |app, _| app.panes.iter().find(|p| p.id == "chart-format").map(|p| p.title.to_string()))
    };
    assert_eq!(title(&app, cx), None);
    let legend = middle(&app, cx, id, ChartElement::Legend);
    double_click(cx, legend);
    assert_eq!(title(&app, cx).as_deref(), Some("Format Legend"));
    // The pane follows the selection.
    click_on(&app, cx, id, ChartElement::Point(0, 0));
    assert_eq!(title(&app, cx).as_deref(), Some("Format Data Series"));
    click_on(&app, cx, id, ChartElement::Axis(AxisId::Value));
    assert_eq!(title(&app, cx).as_deref(), Some("Format Axis"));
    app.update(cx, |app, cx| app.deselect_object(cx));
    redraw(cx);
    assert_eq!(title(&app, cx).as_deref(), Some("Format Chart Area"));
}

#[gpui::test]
fn the_commands_of_the_design_tab(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = with_chart(&app, cx, ChartKind::Column);
    // Add Chart Element: Excel's eleven submenus.
    run(&app, cx, Cmd::AddElement);
    let entries = app.update(cx, |app, _| match app.popup.as_ref().map(|p| &p.kind) {
        Some(PopupKind::Menu(entries)) => entries.clone(),
        _ => panic!("a menu"),
    });
    let labels: Vec<String> = entries.iter().map(|e| e.label.to_string()).collect();
    assert_eq!(
        labels,
        ["Axes", "Axis Titles", "Chart Title", "Data Labels", "Data Table", "Error Bars", "Gridlines", "Legend", "Lines", "Trendline", "Up/Down Bars"]
    );
    assert!(entries[8].disabled && entries[10].disabled && !entries[9].disabled);
    // Legend ▸ Right, ticked afterwards.
    let pick = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext, menu: usize, entry: &str| {
        run(app, cx, Cmd::AddElement);
        let action = app.update(cx, |app, _| match app.popup.as_ref().map(|p| &p.kind) {
            Some(PopupKind::Menu(entries)) => {
                let sub = entries[menu].submenu.as_ref().expect("a submenu");
                sub.iter().find(|e| e.label.as_ref() == entry).unwrap_or_else(|| panic!("no {entry}")).action.clone().expect("an action")
            }
            _ => panic!("a menu"),
        });
        app.update_in(cx, |app, window, cx| {
            app.popup = None;
            action(app, window, cx);
        });
        redraw(cx);
    };
    pick(&app, cx, 7, "Right");
    assert_eq!(chart(&app, cx, id).legend, LegendPos::Right);
    pick(&app, cx, 3, "Outside End");
    assert!(chart(&app, cx, id).data_labels);
    pick(&app, cx, 6, "Primary Major Horizontal");
    assert!(!chart(&app, cx, id).axes.value.major_gridlines);
    pick(&app, cx, 1, "Primary Vertical");
    assert_eq!(chart(&app, cx, id).axes.value.title.as_deref(), Some("Axis Title"));
    pick(&app, cx, 5, "Percentage");
    assert!(chart(&app, cx, id).series.iter().all(|s| s.error_bars.len() == 1));
    // A trendline with two series and none selected asks which one.
    pick(&app, cx, 9, "Linear");
    assert!(app.update(cx, |app, _| app.modal_open()));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(!app.update(cx, |app, _| app.modal_open()));
    assert_eq!(chart(&app, cx, id).series[0].trendlines.len(), 1);
    assert_eq!(element(&app, cx), ChartElement::Trendline(0, 0));

    // The galleries drop a view down; the dialogs open.
    for cmd in [chart_format::Cmd::QuickLayout, chart_format::Cmd::ChangeColors, chart_format::Cmd::Styles] {
        run(&app, cx, cmd);
        assert!(app.update(cx, |app, _| matches!(app.popup.as_ref().map(|p| &p.kind), Some(PopupKind::View(_)))), "{cmd:?}");
        app.update_in(cx, |app, window, cx| app.close_popup(window, cx));
    }
    for cmd in [chart_format::Cmd::SelectData, chart_format::Cmd::MoveChart] {
        run(&app, cx, cmd);
        assert!(app.update(cx, |app, _| app.modal_open()), "{cmd:?}");
        close_dialog(&app, cx);
    }
    // Switch Row/Column: two series of three months become three of two.
    run(&app, cx, Cmd::SwitchRowColumn);
    assert_eq!(chart(&app, cx, id).series.len(), 3);
    // Change Chart Type: the dialog, and Return keeps the type.
    run(&app, cx, Cmd::ChangeType);
    assert!(app.update(cx, |app, _| app.modal_open()));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(!app.update(cx, |app, _| app.modal_open()));
    assert_eq!(chart(&app, cx, id).kind, ChartKind::Column);
}

#[gpui::test]
fn insert_galleries_and_the_insert_dialog(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = with_chart(&app, cx, ChartKind::Column);
    app.update(cx, |app, cx| {
        app.deselect_object(cx);
        app.select_rect(Rect::new(0, 0, 3, 2), cx);
    });
    for page in insert::Page::ALL {
        app.update_in(cx, |app, window, cx| insert::open_gallery(app, "in-chart-col", page, window, cx));
        redraw(cx);
        assert!(app.update(cx, |app, _| matches!(app.popup.as_ref().map(|p| &p.kind), Some(PopupKind::View(_)))), "{page:?}");
        app.update_in(cx, |app, window, cx| app.close_popup(window, cx));
    }
    // Recommended Charts opens Insert Chart; every page of it draws.
    run(&app, cx, Cmd::Recommended);
    assert!(app.update(cx, |app, _| app.modal_open()));
    close_dialog(&app, cx);
    for page in insert::Page::ALL {
        app.update_in(cx, |app, window, cx| insert::open_insert_dialog(app, Some(page), window, cx));
        redraw(cx);
        close_dialog(&app, cx);
    }
    app.update_in(cx, |app, window, cx| insert::open_insert_dialog(app, Some(insert::Page::Pie), window, cx));
    redraw(cx);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    let charts = app.update(cx, |app, _| app.wb.sheets[0].charts.clone());
    assert_eq!(charts.len(), 2);
    assert_eq!((charts[1].kind, charts[1].series.len()), (ChartKind::Pie, 1));
    assert_ne!(charts[1].id, id);
    // The combo page: columns and a line on the secondary axis.
    app.update_in(cx, |app, window, cx| {
        app.deselect_object(cx);
        app.select_rect(Rect::new(0, 0, 3, 2), cx);
        insert::open_insert_dialog(app, Some(insert::Page::Combo), window, cx);
    });
    redraw(cx);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    let combo = app.update(cx, |app, _| app.wb.sheets[0].charts.last().cloned().unwrap());
    assert_eq!(combo.kind, ChartKind::Column);
    assert_eq!(combo.series.iter().map(|s| s.kind).collect::<Vec<_>>(), [None, Some(ChartKind::Line)]);
}

#[gpui::test]
fn a_protected_sheet_keeps_its_charts(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = with_chart(&app, cx, ChartKind::Column);
    app.update(cx, |app, cx| {
        app.wb.protect_sheet(0, Default::default(), "");
        app.commit("protect", cx);
    });
    redraw(cx);
    let before = chart(&app, cx, id);
    // Selecting elements is still possible; changing them is not.
    click_on(&app, cx, id, ChartElement::Title);
    assert_eq!(element(&app, cx), ChartElement::Title);
    cx.simulate_keystrokes("delete");
    redraw(cx);
    assert!(app.update(cx, |app, _| app.modal_open()), "the refusal is said");
    close_dialog(&app, cx);
    click_on(&app, cx, id, ChartElement::Title);
    click_on(&app, cx, id, ChartElement::Title);
    assert!(app.update(cx, |app, _| app.ext_mut::<ChartUi>().edit.is_none()));
    close_dialog(&app, cx);
    run(&app, cx, Cmd::SwitchRowColumn);
    close_dialog(&app, cx);
    assert_eq!(chart(&app, cx, id), before);
}

#[gpui::test]
fn the_floating_buttons_open_their_lists(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = with_chart(&app, cx, ChartKind::Column);
    let button = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext, wanted: FloatButton| {
        app.update(cx, |app, _| {
            let zone = app.zones.iter().find(|z| z.payload::<FloatButton>() == Some(&wanted)).expect("the button is painted");
            let layout = app.current_layout().unwrap();
            layout.to_window((zone.bounds.0 + zone.bounds.2) / 2.0, (zone.bounds.1 + zone.bounds.3) / 2.0)
        })
    };
    for wanted in [FloatButton::Elements, FloatButton::Styles] {
        let at = button(&app, cx, wanted);
        click(cx, at);
        assert!(app.update(cx, |app, _| matches!(app.popup.as_ref().map(|p| &p.kind), Some(PopupKind::View(_)))));
        // The chart stays selected under its list.
        assert_eq!(app.update(cx, |app, _| app.selected_chart().map(|c| c.id)), Some(id));
        app.update_in(cx, |app, window, cx| app.close_popup(window, cx));
        redraw(cx);
    }
    // Without a selected chart there are no buttons.
    app.update(cx, |app, cx| app.deselect_object(cx));
    redraw(cx);
    assert!(app.update(cx, |app, _| app.zones.iter().all(|z| z.payload::<FloatButton>().is_none())));
}

#[gpui::test]
fn charts_follow_the_sheet_in_every_view(cx: &mut TestAppContext) {
    use crate::features::ViewMode;
    let (app, cx) = open(cx);
    let id = with_chart(&app, cx, ChartKind::Column);
    for mode in [ViewMode::PageLayout, ViewMode::PageBreakPreview, ViewMode::Normal] {
        app.update(cx, |app, cx| {
            app.set_view_mode(mode, cx);
            app.select_chart_element(ChartElement::ChartArea, cx);
        });
        redraw(cx);
        // Painted where the sheet puts the object, whatever the view adds
        // between the pages.
        app.update(cx, |app, _| {
            let layout = app.current_layout().unwrap();
            let chart = app.wb.chart(app.sheet, id).unwrap();
            let zone = app.painted_chart(id).expect("the chart is painted");
            let rect = crate::chart_view::chart_rect(&layout, chart);
            assert_eq!(zone.origin, (rect.0, rect.1), "{mode:?}");
        });
        click_on(&app, cx, id, ChartElement::Title);
        assert_eq!(element(&app, cx), ChartElement::Title, "{mode:?}");
    }
}
