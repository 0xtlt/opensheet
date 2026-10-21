//! Form controls driven in GPUI's headless window: controls are drawn
//! with the mouse, worked with clicks, selected, moved and formatted, and
//! the workbook is checked. What these tests cannot tell is how the
//! controls look.

use super::dialogs::{AssignDialog, FormatDialog, TextDialog};
use super::*;
use crate::features::{self, charts};
use crate::i18n::{Lang, set_lang};
use gpui::{Entity, Modifiers, Pixels, Point, TestAppContext, VisualTestContext, point, px};
use opensheet_engine::Value;
use opensheet_engine::ast::{Expr, RefExpr};
use opensheet_engine::parser::parse_formula;

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::none());
    cx.run_until_parked();
}

fn range(text: &str) -> RefExpr {
    match parse_formula(text, opensheet_engine::Locale::en()) {
        Ok(Expr::Ref(r)) => r,
        other => panic!("{text}: {other:?}"),
    }
}

/// A window on a sheet with three colours in H2:H4.
fn open(cx: &mut TestAppContext) -> (Entity<ExcelApp>, &mut VisualTestContext) {
    set_lang(Lang::En);
    cx.update(crate::bind_keys);
    let (app, cx) = cx.add_window_view(ExcelApp::new);
    app.update(cx, |app, cx| {
        for (r, item) in ["Red", "Green", "Blue"].iter().enumerate() {
            let _ = app.wb.set_cell_text(0, r as u32 + 1, 7, item);
        }
        app.commit("setup", cx);
    });
    redraw(cx);
    (app, cx)
}

/// Put a control on the sheet without the mouse.
fn add(
    app: &Entity<ExcelApp>,
    cx: &mut VisualTestContext,
    kind: ControlKind,
    at: (u32, u32),
    edit: impl FnOnce(&mut FormControl),
) -> u32 {
    let id = app.update(cx, |app, cx| {
        let mut control = FormControl::new(kind);
        control.anchor = at;
        edit(&mut control);
        let id = app.wb.add_control(0, control).unwrap();
        app.commit("add", cx);
        id
    });
    redraw(cx);
    id
}

/// A point of a control as it was painted: fractions of its box.
fn at(
    app: &Entity<ExcelApp>,
    cx: &mut VisualTestContext,
    id: u32,
    fx: f32,
    fy: f32,
) -> Point<Pixels> {
    redraw(cx);
    app.update(cx, |app, _| {
        let rect = app
            .zones
            .iter()
            .find_map(|zone| {
                zone.payload::<ControlZone>()
                    .filter(|hit| hit.id == id)
                    .map(|hit| hit.rect)
            })
            .expect("the control was painted");
        app.current_layout().unwrap().to_window(
            rect.0 + (rect.2 - rect.0) * fx,
            rect.1 + (rect.3 - rect.1) * fy,
        )
    })
}

fn click(
    app: &Entity<ExcelApp>,
    cx: &mut VisualTestContext,
    id: u32,
    fx: f32,
    fy: f32,
    modifiers: Modifiers,
) {
    let point = at(app, cx, id, fx, fy);
    cx.simulate_click(point, modifiers);
    cx.run_until_parked();
}

fn cell(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, row: u32, col: u32) -> Value {
    app.update(cx, |app, _| app.wb.value_at(0, row, col))
}

fn shown(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32) -> FormControl {
    app.update(cx, |app, _| {
        app.wb
            .control_state(0, app.wb.sheets[0].control(id).unwrap())
    })
}

