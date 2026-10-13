//! Sparklines driven in GPUI's headless window: commands, dialogs, keys,
//! and what is painted in the cells. What these tests cannot tell is how
//! things look.

use super::gallery::{self, Slot};
use super::*;
use crate::app::ExcelApp;
use crate::features::{self, PaintLayer};
use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, point};
use opensheet_engine::protect::SheetProtection;
use opensheet_engine::sparkline::{EmptyCells, SparklineColors, style};
use opensheet_engine::{Color, Workbook};

/// A window on an empty workbook, with the key bindings of the application.
fn open(cx: &mut TestAppContext) -> (Entity<ExcelApp>, &mut VisualTestContext) {
    cx.update(crate::bind_keys);
    let (app, cx) = cx.add_window_view(ExcelApp::new);
    redraw(cx);
    (app, cx)
}

/// Draw the window as it is now (a pointer move far from everything).
fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::none());
}

fn edit(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, f: impl FnOnce(&mut Workbook)) {
    app.update(cx, |app, cx| {
        f(&mut app.wb);
        app.commit("setup", cx);
    });
    redraw(cx);
}

fn run(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cmd: Cmd) {
    app.update_in(cx, |app, window, cx| app.run_command(cmd, window, cx));
    redraw(cx);
}

fn rect(name: &str) -> Rect {
    Rect::parse(name).expect("a range")
}

fn select(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, range: &str) {
    app.update(cx, |app, cx| app.select_rect(rect(range), cx));
    redraw(cx);
}

