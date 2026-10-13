//! The recorder proved by replay: each test records real commands of the
//! window, stops, undoes everything the commands did, runs the script the
//! recorder wrote through the real script engine, and compares the
//! workbook with the one the commands gave.

use super::super::pane::{CodeEditorPane, Page};
use super::super::run::{OutputKind, Runs};
use super::super::{Cmd, pane as the_pane};
use super::differences;
use crate::app::ExcelApp;
use opensheet_engine::ops::PasteMode;
use gpui::{Context, Entity, Modifiers, TestAppContext, VisualTestContext, Window, point, px};
use opensheet_engine::refs::parse_cell_name;
use opensheet_engine::{Rect, Workbook};

pub(super) fn open(cx: &mut TestAppContext) -> (Entity<ExcelApp>, &mut VisualTestContext) {
    cx.update(crate::bind_keys);
    let (app, cx) = cx.add_window_view(ExcelApp::new);
    settle(cx);
    (app, cx)
}

fn settle(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::none());
    cx.run_until_parked();
}

fn rect(name: &str) -> Rect {
    Rect::parse(name).unwrap_or_else(|| panic!("{name} is not a range"))
}

/// One action of the user: a command of the window, after which the
/// window says it changed.
pub(super) fn act(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, command: impl FnOnce(&mut ExcelApp, &mut Window, &mut Context<ExcelApp>)) {
    app.update_in(cx, command);
    cx.run_until_parked();
}

/// Select a range, as a click and a drag do.
pub(super) fn select(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, range: &str) {
    let range = rect(range);
    act(app, cx, |app, _, cx| app.select_rect(range, cx));
}

/// Type something in a cell and press Return.
pub(super) fn enter(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cell: &str, text: &str) {
    let (row, col) = parse_cell_name(cell).expect("a cell");
    act(app, cx, |app, window, cx| {
        app.select_cell(row, col, cx);
        window.focus(&app.grid_focus);
    });
    cx.simulate_input(text);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
}

/// Fill a block with numbers and a header row, without recording: what a
/// test starts from.
pub(super) fn seed(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, rows: &[&[&str]]) {
    act(app, cx, |app, _, cx| {
        for (row, cells) in rows.iter().enumerate() {
            for (col, text) in cells.iter().enumerate() {
                app.wb.set_cell_text(app.sheet, row as u32, col as u32, text).expect("a value");
            }
        }
        app.commit("seed", cx);
    });
}