fn undo_label(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Option<String> {
    app.update(cx, |app, _| app.wb.undo_label().map(str::to_string))
}

fn object(id: u32) -> features::ObjectId {
    object_id(&Controls, id)
}

#[gpui::test]
fn the_developer_tab_draws_controls_with_the_mouse(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    // The tab is shown when Preferences ▸ View asks for it.
    let tabs = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| -> Vec<&'static str> {
        app.update(cx, |app, _| {
            features::context_tabs(app)
                .into_iter()
                .map(|(_, tab)| tab.id)
                .collect()
        })
    };
    assert!(!tabs(&app, cx).contains(&TAB));
    preferences::settings::update(|settings| settings.developer_tab = true);
    assert!(tabs(&app, cx).contains(&TAB));
    // The choice lasts between launches.
    let kept = preferences::settings::Settings::from_text(&preferences::settings::get().to_text());
    assert!(kept.developer_tab && !preferences::settings::Settings::default().developer_tab);
    app.update(cx, |app, cx| {
        app.context_tab = Some(TAB);
        cx.notify();
    });
    redraw(cx);
    // Check Box, then a drag on the cells.
    app.update_in(cx, |app, window, cx| {
        app.run_command(Cmd::CheckBox, window, cx)
    });
    assert!(app.update(cx, |app, _| Cmd::CheckBox.checked(app)
        && !Cmd::Button.checked(app)));
    redraw(cx);
    let (from, to) = app.update(cx, |app, _| {
        let layout = app.current_layout().unwrap();
        let (x, y) = (layout.col_x(2) + 4.0, layout.row_y(3) + 2.0);
        (
            layout.to_window(x, y),
            layout.to_window(x + 120.0, y + 24.0),
        )
    });
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::none());
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    let id = app.update(cx, |app, _| {
        let control = &app.wb.sheets[0].controls[0];
        assert_eq!(
            (control.kind, control.name.as_str(), control.text.as_str()),
            (ControlKind::CheckBox, "Check Box 1", "Check Box 1")
        );
        assert_eq!((control.anchor, control.offset), ((3, 2), (4.0, 2.0)));
        assert_eq!(control.size, (120.0, 24.0));
        assert!(
            app.is_object_selected(object(control.id)),
            "the new control is selected"
        );
        assert!(!Cmd::CheckBox.checked(app), "the tool is put down");
        assert_eq!(app.wb.undo_label(), Some("Insert Control"));
        control.id
    });
    // A click draws a control of the usual size; Escape puts the tool down.
    app.update_in(cx, |app, window, cx| {
        app.run_command(Cmd::ScrollBar, window, cx)
    });
    redraw(cx);
    let spot = app.update(cx, |app, _| {
        let layout = app.current_layout().unwrap();
        layout.to_window(layout.col_x(5) + 1.0, layout.row_y(1) + 1.0)
    });
    cx.simulate_click(spot, Modifiers::none());
    cx.run_until_parked();
    app.update(cx, |app, _| {
        let bar = &app.wb.sheets[0].controls[1];
        assert_eq!(
            (bar.kind, bar.size),
            (
                ControlKind::ScrollBar,
                ControlKind::ScrollBar.default_size()
            )
        );
    });
    app.update_in(cx, |app, window, cx| {
        app.run_command(Cmd::Label, window, cx);
        assert!(Cmd::Label.checked(app));
        assert!(features::cancel(app, cx));
        assert!(!Cmd::Label.checked(app));
    });
    // The selected control is an object: Delete removes it, undo brings it back.
    app.update(cx, |app, cx| {
        app.select_object(Some(object(id)), cx);
        app.delete_selected_object(cx);
        assert!(app.wb.sheets[0].control(id).is_none());
        app.wb.undo();
        assert!(app.wb.sheets[0].control(id).is_some());
    });
}

