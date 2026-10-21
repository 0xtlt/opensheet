//! Typing in the shapes of a SmartArt graphic, its Format tab, the
//! members of a group selected one by one and charts held in groups,
//! driven in GPUI's headless window on the real application.

use super::*;
use gpui::{Modifiers, TestAppContext, VisualTestContext, point};
use opensheet_engine::smartart::{Layout, SmartArt};

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

fn undo(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) {
    app.update(cx, |app, cx| app.undo(cx));
    redraw(cx);
}

fn undo_label(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Option<String> {
    app.update(cx, |app, _| app.wb.undo_label().map(str::to_string))
}

fn art(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32) -> SmartArt {
    app.update(cx, |app, _| {
        app.wb
            .drawing(app.sheet, id)
            .and_then(Drawing::as_smartart)
            .expect("a graphic")
            .clone()
    })
}

fn outline(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32) -> String {
    art(app, cx, id)
        .outline()
        .iter()
        .map(|(level, text)| format!("{}{text}", ">".repeat(*level)))
        .collect::<Vec<_>>()
        .join("|")
}

/// A graphic of a layout with an outline, selected, its Text Pane open.
fn graphic(
    app: &Entity<ExcelApp>,
    cx: &mut VisualTestContext,
    layout: Layout,
    lines: &[(usize, &str)],
) -> u32 {
    let id = app
        .update_in(cx, |app, window, cx| {
            smartart::insert(app, layout, window, cx)
        })
        .expect("a graphic");
    app.update(cx, |app, cx| {
        app.wb.update_drawing(app.sheet, id, |drawing| {
            let lines: Vec<_> = lines
                .iter()
                .map(|(level, text)| (*level, text.to_string()))
                .collect();
            drawing.as_smartart_mut().unwrap().set_outline(&lines);
        });
        app.commit("setup", cx);
    });
    redraw(cx);
    id
}

/// Where the shape that shows a line of the selected graphic is in the
/// window: its middle.
fn shape_of_line(
    app: &Entity<ExcelApp>,
    cx: &mut VisualTestContext,
    line: usize,
) -> gpui::Point<Pixels> {
    app.update(cx, |app, _| {
        let layout = app.current_layout().unwrap();
        let pane = *paint::panes(&layout).last().unwrap();
        let drawing = selected(app).unwrap();
        let frame = app.wb.sheets[app.sheet].drawing_frame(drawing);
        let placed = paint::placed(&layout, &app.wb.sheets[app.sheet], drawing, &pane);
        let natural = (frame.width as f32, frame.height as f32);
        let pieces = drawing
            .as_smartart()
            .unwrap()
            .pieces(app.wb.theme(), natural.0, natural.1);
        let piece = pieces
            .iter()
            .find(|piece| piece.node == Some(line))
            .expect("a shape for the line");
        let at = smartart::placed_piece(piece, &placed, natural).centre();
        layout.to_window(at.0, at.1)
    })
}

fn click(cx: &mut VisualTestContext, at: gpui::Point<Pixels>) {
    cx.simulate_click(at, Modifiers::none());
    redraw(cx);
}

fn click_line(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, line: usize) {
    let at = shape_of_line(app, cx, line);
    click(cx, at);
}

// ------------------------------------------------- typing in a SmartArt shape

#[gpui::test]
fn an_entry_is_typed_in_its_shape(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = graphic(
        &app,
        cx,
        Layout::Process,
        &[(0, "Plan"), (0, "Build"), (0, "Ship")],
    );
    let pane = app
        .update(cx, |app, _| smartart::pane(app))
        .expect("the pane");
    // A click on a shape of the selected graphic chooses its entry and
    // types nowhere yet.
    click_line(&app, cx, 1);
    assert_eq!(
        app.update(cx, |app, _| (
            smartart::current_line(app),
            smartart::editing(app)
        )),
        (Some(1), None)
    );
    // A second click types in the shape; the pane follows each key and
    // the graphic is laid out again.
    click_line(&app, cx, 1);
    assert_eq!(
        app.update(cx, |app, _| smartart::editing(app)),
        Some((id, 1))
    );
    let size_before = art(&app, cx, id).pieces(&Default::default(), 576.0, 384.0)[0]
        .shape
        .font
        .size;
    // The caret is where the click was: in the middle of the word.
    cx.simulate_keystrokes("end");
    cx.simulate_input(" and test everything twice");
    redraw(cx);
    assert_eq!(
        outline(&app, cx, id),
        "Plan|Build and test everything twice|Ship"
    );
    assert_eq!(
        pane.read_with(cx, |pane, cx| pane.texts(cx)),
        vec!["Plan", "Build and test everything twice", "Ship"]
    );
    let size_after = art(&app, cx, id).pieces(&Default::default(), 576.0, 384.0)[0]
        .shape
        .font
        .size;
    assert!(
        size_after < size_before,
        "the text of every shape shrinks to let the long one fit"
    );
    // Return: a new shape after this one, typed in at once.
    cx.simulate_keystrokes("enter");
    redraw(cx);
    assert_eq!(
        app.update(cx, |app, _| smartart::editing(app)),
        Some((id, 2))
    );
    cx.simulate_input("Check");
    redraw(cx);
    assert_eq!(
        outline(&app, cx, id),
        "Plan|Build and test everything twice|Check|Ship"
    );
    // Tab makes it a detail of the entry before it, Shift+Tab an entry
    // again; the caret stays in it.
    cx.simulate_keystrokes("tab");
    redraw(cx);
    assert_eq!(
        outline(&app, cx, id),
        "Plan|Build and test everything twice|>Check|Ship"
    );
    assert_eq!(
        app.update(cx, |app, _| smartart::editing(app)),
        Some((id, 2))
    );
    cx.simulate_input("s");
    cx.simulate_keystrokes("shift-tab");
    redraw(cx);
    assert_eq!(
        outline(&app, cx, id),
        "Plan|Build and test everything twice|Checks|Ship"
    );
    // Where the layout cannot follow, the key does nothing: the first
    // entry has none before it to go under.
    cx.simulate_keystrokes("shift-tab");
    redraw(cx);
    assert_eq!(
        outline(&app, cx, id),
        "Plan|Build and test everything twice|Checks|Ship"
    );
    // Escape leaves the text; the shape stays chosen, and a second
    // Escape gives the graphic back as a whole.
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!(
        app.update(cx, |app, _| (
            smartart::current_line(app),
            smartart::editing(app)
        )),
        (Some(2), None)
    );
    assert_eq!(
        pane.read_with(cx, |pane, cx| pane.texts(cx)),
        vec!["Plan", "Build and test everything twice", "Checks", "Ship"]
    );
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!(
        app.update(cx, |app, _| (
            smartart::current_line(app),
            selected(app).map(|d| d.id)
        )),
        (None, Some(id))
    );
    // Each stretch of typing is one undo step, each change of the
    // outline another.
    let mut labels = Vec::new();
    let mut outlines = Vec::new();
    for _ in 0..6 {
        labels.push(undo_label(&app, cx).unwrap_or_default());
        undo(&app, cx);
        outlines.push(outline(&app, cx, id));
    }
    assert_eq!(
        labels,
        [
            "Promote",
            "Typing",
            "Demote",
            "Typing",
            "Add Shape",
            "Typing"
        ]
    );
    assert_eq!(outlines[5], "Plan|Build|Ship");
    assert_eq!(outlines[4], "Plan|Build and test everything twice|Ship");
}

#[gpui::test]
fn keys_go_to_the_chosen_entry_of_a_graphic(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = graphic(
        &app,
        cx,
        Layout::OrgChart,
        &[(0, "Head"), (1, "Sales"), (1, "Works")],
    );
    // With a shape chosen, a key types after what it holds.
    click_line(&app, cx, 2);
    cx.simulate_input("!");
    redraw(cx);
    assert_eq!(
        app.update(cx, |app, _| smartart::editing(app)),
        Some((id, 2))
    );
    assert_eq!(outline(&app, cx, id), "Head|>Sales|>Works!");
    // A click on another shape ends the typing as one undo step and
    // chooses that shape.
    click_line(&app, cx, 0);
    assert_eq!(
        app.update(cx, |app, _| (
            smartart::current_line(app),
            smartart::editing(app)
        )),
        (Some(0), None)
    );
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Typing"));
    // A double click types at once.
    let sales = shape_of_line(&app, cx, 1);
    cx.simulate_mouse_down(sales, gpui::MouseButton::Left, Modifiers::none());
    cx.simulate_mouse_up(sales, gpui::MouseButton::Left, Modifiers::none());
    cx.simulate_event(gpui::MouseDownEvent {
        button: gpui::MouseButton::Left,
        position: sales,
        modifiers: Modifiers::none(),
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_mouse_up(sales, gpui::MouseButton::Left, Modifiers::none());
    redraw(cx);
    assert_eq!(
        app.update(cx, |app, _| smartart::editing(app)),
        Some((id, 1))
    );
    // Typing in the Text Pane and in the shape are the same text: a line
    // of the pane takes the keyboard and the shape shows what it gets.
    let pane = app
        .update(cx, |app, _| smartart::pane(app))
        .expect("the pane");
    cx.simulate_keystrokes("end");
    cx.simulate_input(" Team");
    redraw(cx);
    assert_eq!(
        pane.read_with(cx, |pane, cx| pane.texts(cx)),
        vec!["Head", "Sales Team", "Works!"]
    );
    // On a protected sheet nothing is typed.
    cx.simulate_keystrokes("escape");
    redraw(cx);
    app.update(cx, |app, cx| {
        let sheet = app.sheet;
        app.wb.protect_sheet(
            sheet,
            opensheet_engine::protect::SheetProtection::default(),
            "",
        );
        app.commit("protect", cx);
    });
    let started = app.update_in(cx, |app, window, cx| {
        smartart::begin_edit(app, id, 0, TextStart::End, window, cx)
    });
    assert!(!started);
    assert_eq!(app.update(cx, |app, _| smartart::editing(app)), None);
}

// ----------------------------------------------------- the SmartArt Format tab

use opensheet_engine::drawing::{Glow, Outline as ShapeOutline, ShapePreset as Preset};
use opensheet_engine::smartart::NodeFormat;
use opensheet_engine::style::Color;

fn run(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cmd: Cmd) {
    app.update_in(cx, |app, window, cx| app.run_command(cmd, window, cx));
    redraw(cx);
}

fn format_of(
    app: &Entity<ExcelApp>,
    cx: &mut VisualTestContext,
    id: u32,
    line: usize,
) -> NodeFormat {
    art(app, cx, id).format(line).expect("the line")
}

#[gpui::test]
fn the_format_tab_formats_the_chosen_shapes_of_a_graphic(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let id = graphic(
        &app,
        cx,
        Layout::Process,
        &[(0, "Plan"), (0, "Build"), (0, "Ship")],
    );
    // A graphic has two tabs: SmartArt Design and Format.
    let tabs: Vec<&str> = app.update(cx, |app, _| {
        Drawings
            .context_tabs(app)
            .iter()
            .map(|tab| tab.id)
            .collect()
    });
    assert_eq!(tabs, [SMARTART_TAB, SMARTART_FORMAT_TAB]);
    app.update(cx, |app, cx| {
        app.context_tab = Some(SMARTART_FORMAT_TAB);
        cx.notify();
    });
    redraw(cx);
    // With no shape chosen, a shape cannot be made larger; a fill goes
    // to every shape, as one undo step.
    app.update(cx, |app, cx| smartart::leave_node(app, cx));
    assert!(app.update(cx, |app, _| !Cmd::SmartLarger.enabled(app)
        && Cmd::ShapeFill.enabled(app)
        && Cmd::TextOutline.enabled(app)));
    app.update(cx, |app, cx| tabs::set_fill(app, Some(Color(0x7030A0)), cx));
    assert_eq!(
        (0..3)
            .map(|line| format_of(&app, cx, id, line).fill)
            .collect::<Vec<_>>(),
        vec![Some(Some(Color(0x7030A0))); 3]
    );
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Shape Fill"));
    undo(&app, cx);
    assert!(!art(&app, cx, id).has_formats());

    // One shape chosen, a second with Shift: the commands go to both.
    click_line(&app, cx, 0);
    let ship = shape_of_line(&app, cx, 2);
    cx.simulate_click(ship, Modifiers::shift());
    redraw(cx);
    assert_eq!(app.update(cx, |app, _| smartart::chosen_lines(app)), [0, 2]);
    assert!(app.update(cx, |app, _| Cmd::SmartLarger.enabled(app)));
    run(&app, cx, Cmd::SmartLarger);
    run(&app, cx, Cmd::SmartLarger);
    run(&app, cx, Cmd::SmartSmaller);
    assert_eq!(format_of(&app, cx, id, 0).scale, Some((1.1, 1.1)));
    assert_eq!(format_of(&app, cx, id, 2).scale, Some((1.1, 1.1)));
    assert!(format_of(&app, cx, id, 1).is_plain());
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Smaller"));
    // Shape Fill, Shape Outline, Shape Effects, Text Fill, Text Outline:
    // what their menus call.
    app.update(cx, |app, cx| {
        tabs::set_fill(app, Some(Color(0xC00000)), cx);
        tabs::set_outline_color(app, Some(Color(0x00B050)), cx);
        tabs::set_outline(app, cx, |outline| outline.width = 3.0);
        adjust::set_effects(app, "Glow", cx, |effects| {
            effects.glow = Some(Glow {
                color: Color(0xFFC000),
                transparency: 0.5,
                radius: 8.0,
            })
        });
        tabs::set_text_color(app, Some(Color(0xFFFF00)), cx);
        wordart::format(app, Cmd::TextOutline.label(), cx, &|_, effects| {
            effects.outline = Some(opensheet_engine::wordart::TextOutline {
                color: Color(0x0070C0),
                width: 1.0,
            })
        });
    });
    for line in [0, 2] {
        let format = format_of(&app, cx, id, line);
        assert_eq!(format.fill, Some(Some(Color(0xC00000))));
        assert_eq!(
            format.outline,
            Some(Some(ShapeOutline::solid(Color(0x00B050), 3.0)))
        );
        assert_eq!(
            format
                .effects
                .and_then(|effects| effects.glow)
                .map(|glow| glow.radius),
            Some(8.0)
        );
        assert_eq!(format.text_color, Some(Color(0xFFFF00)));
        assert_eq!(
            format
                .text_effects
                .and_then(|effects| effects.outline)
                .map(|outline| outline.color),
            Some(Color(0x0070C0))
        );
    }
    assert!(format_of(&app, cx, id, 1).is_plain());
    // Each was one undo step.
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Text Outline"));
    undo(&app, cx);
    assert_eq!(format_of(&app, cx, id, 0).text_effects, None);
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Text Fill"));

    // Change Shape: its menu, and a real click on its first entry.
    click_line(&app, cx, 1);
    run(&app, cx, Cmd::SmartChangeShape);
    let (entry, first) = app.update(cx, |app, _| {
        let popup = app.popup.as_ref().expect("the Change Shape menu");
        let panel = popup.geometry.borrow().panels[0];
        let first = ShapeTool::ALL
            .into_iter()
            .find(|tool| !tool.preset().is_line() && *tool != ShapeTool::TextBox)
            .unwrap();
        (
            point(panel.origin.x + px(40.), panel.origin.y + px(18.)),
            first.preset(),
        )
    });
    click(cx, entry);
    assert!(
        app.update(cx, |app, _| app.popup.is_none()),
        "the menu closes"
    );
    // The first shape of the gallery: a rectangle, where a process has
    // rounded ones.
    assert_eq!(first, Preset::Rect);
    assert_eq!(format_of(&app, cx, id, 1).preset, Some(Preset::Rect));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Change Shape"));
    // The shapes the graphic is drawn as follow, and the look stays with
    // its entry when the outline is reordered.
    let pieces = app.update(cx, |app, _| art_pieces(app, id));
    let of = |line: usize| {
        pieces
            .iter()
            .find(|piece| piece.node == Some(line))
            .unwrap()
    };
    assert_eq!(
        (
            of(0).shape.fill,
            of(1).shape.preset,
            of(1).shape.fill == of(0).shape.fill
        ),
        (Some(Color(0xC00000)), Preset::Rect, false)
    );
    assert!(of(0).width > of(1).width * 1.05);
    assert_eq!(of(0).shape.font.color, Some(Color(0xFFFF00)));
    click_line(&app, cx, 0);
    run(&app, cx, Cmd::SmartMoveDown);
    assert_eq!(outline(&app, cx, id), "Build|Plan|Ship");
    assert_eq!(format_of(&app, cx, id, 1).fill, Some(Some(Color(0xC00000))));
    assert_eq!(format_of(&app, cx, id, 0).preset, Some(Preset::Rect));

    // Reset Graphic, on the Design tab, clears all of it in one step.
    run(&app, cx, Cmd::SmartReset);
    assert!(!art(&app, cx, id).has_formats());
    undo(&app, cx);
    assert!(art(&app, cx, id).has_formats());
    // A protected sheet keeps the shapes as they are.
    app.update(cx, |app, cx| {
        let sheet = app.sheet;
        app.wb.protect_sheet(
            sheet,
            opensheet_engine::protect::SheetProtection::default(),
            "",
        );
        app.commit("protect", cx);
    });
    let before = art(&app, cx, id);
    app.update(cx, |app, cx| tabs::set_fill(app, None, cx));
    run(&app, cx, Cmd::SmartLarger);
    assert_eq!(art(&app, cx, id), before);
}

fn art_pieces(app: &ExcelApp, id: u32) -> Vec<opensheet_engine::smartart::Piece> {
    let frame = app.wb.drawing_frame(app.sheet, id).unwrap();
    let art = app
        .wb
        .drawing(app.sheet, id)
        .and_then(Drawing::as_smartart)
        .unwrap();
    art.pieces(app.wb.theme(), frame.width as f32, frame.height as f32)
}

// ------------------------------------------------------ one member of a group

use gpui::{MouseButton as Button, Point};
use opensheet_engine::drawing::Frame as Box4;

fn add(
    app: &Entity<ExcelApp>,
    cx: &mut VisualTestContext,
    tool: ShapeTool,
    frame: (f64, f64, f64, f64),
) -> u32 {
    let id = app.update(cx, |app, cx| {
        let id = app
            .wb
            .add_shape(
                app.sheet,
                tool,
                Box4::new(frame.0, frame.1, frame.2, frame.3),
            )
            .unwrap();
        app.commit("add", cx);
        id
    });
    redraw(cx);
    id
}

fn frame(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32) -> (i64, i64, i64, i64) {
    app.update(cx, |app, _| {
        let frame = app.wb.drawing_frame(app.sheet, id).expect("the drawing");
        (
            frame.x.round() as i64,
            frame.y.round() as i64,
            frame.width.round() as i64,
            frame.height.round() as i64,
        )
    })
}

/// A point of the sheet (pixels at 100 % from the corner of A1) in the
/// window.
fn on_sheet(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, x: f64, y: f64) -> Point<Pixels> {
    app.update(cx, |app, _| {
        let layout = app.current_layout().unwrap();
        let pane = *paint::panes(&layout).last().unwrap();
        let (px_x, px_y) = paint::zoomed(&layout, &app.wb.sheets[app.sheet], x, y);
        layout.to_window((px_x + pane.dx) as f32, (px_y + pane.dy) as f32)
    })
}

fn click_at(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, x: f64, y: f64) {
    let at = on_sheet(app, cx, x, y);
    click(cx, at);
}

fn drag_at(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, from: (f64, f64), to: (f64, f64)) {
    let (from, to) = (
        on_sheet(app, cx, from.0, from.1),
        on_sheet(app, cx, to.0, to.1),
    );
    let middle = point((from.x + to.x) / 2.0, (from.y + to.y) / 2.0);
    cx.simulate_mouse_down(from, Button::Left, Modifiers::none());
    cx.simulate_mouse_move(middle, Button::Left, Modifiers::none());
    cx.simulate_mouse_move(to, Button::Left, Modifiers::none());
    cx.simulate_mouse_up(to, Button::Left, Modifiers::none());
    redraw(cx);
}

fn member(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Option<u32> {
    app.update(cx, |app, _| member_selected(app).map(|(_, member)| member))
}

fn fill_of(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32) -> Option<Color> {
    app.update(cx, |app, _| {
        app.wb
            .drawing(app.sheet, id)
            .and_then(Drawing::as_shape)
            .and_then(|shape| shape.fill)
    })
}

/// Three shapes in a group, the group selected: a rectangle, an ellipse
/// to its right and a diamond under them.
fn grouped(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> (u32, [u32; 3]) {
    let a = add(app, cx, ShapeTool::Rectangle, (100.0, 100.0, 100.0, 60.0));
    let b = add(app, cx, ShapeTool::Ellipse, (240.0, 100.0, 80.0, 60.0));
    let c = add(app, cx, ShapeTool::Diamond, (160.0, 190.0, 60.0, 60.0));
    let group = app.update(cx, |app, cx| {
        let group = app
            .wb
            .group_drawings(app.sheet, &[a, b, c])
            .expect("a group");
        app.commit("group", cx);
        app.select_object(Some(object_id(&Drawings, group)), cx);
        group
    });
    redraw(cx);
    (group, [a, b, c])
}

#[gpui::test]
fn a_member_of_a_group_is_selected_formatted_moved_and_resized(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let (group, [a, b, c]) = grouped(&app, cx);
    assert_eq!(frame(&app, cx, group), (100, 100, 220, 150));
    // A click on a member of the selected group selects the member; the
    // group stays the selected object.
    click_at(&app, cx, 280.0, 130.0);
    assert_eq!(member(&app, cx), Some(b));
    assert_eq!(
        app.update(cx, |app, _| (app.object_sel, selected(app).map(|d| d.id))),
        (Some(object_id(&Drawings, group)), Some(b))
    );
    assert_eq!(
        undo_label(&app, cx).as_deref(),
        Some("group"),
        "selecting is no undo step"
    );
    // Shape Format works on the member alone.
    app.update(cx, |app, cx| tabs::set_fill(app, Some(Color(0xC00000)), cx));
    assert_eq!(
        (
            fill_of(&app, cx, b),
            fill_of(&app, cx, a) == Some(Color(0xC00000))
        ),
        (Some(Color(0xC00000)), false)
    );
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Shape Fill"));
    undo(&app, cx);
    assert_ne!(fill_of(&app, cx, b), Some(Color(0xC00000)));
    assert_eq!(member(&app, cx), Some(b), "the member stays selected");
    // Dragged, the member moves alone and the box of the group follows.
    drag_at(&app, cx, (280.0, 130.0), (380.0, 150.0));
    assert_eq!(frame(&app, cx, b), (340, 120, 80, 60));
    assert_eq!(frame(&app, cx, a), (100, 100, 100, 60));
    assert_eq!(frame(&app, cx, group), (100, 100, 320, 150));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Move Object"));
    undo(&app, cx);
    assert_eq!(
        (frame(&app, cx, b), frame(&app, cx, group)),
        ((240, 100, 80, 60), (100, 100, 220, 150))
    );
    // Its bottom-right handle resizes it alone.
    let handle = app.update(cx, |app, _| {
        let layout = app.current_layout().unwrap();
        let pane = *paint::panes(&layout).last().unwrap();
        let held = selected(app).unwrap();
        let placed = paint::placed(&layout, &app.wb.sheets[app.sheet], held, &pane);
        let at = paint::grips(held, &placed)
            .into_iter()
            .find(|(grip, _)| *grip == Grip::Resize(4))
            .unwrap()
            .1;
        layout.to_window(at.0, at.1)
    });
    let to = on_sheet(&app, cx, 360.0, 200.0);
    cx.simulate_mouse_down(handle, Button::Left, Modifiers::none());
    cx.simulate_mouse_move(to, Button::Left, Modifiers::none());
    cx.simulate_mouse_up(to, Button::Left, Modifiers::none());
    redraw(cx);
    assert_eq!(frame(&app, cx, b), (240, 100, 120, 100));
    assert_eq!(frame(&app, cx, group), (100, 100, 260, 150));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Size Object"));
    // The arrow keys nudge the member.
    cx.simulate_keystrokes("right");
    redraw(cx);
    assert_eq!(
        (frame(&app, cx, b), frame(&app, cx, a)),
        ((241, 100, 120, 100), (100, 100, 100, 60))
    );
    // Tab goes to the next member, Shift+Tab back; Escape gives the
    // group back as a whole, and Escape again deselects it.
    cx.simulate_keystrokes("tab");
    redraw(cx);
    assert_eq!(member(&app, cx), Some(c));
    cx.simulate_keystrokes("tab");
    redraw(cx);
    assert_eq!(member(&app, cx), Some(a));
    cx.simulate_keystrokes("shift-tab shift-tab");
    redraw(cx);
    assert_eq!(member(&app, cx), Some(b));
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!(
        (member(&app, cx), app.update(cx, |app, _| app.object_sel)),
        (None, Some(object_id(&Drawings, group)))
    );
    // With the group selected and no member, a drag from a member moves
    // the whole group.
    drag_at(&app, cx, (150.0, 130.0), (170.0, 140.0));
    assert_eq!(member(&app, cx), None);
    assert_eq!(
        (frame(&app, cx, group), frame(&app, cx, a)),
        ((120, 110, 261, 150), (120, 110, 100, 60))
    );
}

#[gpui::test]
fn a_member_of_a_group_is_typed_in_and_deleted(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    let (group, [a, b, c]) = grouped(&app, cx);
    let text = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32| {
        app.update(cx, |app, _| {
            app.wb
                .drawing(app.sheet, id)
                .and_then(Drawing::as_shape)
                .map(|shape| shape.text.clone())
                .unwrap_or_default()
        })
    };
    // A key with a member selected types in it.
    click_at(&app, cx, 150.0, 130.0);
    assert_eq!(member(&app, cx), Some(a));
    cx.simulate_input("North");
    redraw(cx);
    assert_eq!(app.update(cx, |app, _| editing_text(app)), Some(a));
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!(
        (text(&app, cx, a), member(&app, cx)),
        ("North".to_string(), Some(a))
    );
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Typing"));
    // A double click on another member types in that one.
    let at = on_sheet(&app, cx, 280.0, 130.0);
    cx.simulate_mouse_down(at, Button::Left, Modifiers::none());
    cx.simulate_mouse_up(at, Button::Left, Modifiers::none());
    cx.simulate_event(gpui::MouseDownEvent {
        button: Button::Left,
        position: at,
        modifiers: Modifiers::none(),
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_mouse_up(at, Button::Left, Modifiers::none());
    redraw(cx);
    assert_eq!(app.update(cx, |app, _| editing_text(app)), Some(b));
    cx.simulate_input("East");
    cx.simulate_keystrokes("escape");
    redraw(cx);
    assert_eq!(text(&app, cx, b), "East");
    // Bold from the Home tab goes to the member.
    app.update(cx, |app, cx| {
        Drawings.object_style(app, group, "Bold", &|style| style.font.bold = true, cx);
    });
    let bold = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext, id: u32| {
        app.update(cx, |app, _| {
            app.wb
                .drawing(app.sheet, id)
                .and_then(Drawing::as_shape)
                .is_some_and(|shape| shape.font.bold)
        })
    };
    assert_eq!((bold(&app, cx, b), bold(&app, cx, a)), (true, false));
    // Delete removes the member; the group keeps the two others.
    cx.simulate_keystrokes("delete");
    redraw(cx);
    let members = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| {
        app.update(cx, |app, _| {
            app.wb
                .drawing(app.sheet, group)
                .and_then(Drawing::as_group)
                .map(|held| held.members.iter().map(|m| m.id).collect::<Vec<_>>())
        })
    };
    assert_eq!(members(&app, cx), Some(vec![a, c]));
    assert_eq!(frame(&app, cx, group), (100, 100, 120, 150));
    assert_eq!(undo_label(&app, cx).as_deref(), Some("Delete"));
    undo(&app, cx);
    assert_eq!(members(&app, cx), Some(vec![a, b, c]));
    app.update(cx, |app, cx| app.redo(cx));
    redraw(cx);
    // A group left with one member dissolves: the member is a shape of
    // the sheet again, where it was.
    app.update(cx, |app, cx| {
        app.select_object(Some(object_id(&Drawings, group)), cx)
    });
    redraw(cx);
    click_at(&app, cx, 190.0, 220.0);
    assert_eq!(member(&app, cx), Some(c));
    cx.simulate_keystrokes("delete");
    redraw(cx);
    assert_eq!(
        app.update(cx, |app, _| app.wb.sheets[app.sheet]
            .drawings
            .iter()
            .map(|d| d.id)
            .collect::<Vec<_>>()),
        vec![a]
    );
    assert_eq!(frame(&app, cx, a), (100, 100, 100, 60));
    undo(&app, cx);
    assert_eq!(members(&app, cx), Some(vec![a, c]));
    // On a protected sheet a member is selected and nothing more.
    app.update(cx, |app, cx| {
        let sheet = app.sheet;
        app.wb.protect_sheet(
            sheet,
            opensheet_engine::protect::SheetProtection::default(),
            "",
        );
        app.commit("protect", cx);
        select_member(app, group, Some(a), cx);
    });
    redraw(cx);
    app.update(cx, |app, cx| tabs::set_fill(app, Some(Color(0x00FF00)), cx));
    drag_at(&app, cx, (150.0, 130.0), (200.0, 160.0));
    assert_eq!(
        (
            fill_of(&app, cx, a) == Some(Color(0x00FF00)),
            frame(&app, cx, a)
        ),
        (false, (100, 100, 100, 60))
    );
}