fn modal_open(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> bool {
    app.update(cx, |app, _| app.modal_open())
}

fn undo_label(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Option<String> {
    app.update(cx, |app, _| app.wb.undo_label().map(str::to_string))
}

/// Three rows of four numbers in A1:D3.
fn numbers(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) {
    edit(app, cx, |wb| {
        for (r, row) in [["1", "3", "2", "5"], ["4", "-2", "", "1"], ["2", "2", "2", "2"]].iter().enumerate() {
            for (c, text) in row.iter().enumerate() {
                wb.set_cell_text(0, r as u32, c as u32, text).unwrap();
            }
        }
    });
}

/// (row, column, data) of every sparkline of the current sheet, sorted.
fn cells(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Vec<(u32, u32, String)> {
    app.update(cx, |app, _| {
        let mut out: Vec<(u32, u32, String)> = app.wb.sheets[app.sheet]
            .sparklines
            .iter()
            .flat_map(|g| g.sparklines.iter().map(|s| (s.row, s.col, s.data.range.name())))
            .collect();
        out.sort();
        out
    })
}

fn group_of(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cell: &str) -> SparklineGroup {
    let r = rect(cell);
    app.update(cx, |app, _| {
        let sheet = &app.wb.sheets[app.sheet];
        let (group, _) = sheet.sparkline_at(r.r0, r.c0).expect("a sparkline");
        sheet.sparklines[group].clone()
    })
}

/// A group of line sparklines in F1:F3 over the rows of A1:D3.
fn with_lines(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) {
    numbers(app, cx);
    edit(app, cx, |wb| {
        wb.add_sparklines(0, SparklineType::Line, (0, rect("A1:D3")), rect("F1:F3")).unwrap();
    });
    select(app, cx, "F2");
}

fn tabs(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Vec<&'static str> {
    app.update(cx, |app, _| features::context_tabs(app).into_iter().map(|(_, tab)| tab.id).collect())
}

#[gpui::test]
fn the_dialog_creates_sparklines(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    numbers(&app, cx);
    select(&app, cx, "F1:F3");
    let dialog = app.update_in(cx, |app, window, cx| dialogs::open_create(app, SparklineType::Column, window, cx)).expect("the dialog opens");
    redraw(cx);
    // The selection is proposed as the location.
    assert_eq!(dialog.update(cx, |d, cx| d.location.read(cx).text().to_string()), "$F$1:$F$3");
    // Nothing in the data range: Excel's message, the dialog stays.
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(modal_open(&app, cx));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(modal_open(&app, cx));
    assert!(cells(&app, cx).is_empty());
    dialog.update(cx, |d, cx| d.data.update(cx, |field, cx| field.set_text("A1:D3", cx)));
    dialog.update_in(cx, |d, window, cx| d.ok(&(), window, cx));
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert_eq!(cells(&app, cx), [(0, 5, "A1:D1".into()), (1, 5, "A2:D2".into()), (2, 5, "A3:D3".into())]);
    assert_eq!(group_of(&app, cx, "F1").kind, SparklineType::Column);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Insert Sparkline"));
    // The Sparkline tab comes with a cell that has one.
    assert_eq!(tabs(&app, cx), ["sparkline"]);
    select(&app, cx, "A1");
    assert!(tabs(&app, cx).is_empty());
    cx.simulate_keystrokes("cmd-z");
    redraw(cx);
    assert!(cells(&app, cx).is_empty());

    // From the ribbon's menu command; a block of data selected is
    // proposed as the data.
    select(&app, cx, "A1:D3");
    run(&app, cx, Cmd::InsertWinLoss);
    assert!(modal_open(&app, cx));
    cx.simulate_keystrokes("escape");
    redraw(cx);
    let dialog = app.update_in(cx, |app, window, cx| dialogs::open_create(app, SparklineType::WinLoss, window, cx)).unwrap();
    redraw(cx);
    assert_eq!(dialog.update(cx, |d, cx| d.data.read(cx).text().to_string()), "A1:D3");
    dialog.update(cx, |d, cx| d.location.update(cx, |field, cx| field.set_text("$A$5:$D$5", cx)));
    dialog.update_in(cx, |d, window, cx| d.ok(&(), window, cx));
    redraw(cx);
    assert_eq!(cells(&app, cx)[0], (4, 0, "A1:A3".into()));
    assert!(group_of(&app, cx, "A5").negative);
}

#[gpui::test]
fn what_the_dialog_refuses(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    numbers(&app, cx);
    let read = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext, data: &str, location: &str| {
        app.update(cx, |app, _| dialogs::read_fields(app, data, location).err())
    };
    assert_eq!(read(&app, cx, "", "$F$1"), Some(dialogs::Refusal::Data));
    assert_eq!(read(&app, cx, "A1:D1", "nowhere"), Some(dialogs::Refusal::Location));
    assert_eq!(read(&app, cx, "A1:D3", "$F$1:$G$3"), Some(dialogs::Refusal::LocationShape));
    assert_eq!(read(&app, cx, "A1:D3", "$F$1:$F$3"), None);
    // Ranges that do not go together are told so by the engine.
    select(&app, cx, "F1:F2");
    let dialog = app.update_in(cx, |app, window, cx| dialogs::open_create(app, SparklineType::Line, window, cx)).unwrap();
    redraw(cx);
    dialog.update(cx, |d, cx| d.data.update(cx, |field, cx| field.set_text("A1:D3", cx)));
    dialog.update_in(cx, |d, window, cx| d.ok(&(), window, cx));
    redraw(cx);
    assert!(modal_open(&app, cx));
    cx.simulate_keystrokes("enter escape");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert!(cells(&app, cx).is_empty());
    // A protected sheet greys the commands out.
    edit(&app, cx, |wb| {
        wb.protect_sheet(0, SheetProtection::default(), "");
    });
    assert!(app.update(cx, |app, _| !Cmd::Menu.enabled(app) && !Cmd::InsertLine.enabled(app)));
}

#[gpui::test]
fn the_tab_changes_the_group(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    with_lines(&app, cx);
    // Type.
    assert!(app.update(cx, |app, _| Cmd::TypeLine.checked(app) && Cmd::Markers.enabled(app)));
    run(&app, cx, Cmd::TypeColumn);
    assert_eq!(group_of(&app, cx, "F1").kind, SparklineType::Column);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Sparkline Type Change"));
    assert!(app.update(cx, |app, _| Cmd::TypeColumn.checked(app) && !Cmd::Markers.enabled(app)));
    run(&app, cx, Cmd::TypeLine);
    // Show.
    for cmd in [Cmd::HighPoint, Cmd::LowPoint, Cmd::NegativePoints, Cmd::FirstPoint, Cmd::LastPoint, Cmd::Markers] {
        assert!(!app.update(cx, |app, _| cmd.checked(app)));
        run(&app, cx, cmd);
        assert!(app.update(cx, |app, _| cmd.checked(app)), "{cmd:?}");
    }
    let g = group_of(&app, cx, "F3");
    assert!(g.high && g.low && g.negative && g.first && g.last && g.markers);
    run(&app, cx, Cmd::HighPoint);
    assert!(!group_of(&app, cx, "F3").high);
    // Style, colours and weight.
    app.update(cx, |app, cx| gallery::apply_style(app, 30, cx));
    assert_eq!(group_of(&app, cx, "F1").colors, style(30));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Sparkline Style Change"));
    app.update(cx, |app, cx| gallery::apply_color(app, Slot::Series, Color(0x123456), cx));
    app.update(cx, |app, cx| gallery::apply_color(app, Slot::Low, Color(0x654321), cx));
    app.update(cx, |app, cx| gallery::apply_weight(app, 2.25, cx));
    let g = group_of(&app, cx, "F1");
    assert_eq!((g.colors.series, g.colors.low, g.line_weight), (Color(0x123456), Color(0x654321), 2.25));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Sparkline Color Change"));
    cx.simulate_keystrokes("cmd-z cmd-z cmd-z cmd-z");
    redraw(cx);
    assert_eq!(group_of(&app, cx, "F1").colors, SparklineColors::default());
    // The drop-downs open on the group.
    for cmd in [Cmd::Styles, Cmd::SparklineColor, Cmd::MarkerColor, Cmd::Axis, Cmd::EditDataMenu, Cmd::ClearMenu] {
        run(&app, cx, cmd);
        assert!(app.update(cx, |app, _| app.popup.is_some()), "{cmd:?}");
        app.update_in(cx, |app, window, cx| app.close_popup(window, cx));
        redraw(cx);
    }
    assert_eq!(gallery::style_name(0), "Sparkline Style Accent 1, Darker 50%");
    assert_eq!(gallery::style_name(35), "Sparkline Style Colorful #6");
}

#[gpui::test]
fn axis_menu_and_its_custom_values(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    with_lines(&app, cx);
    let entries = app.update(cx, |app, _| axis_menu(app));
    let labels: Vec<String> = entries.iter().filter(|e| !e.separator).map(|e| e.label.to_string()).collect();
    assert_eq!(
        labels,
        [
            "Horizontal Axis Options",
            "General Axis Type",
            "Date Axis Type...",
            "Show Axis",
            "Plot Data Right-to-Left",
            "Vertical Axis Minimum Value Options",
            "Automatic for Each Sparkline",
            "Same for All Sparklines",
            "Custom Value…",
            "Vertical Axis Maximum Value Options",
            "Automatic for Each Sparkline",
            "Same for All Sparklines",
            "Custom Value…",
        ]
    );
    // Automatic is ticked at both ends.
    let ticked: Vec<usize> = entries.iter().enumerate().filter(|(_, e)| e.checked).map(|(i, _)| i).collect();
    assert_eq!(ticked, [1, 7, 12]);
    let pick = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext, index: usize| {
        let action = app.update(cx, |app, _| axis_menu(app)[index].action.clone()).expect("an entry that acts");
        app.update_in(cx, |app, window, cx| action(app, window, cx));
        redraw(cx);
    };
    pick(&app, cx, 3);
    pick(&app, cx, 4);
    pick(&app, cx, 8);
    pick(&app, cx, 13);
    let g = group_of(&app, cx, "F1");
    assert!(g.show_axis && g.right_to_left);
    assert_eq!((g.min, g.max), (AxisBound::Group, AxisBound::Group));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Sparkline Axis Change"));
    // Custom Value… asks for a number.
    let dialog = app.update_in(cx, |app, window, cx| dialogs::open_axis_value(app, false, window, cx)).unwrap();
    redraw(cx);
    assert_eq!(dialog.update(cx, |d, cx| d.value.read(cx).text().to_string()), "0");
    dialog.update(cx, |d, cx| d.value.update(cx, |field, cx| field.set_text("abc", cx)));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(modal_open(&app, cx));
    dialog.update(cx, |d, cx| d.value.update(cx, |field, cx| field.set_text("-3.5", cx)));
    dialog.update_in(cx, |d, window, cx| d.ok(&(), window, cx));
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert_eq!(group_of(&app, cx, "F1").min, AxisBound::Custom(-3.5));
    // The maximum is proposed as the largest value plotted.
    let dialog = app.update_in(cx, |app, window, cx| dialogs::open_axis_value(app, true, window, cx)).unwrap();
    redraw(cx);
    assert_eq!(dialog.update(cx, |d, cx| d.value.read(cx).text().to_string()), "5");
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(group_of(&app, cx, "F1").max, AxisBound::Custom(5.0));
    // Custom Weight.
    let dialog = app.update_in(cx, |app, window, cx| dialogs::open_weight(app, window, cx)).unwrap();
    redraw(cx);
    dialog.update(cx, |d, cx| d.value.update(cx, |field, cx| field.set_text("4", cx)));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(group_of(&app, cx, "F1").line_weight, 4.0);
}

#[gpui::test]
fn edit_data_group_ungroup_and_clear(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    with_lines(&app, cx);
    // Edit Group Location & Data opens on what the group has.
    let dialog = app.update_in(cx, |app, window, cx| dialogs::open_edit_group(app, window, cx)).unwrap();
    redraw(cx);
    assert_eq!(dialog.update(cx, |d, cx| (d.data.read(cx).text().to_string(), d.location.read(cx).text().to_string())), ("A1:D3".into(), "$F$1:$F$3".into()));
    dialog.update(cx, |d, cx| {
        d.data.update(cx, |field, cx| field.set_text("A1:D2", cx));
        d.location.update(cx, |field, cx| field.set_text("$F$1:$F$2", cx));
    });
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert!(!modal_open(&app, cx));
    assert_eq!(cells(&app, cx), [(0, 5, "A1:D1".into()), (1, 5, "A2:D2".into())]);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Sparkline Data Change"));
    // Edit Single Sparkline's Data.
    let dialog = app.update_in(cx, |app, window, cx| dialogs::open_edit_single(app, window, cx)).unwrap();
    redraw(cx);
    assert_eq!(dialog.update(cx, |d, cx| d.data.read(cx).text().to_string()), "A2:D2");
    dialog.update(cx, |d, cx| d.data.update(cx, |field, cx| field.set_text("A1:A3", cx)));
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(cells(&app, cx)[1], (1, 5, "A1:A3".into()));
    // Hidden and Empty Cells.
    let dialog = app.update_in(cx, |app, window, cx| dialogs::open_hidden_empty(app, window, cx)).unwrap();
    redraw(cx);
    dialog.update(cx, |d, cx| {
        assert_eq!((d.empty, d.show_hidden), (EmptyCells::Gaps, false));
        d.set(EmptyCells::Connect, true, cx);
    });
    cx.simulate_keystrokes("enter");
    redraw(cx);
    let g = group_of(&app, cx, "F1");
    assert_eq!((g.empty, g.show_hidden), (EmptyCells::Connect, true));
    // Ungroup, group, clear.
    select(&app, cx, "F1:F2");
    run(&app, cx, Cmd::Ungroup);
    assert_eq!(app.update(cx, |app, _| app.wb.sheets[0].sparklines.len()), 2);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Ungroup Sparklines"));
    run(&app, cx, Cmd::Group);
    assert_eq!(app.update(cx, |app, _| app.wb.sheets[0].sparklines.len()), 1);
    select(&app, cx, "F1");
    run(&app, cx, Cmd::ClearSelected);
    assert_eq!(cells(&app, cx).len(), 1);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Clear Sparklines"));
    cx.simulate_keystrokes("cmd-z");
    redraw(cx);
    run(&app, cx, Cmd::ClearGroups);
    assert!(cells(&app, cx).is_empty());
    assert!(tabs(&app, cx).is_empty());
}

#[gpui::test]
fn sparklines_travel_with_their_cells(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    numbers(&app, cx);
    edit(&app, cx, |wb| {
        wb.add_sparklines(0, SparklineType::Line, (0, rect("A1:D1")), rect("F1")).unwrap();
    });
    // Fill Down.
    select(&app, cx, "F1:F3");
    cx.simulate_keystrokes("cmd-d");
    redraw(cx);
    assert_eq!(cells(&app, cx), [(0, 5, "A1:D1".into()), (1, 5, "A2:D2".into()), (2, 5, "A3:D3".into())]);
    cx.simulate_keystrokes("cmd-z");
    redraw(cx);
    assert_eq!(cells(&app, cx).len(), 1);
    // Copy and paste.
    select(&app, cx, "F1");
    cx.simulate_keystrokes("cmd-c");
    select(&app, cx, "F2:F3");
    cx.simulate_keystrokes("cmd-v");
    redraw(cx);
    assert_eq!(cells(&app, cx), [(0, 5, "A1:D1".into()), (1, 5, "A2:D2".into()), (2, 5, "A3:D3".into())]);
    assert_eq!(app.update(cx, |app, _| app.wb.sheets[0].sparklines.len()), 1);
    // One step takes the paste and its sparklines back.
    cx.simulate_keystrokes("cmd-z");
    redraw(cx);
    assert_eq!(cells(&app, cx).len(), 1);
    // Clear All takes the sparkline; Delete (contents) leaves it.
    select(&app, cx, "F1");
    cx.simulate_keystrokes("delete");
    redraw(cx);
    assert_eq!(cells(&app, cx).len(), 1);
    app.update(cx, |app, cx| app.clear(opensheet_engine::workbook::ClearMode::All, cx));
    redraw(cx);
    assert!(cells(&app, cx).is_empty());
}

#[gpui::test]
fn sparklines_are_painted_in_their_cells(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    with_lines(&app, cx);
    let painted = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| {
        app.update_in(cx, |app, window, _| {
            let layout = app.current_layout().expect("the grid was drawn");
            let text_system = window.text_system().clone();
            let everything = (0.0, 0.0, layout.width(), layout.height());
            let mut painter = Painter::new(app, &layout, &text_system, PaintLayer::Under, Rect::new(0, 0, 40, 15), everything, 1.0);
            paint_cells(&mut painter);
            let cell = layout.bounds_px(layout.col_x(5), layout.row_y(0), layout.col_x(6) - 1.0, layout.row_y(1) - 1.0);
            (std::mem::take(&mut painter.items), cell)
        })
    };
    let (items, first) = painted(&app, cx);
    // One clipped picture per cell, each holding the line (row 2 has an
    // empty cell: two runs).
    assert_eq!(items.len(), 3);
    let Draw::Clipped { clip, items: shapes } = &items[0] else {
        panic!("a clipped picture");
    };
    assert_eq!(*clip, first);
    assert_eq!(shapes.len(), 1);
    assert!(matches!(shapes[0], Draw::Path(..)));
    // Columns are rectangles.
    run(&app, cx, Cmd::TypeColumn);
    let (items, _) = painted(&app, cx);
    let Draw::Clipped { items: shapes, .. } = &items[0] else {
        panic!("a clipped picture");
    };
    assert_eq!(shapes.len(), 4);
    assert!(shapes.iter().all(|s| matches!(s, Draw::Quad(_))));
    // Outside the cells in view nothing is drawn.
    let none = app.update_in(cx, |app, window, _| {
        let layout = app.current_layout().unwrap();
        let text_system = window.text_system().clone();
        let mut painter = Painter::new(app, &layout, &text_system, PaintLayer::Under, Rect::new(10, 0, 40, 15), (0.0, 0.0, 900.0, 900.0), 1.0);
        paint_cells(&mut painter);
        painter.items.len()
    });
    assert_eq!(none, 0);
}

#[gpui::test]
fn the_axis_menu_gives_a_date_axis_and_takes_it_back(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    app.update(cx, |app, cx| {
        for (c, day) in ["45000", "45001", "45003", "45010"].iter().enumerate() {
            let _ = app.wb.set_cell_text(0, 6, c as u32, day);
        }
        app.wb.add_sparklines(0, SparklineType::Line, (0, rect("A1:D3")), rect("F1:F3")).unwrap();
        app.commit("setup", cx);
        app.select_rect(rect("F2"), cx);
    });
    let entry = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext, label: &str| {
        app.update(cx, |app, _| {
            let entries = axis_menu(app);
            let found = entries.iter().find(|e| e.label.as_ref() == label).unwrap_or_else(|| panic!("no entry {label}"));
            (found.checked, found.action.clone().unwrap())
        })
    };
    let (general, _) = entry(&app, cx, "General Axis Type");
    let (dated, open_dialog) = entry(&app, cx, "Date Axis Type...");
    assert!(general && !dated);
    // The range dialog: a range that does not fit is refused and the dialog stays.
    app.update_in(cx, |app, window, cx| open_dialog(app, window, cx));
    let dialog: Entity<dialogs::DateRangeDialog> = app.update(cx, |app, _| app.front_modal_view().unwrap().downcast().unwrap());
    dialog.update(cx, |d, cx| d.range.update(cx, |field, cx| field.set_text("A7:C7", cx)));
    dialog.update_in(cx, |d, window, cx| d.ok(&(), window, cx));
    assert!(app.update(cx, |app, _| app.wb.sheets[0].sparklines[0].date_axis.is_none() && app.modal_open()));
    app.update_in(cx, |app, window, cx| app.close_modal(window, cx));
    dialog.update(cx, |d, cx| d.range.update(cx, |field, cx| field.set_text("A7:D7", cx)));
    dialog.update_in(cx, |d, window, cx| d.ok(&(), window, cx));
    app.update(cx, |app, _| {
        assert!(!app.modal_open());
        let group = &app.wb.sheets[0].sparklines[0];
        assert_eq!(group.date_axis.as_ref().map(|d| d.range.name()), Some("A7:D7".to_string()));
        assert_eq!(app.wb.undo_label(), Some("Sparkline Axis Change"));
    });
    let (general, back) = entry(&app, cx, "General Axis Type");
    assert!(!general && entry(&app, cx, "Date Axis Type...").0);
    // The dialog proposes the range the group has.
    let open_again = entry(&app, cx, "Date Axis Type...").1;
    app.update_in(cx, |app, window, cx| open_again(app, window, cx));
    let dialog: Entity<dialogs::DateRangeDialog> = app.update(cx, |app, _| app.front_modal_view().unwrap().downcast().unwrap());
    assert_eq!(dialog.update(cx, |d, cx| d.range.read(cx).text().to_string()), "A7:D7");
    app.update_in(cx, |app, window, cx| app.close_modal(window, cx));
    // General Axis Type: evenly spaced again, one undo step each.
    app.update_in(cx, |app, window, cx| back(app, window, cx));
    assert!(app.update(cx, |app, _| app.wb.sheets[0].sparklines[0].date_axis.is_none()));
    app.update(cx, |app, cx| {
        app.wb.undo();
        cx.notify();
    });
    assert!(app.update(cx, |app, _| app.wb.sheets[0].sparklines[0].date_axis.is_some()));
}