#[gpui::test]
fn a_click_works_a_control_and_a_control_click_selects_it(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let check = add(&app, cx, ControlKind::CheckBox, (1, 1), |c| {
        c.link = Some(range("$A$1"))
    });
    // A plain click ticks the box and writes its cell, as one undo step.
    click(&app, cx, check, 0.5, 0.5, Modifiers::none());
    assert_eq!(cell(&app, cx, 0, 0), Value::Bool(true));
    assert_eq!(shown(&app, cx, check).checked, CheckState::Checked);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Check Box"));
    assert!(
        app.update(cx, |app, _| app.object_sel.is_none()),
        "working a control does not select it"
    );
    click(&app, cx, check, 0.5, 0.5, Modifiers::none());
    assert_eq!(cell(&app, cx, 0, 0), Value::Bool(false));
    app.update(cx, |app, cx| {
        app.wb.undo();
        cx.notify();
    });
    assert_eq!(cell(&app, cx, 0, 0), Value::Bool(true));
    // The cell changes: the box follows.
    app.update(cx, |app, cx| {
        let _ = app.wb.set_cell_text(0, 0, 0, "FALSE");
        app.commit("type", cx);
    });
    assert_eq!(shown(&app, cx, check).checked, CheckState::Unchecked);
    // ⌃-click selects; a click on the selected control leaves it alone.
    click(&app, cx, check, 0.5, 0.5, Modifiers::control());
    assert!(app.update(cx, |app, _| app.is_object_selected(object(check))));
    click(&app, cx, check, 0.5, 0.5, Modifiers::none());
    assert_eq!(
        cell(&app, cx, 0, 0),
        Value::Bool(false),
        "a selected control is not worked"
    );
    // Selected, it is dragged like any object.
    let (from, to) = (at(&app, cx, check, 0.5, 0.5), at(&app, cx, check, 0.5, 3.5));
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::none());
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    app.update(cx, |app, _| {
        assert!(
            app.wb.sheets[0].control(check).unwrap().anchor.0 > 1,
            "moved down"
        );
        assert_eq!(app.wb.undo_label(), Some("Move Object"));
    });
    // A click on a cell lets go of it; the next click works it again.
    app.update(cx, |app, cx| app.deselect_object(cx));
    click(&app, cx, check, 0.5, 0.5, Modifiers::none());
    assert_eq!(cell(&app, cx, 0, 0), Value::Bool(true));
    // On a protected sheet the linked cell is locked: the click is refused.
    app.update(cx, |app, cx| {
        app.wb.protect_sheet(0, Default::default(), "");
        app.commit("protect", cx);
    });
    click(&app, cx, check, 0.5, 0.5, Modifiers::none());
    assert_eq!(cell(&app, cx, 0, 0), Value::Bool(true));
    assert!(
        app.update(cx, |app, _| app.modal_open()),
        "the message of the protected sheet"
    );
}