fn pane(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Entity<CodeEditorPane> {
    app.update(cx, |app, _| the_pane(app).expect("the Code Editor pane"))
}

pub(super) fn start(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) {
    act(app, cx, |app, window, cx| app.run_command(Cmd::RecordActions, window, cx));
    assert_eq!(pane(app, cx).update(cx, |pane, _| pane.page.clone()), Page::Recording);
}

/// What the pane lists: each action with whether it was recorded.
pub(super) fn listed(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Vec<(String, bool)> {
    pane(app, cx).update(cx, |pane, _| pane.recorder.as_ref().expect("recording").actions.iter().map(|action| (action.label.clone(), action.recorded)).collect())
}

/// Stop recording; the script the recorder wrote, as the editor shows it.
pub(super) fn stop(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> String {
    let pane = pane(app, cx);
    cx.update(|window, cx| pane.update(cx, |pane, cx| pane.stop_recording(window, cx)));
    cx.run_until_parked();
    assert_eq!(pane.update(cx, |pane, _| pane.page.clone()), Page::Editor);
    pane.update(cx, |pane, cx| pane.editor.read(cx).text().to_string())
}

/// Record what `actions` does, then prove the script: everything is
/// undone, the script runs, and the workbook must be the one the actions
/// gave. Every action must have been recorded. Returns the script.
pub(super) fn replayed(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, actions: impl FnOnce(&Entity<ExcelApp>, &mut VisualTestContext)) -> String {
    let depth = app.update(cx, |app, _| app.wb.history_depth().0);
    start(app, cx);
    actions(app, cx);
    let actions = listed(app, cx);
    let script = stop(app, cx);
    assert!(actions.iter().all(|(_, recorded)| *recorded), "not recorded: {actions:?}\n{script}");
    assert!(!actions.is_empty(), "nothing was recorded");
    let wanted: Workbook = app.update(cx, |app, _| app.wb.fork());
    // Back to where the recording started, on the sheet it started on.
    act(app, cx, |app, window, cx| {
        while app.wb.history_depth().0 > depth {
            app.undo(cx);
        }
        app.switch_sheet(0, window, cx);
        app.select_cell(0, 0, cx);
    });
    let pane = pane(app, cx);
    cx.update(|window, cx| pane.update(cx, |pane, cx| pane.run(window, cx)));
    cx.run_until_parked();
    let errors: Vec<String> = app.update(cx, |app, _| {
        app.ext::<Runs>().map(|runs| runs.output.iter().filter(|line| line.kind == OutputKind::Error).map(|line| line.text.clone()).collect()).unwrap_or_default()
    });
    assert!(errors.is_empty(), "{errors:?}\n{script}");
    let left = app.update(cx, |app, _| differences(&app.wb, &wanted));
    assert!(left.is_empty(), "the replay differs: {left:#?}\n{script}");
    script
}

/// The statements of a script, without the comments and what every
/// script starts with.
pub(super) fn statements(script: &str) -> Vec<&str> {
    script.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with("//") && !line.starts_with("function") && !line.starts_with("let selectedSheet") && *line != "}").collect()
}

const SALES: &[&[&str]] = &[&["Region", "Units", "Price"], &["North", "12", "3.5"], &["South", "7", "4"], &["East", "30", "2.25"], &["West", "18", "5"]];

// ----------------------------------------------------------------- typing

#[gpui::test]
fn typing_values_and_formulas(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let script = replayed(&app, cx, |app, cx| {
        enter(app, cx, "A1", "Total");
        enter(app, cx, "B1", "12.5");
        enter(app, cx, "B2", "30");
        enter(app, cx, "B3", "=SUM(B1:B2)*2");
        enter(app, cx, "C1", "TRUE");
        enter(app, cx, "C2", "'007");
        enter(app, cx, "C3", "He said \"hi\"");
        enter(app, cx, "D1", "25%");
        enter(app, cx, "D2", "3/14/2024");
    });
    assert_eq!(
        statements(&script)[..4],
        [
            "selectedSheet.getRange(\"A1\").setValue(\"Total\");",
            "selectedSheet.getRange(\"B1\").setValue(12.5);",
            "selectedSheet.getRange(\"B2\").setValue(30);",
            "selectedSheet.getRange(\"B3\").setFormula(\"=SUM(B1:B2)*2\");",
        ]
    );
    assert!(script.contains("selectedSheet.getRange(\"C2\").setValue(\"'007\");"), "{script}");
    assert!(script.starts_with("function main(workbook: ExcelScript.Workbook) {\n\tlet selectedSheet = workbook.getActiveWorksheet();\n\t// Set range A1 on selectedSheet\n"), "{script}");
}

#[gpui::test]
fn clearing_contents_formats_and_all(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    seed(&app, cx, SALES);
    act(&app, cx, |app, _, cx| {
        app.select_rect(rect("A1:C1"), cx);
        app.toggle_bold(cx);
        app.set_fill(Some(opensheet_engine::Color(0xFFFF00)), cx);
    });
    let script = replayed(&app, cx, |app, cx| {
        select(app, cx, "B2:B3");
        act(app, cx, |app, _, cx| app.clear(opensheet_engine::workbook::ClearMode::Contents, cx));
        select(app, cx, "A1:B1");
        act(app, cx, |app, _, cx| app.clear(opensheet_engine::workbook::ClearMode::Formats, cx));
        select(app, cx, "C1:C2");
        act(app, cx, |app, _, cx| app.clear(opensheet_engine::workbook::ClearMode::All, cx));
    });
    assert_eq!(
        statements(&script),
        [
            "selectedSheet.getRange(\"B2:B3\").clear(ExcelScript.ClearApplyTo.contents);",
            "selectedSheet.getRange(\"A1:B1\").clear(ExcelScript.ClearApplyTo.formats);",
            "selectedSheet.getRange(\"C1:C2\").clear(ExcelScript.ClearApplyTo.all);",
        ]
    );
}

#[gpui::test]
fn rows_and_columns_inserted_and_deleted(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    seed(&app, cx, SALES);
    let script = replayed(&app, cx, |app, cx| {
        select(app, cx, "A2:A3");
        act(app, cx, |app, _, cx| app.insert_rows(cx));
        select(app, cx, "B1");
        act(app, cx, |app, _, cx| app.insert_cols(cx));
        select(app, cx, "A6");
        act(app, cx, |app, _, cx| app.delete_rows(cx));
        select(app, cx, "D1");
        act(app, cx, |app, _, cx| app.delete_cols(cx));
    });
    assert_eq!(
        statements(&script),
        [
            "selectedSheet.getRange(\"2:3\").insert(ExcelScript.InsertShiftDirection.down);",
            "selectedSheet.getRange(\"B:B\").insert(ExcelScript.InsertShiftDirection.right);",
            "selectedSheet.getRange(\"6:6\").delete(ExcelScript.DeleteShiftDirection.up);",
            "selectedSheet.getRange(\"D:D\").delete(ExcelScript.DeleteShiftDirection.left);",
        ]
    );
}

#[gpui::test]
fn paste_from_a_range_of_the_workbook(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    seed(&app, cx, SALES);
    enter(&app, cx, "D2", "=B2*C2");
    act(&app, cx, |app, _, cx| {
        app.select_rect(rect("A1:C1"), cx);
        app.toggle_bold(cx);
    });
    let script = replayed(&app, cx, |app, cx| {
        select(app, cx, "A1:D2");
        act(app, cx, |app, _, cx| app.copy(false, cx));
        select(app, cx, "F1");
        act(app, cx, |app, _, cx| app.paste(PasteMode::All, cx));
        select(app, cx, "F5");
        act(app, cx, |app, _, cx| app.paste(PasteMode::Values, cx));
        select(app, cx, "F9");
        act(app, cx, |app, _, cx| app.paste(PasteMode::Formats, cx));
        select(app, cx, "F12");
        act(app, cx, |app, _, cx| app.paste(PasteMode::Formulas, cx));
    });
    assert_eq!(
        statements(&script),
        [
            "selectedSheet.getRange(\"F1\").copyFrom(selectedSheet.getRange(\"A1:D2\"), ExcelScript.RangeCopyType.all, false, false);",
            "selectedSheet.getRange(\"F5\").copyFrom(selectedSheet.getRange(\"A1:D2\"), ExcelScript.RangeCopyType.values, false, false);",
            "selectedSheet.getRange(\"F9\").copyFrom(selectedSheet.getRange(\"A1:D2\"), ExcelScript.RangeCopyType.formats, false, false);",
            "selectedSheet.getRange(\"F12\").copyFrom(selectedSheet.getRange(\"A1:D2\"), ExcelScript.RangeCopyType.formulas, false, false);",
        ]
    );
}

// ---------------------------------------------------------------- formats

#[gpui::test]
fn formats_of_the_home_tab(cx: &mut TestAppContext) {
    use opensheet_engine::Color;
    use opensheet_engine::style::{HAlign, VAlign};
    let (app, cx) = open(cx);
    seed(&app, cx, SALES);
    let script = replayed(&app, cx, |app, cx| {
        select(app, cx, "A1:C1");
        act(app, cx, |app, _, cx| app.toggle_bold(cx));
        act(app, cx, |app, _, cx| app.toggle_italic(cx));
        act(app, cx, |app, _, cx| app.toggle_underline(cx));
        act(app, cx, |app, _, cx| app.set_fill(Some(Color(0xFFC000)), cx));
        act(app, cx, |app, _, cx| app.set_font_color(Some(Color(0xC00000)), cx));
        act(app, cx, |app, _, cx| app.set_h_align(HAlign::Center, cx));
        select(app, cx, "A2:A5");
        act(app, cx, |app, _, cx| app.toggle_strike(cx));
        act(app, cx, |app, _, cx| app.set_font_name("Courier New", cx));
        act(app, cx, |app, _, cx| app.set_font_size(14.0, cx));
        act(app, cx, |app, _, cx| app.set_v_align(VAlign::Top, cx));
        act(app, cx, |app, _, cx| app.toggle_wrap(cx));
        act(app, cx, |app, _, cx| app.change_indent(2, cx));
        select(app, cx, "B2:C5");
        act(app, cx, |app, _, cx| app.set_number_format("#,##0.00", cx));
        act(app, cx, |app, _, cx| app.change_decimals(1, cx));
        select(app, cx, "C2:C5");
        act(app, cx, |app, _, cx| app.set_number_format("0.0%", cx));
        // Back to no fill, and bold off again.
        select(app, cx, "B1");
        act(app, cx, |app, _, cx| app.set_fill(None, cx));
        act(app, cx, |app, _, cx| app.toggle_bold(cx));
    });
    for line in [
        "selectedSheet.getRange(\"A1:C1\").getFormat().getFont().setBold(true);",
        "selectedSheet.getRange(\"A1:C1\").getFormat().getFont().setItalic(true);",
        "selectedSheet.getRange(\"A1:C1\").getFormat().getFont().setUnderline(ExcelScript.RangeUnderlineStyle.single);",
        "selectedSheet.getRange(\"A1:C1\").getFormat().getFill().setColor(\"#FFC000\");",
        "selectedSheet.getRange(\"A1:C1\").getFormat().getFont().setColor(\"#C00000\");",
        "selectedSheet.getRange(\"A1:C1\").getFormat().setHorizontalAlignment(ExcelScript.HorizontalAlignment.center);",
        "selectedSheet.getRange(\"A2:A5\").getFormat().getFont().setStrikethrough(true);",
        "selectedSheet.getRange(\"A2:A5\").getFormat().getFont().setName(\"Courier New\");",
        "selectedSheet.getRange(\"A2:A5\").getFormat().getFont().setSize(14);",
        "selectedSheet.getRange(\"A2:A5\").getFormat().setVerticalAlignment(ExcelScript.VerticalAlignment.top);",
        "selectedSheet.getRange(\"A2:A5\").getFormat().setWrapText(true);",
        "selectedSheet.getRange(\"A2:A5\").getFormat().setIndentLevel(2);",
        "selectedSheet.getRange(\"B2:C5\").setNumberFormat(\"#,##0.00\");",
        "selectedSheet.getRange(\"C2:C5\").setNumberFormat(\"0.0%\");",
        "selectedSheet.getRange(\"B1\").getFormat().getFill().clear();",
        "selectedSheet.getRange(\"B1\").getFormat().getFont().setBold(false);",
    ] {
        assert!(statements(&script).contains(&line), "{line}\n{script}");
    }
}

#[gpui::test]
fn borders_and_merged_cells(cx: &mut TestAppContext) {
    use opensheet_engine::ops::BorderPreset;
    let (app, cx) = open(cx);
    seed(&app, cx, SALES);
    let script = replayed(&app, cx, |app, cx| {
        select(app, cx, "A1:C5");
        act(app, cx, |app, _, cx| app.apply_border(BorderPreset::All, cx));
        act(app, cx, |app, _, cx| app.apply_border(BorderPreset::ThickOutside, cx));
        select(app, cx, "A1:C1");
        act(app, cx, |app, _, cx| app.apply_border(BorderPreset::BottomDouble, cx));
        select(app, cx, "E2:F3");
        act(app, cx, |app, _, cx| app.apply_border(BorderPreset::Outside, cx));
        act(app, cx, |app, _, cx| app.apply_border(BorderPreset::None, cx));
        select(app, cx, "E5:G5");
        act(app, cx, |app, _, cx| app.toggle_merge(true, cx));
        select(app, cx, "E7:F8");
        act(app, cx, |app, _, cx| app.toggle_merge(false, cx));
        act(app, cx, |app, _, cx| app.toggle_merge(false, cx));
    });
    // All Borders is written as Excel's recorder writes it: the four
    // edges and the lines inside.
    let all: Vec<&str> = statements(&script).into_iter().take(12).collect();
    assert_eq!(all[0], "selectedSheet.getRange(\"A1:C5\").getFormat().getRangeBorder(ExcelScript.BorderIndex.edgeTop).setStyle(ExcelScript.BorderLineStyle.continuous);");
    assert_eq!(all[1], "selectedSheet.getRange(\"A1:C5\").getFormat().getRangeBorder(ExcelScript.BorderIndex.edgeTop).setWeight(ExcelScript.BorderWeight.thin);");
    assert!(all[10].contains("insideHorizontal"), "{script}");
    assert!(script.contains("selectedSheet.getRange(\"E5:G5\").merge(false);"), "{script}");
    assert!(script.contains("selectedSheet.getRange(\"E7:F8\").unmerge();"), "{script}");
}

#[gpui::test]
fn the_format_cells_dialog(cx: &mut TestAppContext) {
    use opensheet_engine::style::{Border, BorderStyle};
    let (app, cx) = open(cx);
    seed(&app, cx, SALES);
    // What OK of Format Cells does with what its pages hold.
    let script = replayed(&app, cx, |app, cx| {
        select(app, cx, "B2:C5");
        act(app, cx, |app, _, cx| {
            app.apply_style("Format Cells", cx, |style| {
                style.num_fmt = "0.00".into();
                style.font.set_name("Arial");
                style.font.set_size_pt(9.0);
                style.font.underline = opensheet_engine::style::Underline::Double;
                style.rotation = 45;
                style.shrink_to_fit = true;
                style.locked = false;
                style.hidden = true;
                style.borders.left = Border { style: BorderStyle::Dashed, color: Some(opensheet_engine::Color(0x0000FF)) };
                style.borders.bottom = Border { style: BorderStyle::Medium, color: None };
            })
        });
    });
    for part in [
        "setNumberFormat(\"0.00\")",
        "setName(\"Arial\")",
        "setSize(9)",
        "RangeUnderlineStyle.double",
        "setTextOrientation(45)",
        "setShrinkToFit(true)",
        "getProtection().setLocked(false)",
        "getProtection().setFormulaHidden(true)",
        "BorderLineStyle.dash",
        "setColor(\"#0000FF\")",
        "BorderWeight.medium",
    ] {
        assert!(script.contains(part), "{part}\n{script}");
    }
}

// ------------------------------------------------------------------ sizes

/// A point of the window from one of the grid.
fn grid_point(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, at: impl FnOnce(&crate::grid::GridLayout) -> (f32, f32)) -> gpui::Point<gpui::Pixels> {
    app.update(cx, |app, _| {
        let layout = app.current_layout().expect("the grid is drawn");
        let (x, y) = at(&layout);
        layout.to_window(x, y)
    })
}

fn drag(cx: &mut VisualTestContext, from: gpui::Point<gpui::Pixels>, to: gpui::Point<gpui::Pixels>) {
    cx.simulate_mouse_move(from, None, Modifiers::none());
    cx.simulate_mouse_down(from, gpui::MouseButton::Left, Modifiers::none());
    let middle = point((from.x + to.x) / 2.0, (from.y + to.y) / 2.0);
    cx.simulate_mouse_move(middle, Some(gpui::MouseButton::Left), Modifiers::none());
    cx.simulate_mouse_move(to, Some(gpui::MouseButton::Left), Modifiers::none());
    cx.simulate_mouse_up(to, gpui::MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
}

#[gpui::test]
fn column_widths_row_heights_and_autofit(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    seed(&app, cx, &[&["A rather long heading", "x"], &["1", "2"]]);
    let script = replayed(&app, cx, |app, cx| {
        // The right edge of the heading of column B, dragged 40 px.
        let from = grid_point(app, cx, |layout| (layout.col_x(1) + layout.col_w(1) - 1.0, layout.header_h / 2.0));
        drag(cx, from, point(from.x + px(40.), from.y));
        // The bottom edge of the heading of row 2, dragged 12 px.
        let from = grid_point(app, cx, |layout| (layout.header_w / 2.0, layout.row_y(1) + layout.row_h(1) - 1.0));
        drag(cx, from, point(from.x, from.y + px(12.)));
        act(app, cx, |app, window, cx| app.autofit_col(0, window, cx));
        select(app, cx, "D3:E4");
        act(app, cx, |app, _, cx| app.hide_cols(true, cx));
        act(app, cx, |app, _, cx| app.hide_rows(true, cx));
        act(app, cx, |app, _, cx| app.hide_rows(false, cx));
    });
    let lines = statements(&script);
    assert_eq!(lines[0], "selectedSheet.getRange(\"B:B\").getFormat().setColumnWidth(78);", "{script}");
    assert_eq!(lines[1], "selectedSheet.getRange(\"2:2\").getFormat().setRowHeight(24);", "{script}");
    assert_eq!(lines[2], "selectedSheet.getRange(\"A:A\").getFormat().autofitColumns();", "{script}");
    assert_eq!(lines[3], "selectedSheet.getRange(\"D:E\").setColumnHidden(true);", "{script}");
    assert_eq!(lines[4], "selectedSheet.getRange(\"3:4\").setRowHidden(true);", "{script}");
    assert_eq!(lines[5], "selectedSheet.getRange(\"3:4\").setRowHidden(false);", "{script}");
}

// ------------------------------------------------------------------ cells

#[gpui::test]
fn cells_inserted_and_deleted(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    seed(&app, cx, SALES);
    // The window has no Insert Cells command of its own: the shift the
    // engine has is committed as a command would commit it.
    let script = replayed(&app, cx, |app, cx| {
        select(app, cx, "B2:B3");
        act(app, cx, |app, _, cx| {
            app.wb.shift_cells(0, rect("B2:B3"), true, true).expect("a shift");
            app.commit("Insert Cells", cx);
        });
        select(app, cx, "A4:B4");
        act(app, cx, |app, _, cx| {
            app.wb.shift_cells(0, rect("A4:B4"), false, true).expect("a shift");
            app.commit("Insert Cells", cx);
        });
        select(app, cx, "C1");
        act(app, cx, |app, _, cx| {
            app.wb.shift_cells(0, rect("C1"), true, false).expect("a shift");
            app.commit("Delete Cells", cx);
        });
        select(app, cx, "A2");
        act(app, cx, |app, _, cx| {
            app.wb.shift_cells(0, rect("A2"), false, false).expect("a shift");
            app.commit("Delete Cells", cx);
        });
    });
    assert_eq!(
        statements(&script),
        [
            "selectedSheet.getRange(\"B2:B3\").insert(ExcelScript.InsertShiftDirection.down);",
            "selectedSheet.getRange(\"A4:B4\").insert(ExcelScript.InsertShiftDirection.right);",
            "selectedSheet.getRange(\"C1\").delete(ExcelScript.DeleteShiftDirection.up);",
            "selectedSheet.getRange(\"A2\").delete(ExcelScript.DeleteShiftDirection.left);",
        ]
    );
}

#[gpui::test]
fn fill_down_fill_right_and_the_fill_handle(cx: &mut TestAppContext) {
    use opensheet_engine::ops::Direction;
    let (app, cx) = open(cx);
    seed(&app, cx, &[&["1", "10", "=A1+B1"], &["2", "", ""], &["", "", ""]]);
    let script = replayed(&app, cx, |app, cx| {
        // ⌘D over C1:C3, ⌘R over A5:C5 after a value in A5.
        select(app, cx, "C1:C3");
        act(app, cx, |app, _, cx| app.fill(Direction::Down, cx));
        enter(app, cx, "A5", "7");
        select(app, cx, "A5:C5");
        act(app, cx, |app, _, cx| app.fill(Direction::Right, cx));
        // The handle of A1:A2 dragged down to row 6: the series goes on.
        select(app, cx, "A1:A2");
        cx.run_until_parked();
        let from = grid_point(app, cx, |layout| {
            let (_, _, x1, y1) = layout.rect_px(rect("A1:A2"));
            (x1, y1)
        });
        let to = grid_point(app, cx, |layout| {
            let (x0, y0, x1, y1) = layout.rect_px(rect("A6"));
            ((x0 + x1) / 2.0, (y0 + y1) / 2.0)
        });
        drag(cx, from, to);
    });
    assert_eq!(app.update(cx, |app, _| app.wb.value_at(0, 5, 0)), opensheet_engine::Value::Number(6.0));
    assert_eq!(
        statements(&script),
        [
            "selectedSheet.getRange(\"C2:C3\").copyFrom(selectedSheet.getRange(\"C1\"));",
            "selectedSheet.getRange(\"A5\").setValue(7);",
            "selectedSheet.getRange(\"B5:C5\").copyFrom(selectedSheet.getRange(\"A5\"));",
            "selectedSheet.getRange(\"A1:A2\").autoFill(\"A1:A6\", ExcelScript.AutoFillType.fillDefault);",
        ]
    );
}

// ------------------------------------------------------- sort and filter

#[gpui::test]
fn sorting(cx: &mut TestAppContext) {
    use crate::features::filters;
    use opensheet_engine::ops::{SortLevel, SortSpec};
    let (app, cx) = open(cx);
    seed(&app, cx, SALES);
    let script = replayed(&app, cx, |app, cx| {
        select(app, cx, "B3");
        act(app, cx, |app, window, cx| app.run_command(filters::Cmd::SortAscending, window, cx));
        select(app, cx, "A2");
        act(app, cx, |app, window, cx| app.run_command(filters::Cmd::SortDescending, window, cx));
        // Two levels, as the Sort dialog sorts: no one statement is
        // guessed, the cells are written where they went.
        select(app, cx, "A1:C5");
        act(app, cx, |app, _, cx| {
            let spec = SortSpec {
                levels: vec![SortLevel::values(2, true), SortLevel::values(0, false)],
                has_header: true,
                case_sensitive: false,
                left_to_right: false,
            };
            app.wb.sort_range(0, rect("A1:C5"), &spec).expect("a sort");
            app.commit("Sort", cx);
        });
    });
    let lines = statements(&script);
    assert_eq!(lines[0], "selectedSheet.getRange(\"A1:C5\").getSort().apply([{ key: 1, ascending: true }], false, true);", "{script}");
    assert_eq!(lines[1], "selectedSheet.getRange(\"A1:C5\").getSort().apply([{ key: 0, ascending: false }], false, true);", "{script}");
}

#[gpui::test]
fn the_autofilter_and_its_criteria(cx: &mut TestAppContext) {
    use crate::features::filters;
    use opensheet_engine::filter::{Condition, Criteria, CustomFilter, CustomOp, DynamicFilter, FilterScope, Top10, ValueFilter};
    let (app, cx) = open(cx);
    seed(&app, cx, SALES);
    let criteria = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext, column: u32, criteria: Option<Criteria>| {
        act(app, cx, |app, _, cx| {
            assert!(app.wb.set_filter_criteria(0, FilterScope::Sheet, column, criteria));
            app.commit("Filter", cx);
        });
    };
    let script = replayed(&app, cx, |app, cx| {
        select(app, cx, "B2");
        act(app, cx, |app, window, cx| app.run_command(filters::Cmd::Toggle, window, cx));
        criteria(app, cx, 0, Some(Criteria::Values(ValueFilter { values: vec!["North".to_string(), "East".to_string()], blanks: true, dates: Vec::new() })));
        criteria(
            app,
            cx,
            1,
            Some(Criteria::Custom(CustomFilter {
                first: Condition { op: CustomOp::GreaterOrEqual, value: "10".to_string() },
                second: Some(Condition { op: CustomOp::Less, value: "30".to_string() }),
                and: true,
            })),
        );
        criteria(app, cx, 0, None);
        criteria(app, cx, 1, Some(Criteria::Top10(Top10::new(true, false, 2.0))));
        criteria(app, cx, 2, Some(Criteria::Dynamic(DynamicFilter::AboveAverage)));
        act(app, cx, |app, window, cx| app.run_command(filters::Cmd::Clear, window, cx));
        act(app, cx, |app, window, cx| app.run_command(filters::Cmd::Toggle, window, cx));
    });
    assert_eq!(
        statements(&script),
        [
            "selectedSheet.getAutoFilter().apply(\"A1:C5\");",
            "selectedSheet.getAutoFilter().apply(\"A1:C5\", 0, { filterOn: ExcelScript.FilterOn.values, values: [\"North\", \"East\", \"\"] });",
            "selectedSheet.getAutoFilter().apply(\"A1:C5\", 1, { filterOn: ExcelScript.FilterOn.custom, criterion1: \">=10\", criterion2: \"<30\", operator: ExcelScript.FilterOperator.and });",
            "selectedSheet.getAutoFilter().clearColumnCriteria(0);",
            "selectedSheet.getAutoFilter().apply(\"A1:C5\", 1, { filterOn: ExcelScript.FilterOn.topItems, criterion1: \"2\" });",
            "selectedSheet.getAutoFilter().apply(\"A1:C5\", 2, { filterOn: ExcelScript.FilterOn.dynamic, dynamicCriteria: ExcelScript.DynamicFilterCriteria.aboveAverage });",
            "selectedSheet.getAutoFilter().clearColumnCriteria(1);",
            "selectedSheet.getAutoFilter().clearColumnCriteria(2);",
            "selectedSheet.getAutoFilter().remove();",
        ]
    );
}