#[gpui::test]
fn option_buttons_lists_spin_buttons_and_scroll_bars(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let first = add(&app, cx, ControlKind::OptionButton, (1, 1), |c| {
        c.link = Some(range("$A$1"))
    });
    let second = add(&app, cx, ControlKind::OptionButton, (2, 1), |_| {});
    click(&app, cx, second, 0.5, 0.5, Modifiers::none());
    assert_eq!(cell(&app, cx, 0, 0), Value::Number(2.0));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Option Button"));
    click(&app, cx, first, 0.5, 0.5, Modifiers::none());
    assert_eq!(cell(&app, cx, 0, 0), Value::Number(1.0));
    assert!(
        shown(&app, cx, first).checked == CheckState::Checked
            && shown(&app, cx, second).checked == CheckState::Unchecked
    );

    // A list box: a click on its second line.
    let list = add(&app, cx, ControlKind::ListBox, (4, 1), |c| {
        c.link = Some(range("$A$2"));
        c.input = Some(range("$H$2:$H$4"));
    });
    let second_line = (1.0 + 16.0 * 1.5) / ControlKind::ListBox.default_size().1;
    click(&app, cx, list, 0.5, second_line, Modifiers::none());
    assert_eq!(cell(&app, cx, 1, 0), Value::Number(2.0));
    assert_eq!(shown(&app, cx, list).selected, 2);
    // Below its lines: nothing.
    click(&app, cx, list, 0.5, 0.95, Modifiers::none());
    assert_eq!(cell(&app, cx, 1, 0), Value::Number(2.0));

    // A combo box opens its list; choosing a line writes its number.
    let combo = add(&app, cx, ControlKind::ComboBox, (10, 1), |c| {
        c.link = Some(range("$A$3"));
        c.input = Some(range("$H$2:$H$4"));
    });
    click(&app, cx, combo, 0.5, 0.5, Modifiers::none());
    app.update_in(cx, |app, window, cx| {
        let Some(PopupKind::Menu(entries)) = app.popup.as_ref().map(|popup| &popup.kind) else {
            panic!("the list of the combo box");
        };
        let labels: Vec<String> = entries
            .iter()
            .map(|entry| entry.label.to_string())
            .collect();
        assert_eq!(labels, ["Red", "Green", "Blue"]);
        let pick = entries[2].action.clone().unwrap();
        app.popup = None;
        pick(app, window, cx);
    });
    assert_eq!(cell(&app, cx, 2, 0), Value::Number(3.0));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Combo Box"));

    // A spin button: the upper arrow adds, the lower one takes away.
    let spin = add(&app, cx, ControlKind::SpinButton, (1, 4), |c| {
        c.link = Some(range("$B$1"));
        c.inc = 5;
        c.max = 12;
    });
    click(&app, cx, spin, 0.5, 0.25, Modifiers::none());
    click(&app, cx, spin, 0.5, 0.25, Modifiers::none());
    click(&app, cx, spin, 0.5, 0.25, Modifiers::none());
    assert_eq!(
        cell(&app, cx, 0, 1),
        Value::Number(12.0),
        "kept under the maximum"
    );
    click(&app, cx, spin, 0.5, 0.75, Modifiers::none());
    assert_eq!(cell(&app, cx, 0, 1), Value::Number(7.0));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Spin Button"));

    // A scroll bar: its arrows step, its track turns a page, its thumb drags.
    let bar = add(&app, cx, ControlKind::ScrollBar, (1, 6), |c| {
        c.link = Some(range("$C$1"))
    });
    click(&app, cx, bar, 0.5, 0.98, Modifiers::none());
    assert_eq!(cell(&app, cx, 0, 2), Value::Number(1.0));
    click(&app, cx, bar, 0.5, 0.8, Modifiers::none());
    assert_eq!(cell(&app, cx, 0, 2), Value::Number(11.0), "a page down");
    click(&app, cx, bar, 0.5, 0.02, Modifiers::none());
    assert_eq!(cell(&app, cx, 0, 2), Value::Number(10.0));
    let (from, middle, to) = (
        at(&app, cx, bar, 0.5, 0.26),
        at(&app, cx, bar, 0.5, 0.5),
        at(&app, cx, bar, 0.5, 0.9),
    );
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_move(middle, Some(MouseButton::Left), Modifiers::none());
    cx.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::none());
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    cx.run_until_parked();
    let Value::Number(dragged) = cell(&app, cx, 0, 2) else {
        panic!("a number");
    };
    assert!(dragged > 60.0, "{dragged}");
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Scroll Bar"));
    // The drag is one undo step.
    app.update(cx, |app, cx| {
        app.wb.undo();
        cx.notify();
    });
    assert_eq!(cell(&app, cx, 0, 2), Value::Number(10.0));
}

#[gpui::test]
fn a_button_assigned_to_a_macro_says_macros_are_not_run(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let button = add(&app, cx, ControlKind::Button, (1, 1), |c| {
        c.macro_name = Some("[0]!FormatReport".to_string())
    });
    click(&app, cx, button, 0.5, 0.5, Modifiers::none());
    app.update_in(cx, |app, window, cx| {
        assert!(app.modal_open(), "the message of the Macro dialog");
        app.close_modal(window, cx);
        assert_eq!(app.wb.undo_label(), Some("add"), "nothing else happened");
    });
    // Without a macro a button does nothing at all.
    let plain = add(&app, cx, ControlKind::Button, (4, 1), |_| {});
    click(&app, cx, plain, 0.5, 0.5, Modifiers::none());
    assert!(!app.update(cx, |app, _| app.modal_open()));
}

#[gpui::test]
fn format_control_assign_macro_and_edit_text(cx: &mut TestAppContext) {
    use opensheet_io::vba::{ModuleKind, build_project};
    let (app, cx) = open(cx);
    let bar = add(&app, cx, ControlKind::ScrollBar, (1, 1), |_| {});
    let open_format =
        |app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32| -> Entity<FormatDialog> {
            app.update_in(cx, |app, window, cx| {
                app.select_object(Some(object(id)), cx);
                assert!(Cmd::FormatControl.enabled(app));
                dialogs::open_format(app, window, cx).expect("the dialog opens")
            })
        };
    // The Control tab of a scroll bar: values and the linked cell.
    let dialog = open_format(&app, cx, bar);
    redraw(cx);
    let set = |field: &Entity<crate::text_input::TextInput>,
               cx: &mut VisualTestContext,
               text: &str| field.update(cx, |field, cx| field.set_text(text, cx));
    let fields = dialog.update(cx, |d, _| (d.numbers.clone(), d.link.clone()));
    for (field, text) in fields.0.iter().zip(["40", "10", "90", "5", "20"]) {
        set(field, cx, text);
    }
    set(&fields.1, cx, "B2");
    dialog.update(cx, |d, cx| {
        d.shaded = true;
        d.positioning = 2;
        d.print = false;
        cx.notify();
    });
    dialog.update_in(cx, |d, window, cx| d.ok(&(), window, cx));
    app.update(cx, |app, _| {
        assert!(!app.modal_open());
        let control = app.wb.sheets[0].control(bar).unwrap();
        assert_eq!(
            (
                control.value,
                control.min,
                control.max,
                control.inc,
                control.page
            ),
            (40, 10, 90, 5, 20)
        );
        assert_eq!(
            control.link.as_ref().map(RefExpr::to_text).as_deref(),
            Some("$B$2")
        );
        assert!(
            control.shaded
                && !control.print
                && !control.move_with_cells
                && !control.size_with_cells
        );
        assert_eq!(
            app.wb.value_at(0, 1, 1),
            Value::Number(40.0),
            "the value goes to the cell"
        );
        assert_eq!(app.wb.undo_label(), Some("Format Control"));
    });
    // A number out of range, or a link that is no reference, keeps the dialog open.
    let dialog = open_format(&app, cx, bar);
    let fields = dialog.update(cx, |d, _| (d.numbers.clone(), d.link.clone()));
    set(&fields.0[2], cx, "40000");
    dialog.update_in(cx, |d, window, cx| d.ok(&(), window, cx));
    assert!(
        app.update(cx, |app, _| app.wb.sheets[0].control(bar).unwrap().max
            == 90
            && app.modal_open())
    );
    app.update_in(cx, |app, window, cx| app.close_modal(window, cx));
    set(&fields.0[2], cx, "90");
    set(&fields.1, cx, "not a cell");
    dialog.update_in(cx, |d, window, cx| d.ok(&(), window, cx));
    assert!(app.update(cx, |app, _| {
        app.wb.sheets[0].control(bar).unwrap().link.is_some()
    }));
    app.update_in(cx, |app, window, cx| {
        app.close_modal(window, cx);
        app.close_modal(window, cx);
    });
    // A check box: Mixed writes #N/A; a combo box: its range and lines.
    let check = add(&app, cx, ControlKind::CheckBox, (8, 1), |c| {
        c.link = Some(range("$A$1"))
    });
    let dialog = open_format(&app, cx, check);
    dialog.update(cx, |d, _| d.checked = CheckState::Mixed);
    dialog.update_in(cx, |d, window, cx| d.ok(&(), window, cx));
    assert_eq!(
        cell(&app, cx, 0, 0),
        Value::Error(opensheet_engine::ErrorKind::NA)
    );
    let combo = add(&app, cx, ControlKind::ComboBox, (10, 1), |_| {});
    let dialog = open_format(&app, cx, combo);
    let (input, lines) = dialog.update(cx, |d, _| (d.input.clone(), d.drop_lines.clone()));
    set(&input, cx, "H2:H4");
    set(&lines, cx, "3");
    dialog.update_in(cx, |d, window, cx| d.ok(&(), window, cx));
    app.update(cx, |app, _| {
        let control = app.wb.sheets[0].control(combo).unwrap();
        assert_eq!(
            control.input.as_ref().map(RefExpr::to_text).as_deref(),
            Some("$H$2:$H$4")
        );
        assert_eq!(control.drop_lines, 3);
    });

    // Assign Macro lists the macros of the workbook and stores the name.
    let button = add(&app, cx, ControlKind::Button, (14, 1), |_| {});
    app.update(cx, |app, _| {
        let project = build_project(&[(
            "Module1",
            ModuleKind::Standard,
            "Sub FormatReport()\nEnd Sub\n\nSub ClearReport()\nEnd Sub\n",
        )]);
        app.wb.vba.project = Some(opensheet_engine::vba::VbaProject {
            bin: project.into(),
            signatures: Vec::new(),
        });
    });
    let dialog: Entity<AssignDialog> = app.update_in(cx, |app, window, cx| {
        app.select_object(Some(object(button)), cx);
        dialogs::open_assign_macro(app, window, cx).unwrap()
    });
    assert_eq!(
        dialog.update(cx, |d, _| d.macros.clone()),
        ["ClearReport", "FormatReport"]
    );
    dialog.update(cx, |d, cx| {
        d.name
            .update(cx, |field, cx| field.set_text("FormatReport", cx))
    });
    dialog.update_in(cx, |d, window, cx| d.ok(&(), window, cx));
    app.update(cx, |app, _| {
        assert_eq!(
            app.wb.sheets[0]
                .control(button)
                .unwrap()
                .macro_name
                .as_deref(),
            Some("[0]!FormatReport")
        );
        assert_eq!(app.wb.undo_label(), Some("Assign Macro"));
    });
    // Edit Text.
    let dialog: Entity<TextDialog> = app.update_in(cx, |app, window, cx| {
        dialogs::open_edit_text(app, window, cx).unwrap()
    });
    dialog.update(cx, |d, cx| {
        d.text
            .update(cx, |field, cx| field.set_text("Format the report", cx))
    });
    dialog.update_in(cx, |d, window, cx| d.ok(&(), window, cx));
    app.update(cx, |app, _| {
        assert_eq!(
            app.wb.sheets[0].control(button).unwrap().text,
            "Format the report"
        );
        // The commands of a control are those of its context menu.
        let mut menu = ContextMenu::default();
        Controls.context_menu(app, &MenuTarget::Object(object(button)), &mut menu);
        let labels: Vec<String> = menu
            .build()
            .iter()
            .filter(|entry| !entry.separator)
            .map(|entry| entry.label.to_string())
            .collect();
        assert_eq!(
            labels,
            ["Edit Text", "Assign Macro...", "Format Control..."]
        );
        // Nothing of this is offered for another object.
        let mut other = ContextMenu::default();
        Controls.context_menu(
            app,
            &MenuTarget::Object(object_id(&charts::Charts, 1)),
            &mut other,
        );
        assert!(other.build().is_empty());
    });
    // A scroll bar has no caption to edit.
    app.update(cx, |app, cx| {
        app.select_object(Some(object(bar)), cx);
        assert!(!Cmd::EditText.enabled(app) && Cmd::AssignMacro.enabled(app));
    });
}
