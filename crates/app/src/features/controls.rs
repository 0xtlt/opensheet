//! Form controls: the Developer tab and its Controls group, the controls
//! drawn on the sheet, and what a click on them does.
//!
//! * this file: the commands, the objects (selecting, moving, resizing,
//!   copying and stacking come from `objects.rs`), drawing a new control
//!   with the mouse, and working a control with a click;
//! * `paint.rs`: how each kind is drawn, and where its parts are;
//! * `dialogs.rs`: Format Control, Assign Macro and Edit Text.
//!
//! As in Excel, a plain click works a control and a ⌃-click or a right
//! click selects it; once selected it is moved and resized like any
//! object, until a click elsewhere lets go of it. A control never runs a
//! macro: one assigned to a macro shows the message of the Macro dialog.
//!
//! The controls are objects of this feature and lie over the charts, the
//! pictures and the shapes, in an order of their own.

mod dialogs;
mod paint;

#[cfg(test)]
mod ui_tests;

use super::{
    Command, ContextMenu, ContextTab, Cx, Feature, Intent, MenuSection, MenuTarget, ObjectInfo, ObjectVerb, PaintLayer,
    Painter, Placement, ZMove, Zone, ZoneEvent, ZoneResponse, automate, object_id, preferences,
};
use crate::app::{ExcelApp, MenuEntry, Popup, PopupKind};
use crate::i18n::t;
use crate::ribbon::{group, row, rows};
use gpui::{AnyElement, CursorStyle, IntoElement, MouseButton, Styled, Window};
use opensheet_engine::Rect;
use opensheet_engine::control::{CheckState, ControlKind, FormControl, stepped};
use paint::Part;
use std::any::Any;
use std::collections::HashMap;
use std::rc::Rc;

pub struct Controls;

/// Id of the Developer tab.
pub(crate) const TAB: &str = "developer";

commands! {
    /// What the user can ask of form controls.
    pub enum Cmd {
        // Developer ▸ Code
        Macros = "automate-macro", t("Macros", "Macros");
        // Developer ▸ Form Controls: the next drag on the sheet draws one.
        Button = "controls-button", t("Button", "Bouton");
        CheckBox = "controls-check", t("Check Box", "Case à cocher");
        OptionButton = "controls-option", t("Option Button", "Case d'option");
        ComboBox = "controls-combo", t("Combo Box", "Zone de liste modifiable");
        ListBox = "controls-list", t("List Box", "Zone de liste");
        SpinButton = "controls-spin", t("Spin Button", "Toupie");
        ScrollBar = "controls-scroll", t("Scroll Bar", "Barre de défilement");
        GroupBox = "controls-group", t("Group Box", "Zone de groupe");
        Label = "controls-label", t("Label", "Étiquette");
        // The selected control
        FormatControl = "format-cells", t("Format Control...", "Format de contrôle...");
        AssignMacro = "automate-macro", t("Assign Macro...", "Affecter une macro...");
        EditText = "textbox", t("Edit Text", "Modifier le texte");
    }
}

impl Cmd {
    /// The kind of control an Insert command draws.
    fn kind(self) -> Option<ControlKind> {
        Some(match self {
            Cmd::Button => ControlKind::Button,
            Cmd::CheckBox => ControlKind::CheckBox,
            Cmd::OptionButton => ControlKind::OptionButton,
            Cmd::ComboBox => ControlKind::ComboBox,
            Cmd::ListBox => ControlKind::ListBox,
            Cmd::SpinButton => ControlKind::SpinButton,
            Cmd::ScrollBar => ControlKind::ScrollBar,
            Cmd::GroupBox => ControlKind::GroupBox,
            Cmd::Label => ControlKind::Label,
            _ => return None,
        })
    }
}

impl Command for Cmd {
    fn label(self) -> &'static str {
        self.face().1
    }

    fn icon(self) -> &'static str {
        self.face().0
    }

    fn enabled(self, app: &ExcelApp) -> bool {
        match self {
            Cmd::FormatControl | Cmd::AssignMacro => selected(app).is_some(),
            Cmd::EditText => selected(app).is_some_and(|control| control.kind.has_text()),
            _ => true,
        }
    }

    fn checked(self, app: &ExcelApp) -> bool {
        self.kind().is_some() && app.ext::<ControlsUi>().and_then(|ui| ui.tool) == self.kind()
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        if app.is_editing() && !app.commit_edit(None, window, cx) {
            return;
        }
        match self {
            Cmd::Macros => {
                automate::open_macro_dialog(app, window, cx);
            }
            Cmd::FormatControl => {
                dialogs::open_format(app, window, cx);
            }
            Cmd::AssignMacro => {
                dialogs::open_assign_macro(app, window, cx);
            }
            Cmd::EditText => {
                dialogs::open_edit_text(app, window, cx);
            }
            _ => {
                if let Some(kind) = self.kind() {
                    arm(app, kind, window, cx);
                }
            }
        }
    }
}

// -------------------------------------------------------------------- state

/// What the mouse is doing to the controls.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Press {
    /// A part of a control is held down; it acts when the button is
    /// released over it.
    Part(u32, Part),
    /// The thumb of a scroll bar is dragged: the pointer was this far
    /// along the thumb when the button went down.
    Thumb(u32, f32),
    /// A new control is dragged out, from one point of the grid element
    /// to another.
    Draw { start: (f32, f32), end: (f32, f32) },
}

/// What the interface remembers of the controls between events.
#[derive(Default)]
pub(crate) struct ControlsUi {
    /// The kind of control the next drag on the sheet draws.
    tool: Option<ControlKind>,
    press: Option<Press>,
    /// The first line shown by each list box that was scrolled.
    list_top: HashMap<u32, u32>,
}

/// What the zone over a control carries: the control as it was painted.
pub(crate) struct ControlZone {
    pub id: u32,
    /// Its box in grid-element coordinates, and the zoom.
    pub rect: (f32, f32, f32, f32),
    pub zoom: f32,
}

/// The zone that covers the cells while a control waits to be drawn.
struct Canvas;

// ------------------------------------------------------------------ helpers

/// The control that is selected, when one is and nothing else.
pub(crate) fn selected(app: &ExcelApp) -> Option<&FormControl> {
    let id = app.object_sel.filter(|id| id.owner == super::index_of(&Controls))?;
    app.wb.sheets[app.sheet].control(id.id)
}

/// The name of a kind of control, as the ribbon and the undo list say it.
pub(crate) fn kind_label(kind: ControlKind) -> &'static str {
    match kind {
        ControlKind::Button => Cmd::Button.label(),
        ControlKind::CheckBox => Cmd::CheckBox.label(),
        ControlKind::OptionButton => Cmd::OptionButton.label(),
        ControlKind::ComboBox => Cmd::ComboBox.label(),
        ControlKind::ListBox => Cmd::ListBox.label(),
        ControlKind::SpinButton => Cmd::SpinButton.label(),
        ControlKind::ScrollBar => Cmd::ScrollBar.label(),
        ControlKind::GroupBox => Cmd::GroupBox.label(),
        ControlKind::Label => Cmd::Label.label(),
    }
}

/// The first line a list box shows.
pub(crate) fn list_top(app: &ExcelApp, id: u32) -> u32 {
    app.ext::<ControlsUi>().and_then(|ui| ui.list_top.get(&id).copied()).unwrap_or(0)
}

/// The part of a control that is held down, if any.
pub(crate) fn pressed_part(app: &ExcelApp, id: u32) -> Option<Part> {
    match app.ext::<ControlsUi>()?.press? {
        Press::Part(held, part) if held == id => Some(part),
        Press::Thumb(held, _) if held == id => Some(Part::Thumb),
        _ => None,
    }
}

/// True when the developer tab is asked for in Preferences ▸ View.
fn tab_shown() -> bool {
    preferences::settings::get().developer_tab
}

// ------------------------------------------------------- drawing a control

/// Take a control in hand: the next drag on the sheet draws it.
fn arm(app: &mut ExcelApp, kind: ControlKind, window: &mut Window, cx: &mut Cx) {
    if !app.may(Intent::EditObject(None), cx) {
        return;
    }
    app.popup = None;
    let ui = app.ext_mut::<ControlsUi>();
    // The button that is down puts the tool back.
    ui.tool = if ui.tool == Some(kind) { None } else { Some(kind) };
    ui.press = None;
    window.focus(&app.grid_focus);
    cx.notify();
}

/// Put the tool down. False when no control was waiting.
fn disarm(app: &mut ExcelApp, cx: &mut Cx) -> bool {
    if app.ext::<ControlsUi>().is_none_or(|ui| ui.tool.is_none()) {
        return false;
    }
    let ui = app.ext_mut::<ControlsUi>();
    ui.tool = None;
    ui.press = None;
    cx.notify();
    true
}

/// Put a new control of `kind` on the sheet, in the box dragged out on
/// the grid element (a click gives it the size Excel does), select it,
/// and ask for the macro of a button as Excel does.
pub(crate) fn insert(app: &mut ExcelApp, kind: ControlKind, start: (f32, f32), end: (f32, f32), window: &mut Window, cx: &mut Cx) -> Option<u32> {
    let layout = app.current_layout()?;
    let z = layout.zoom;
    let (x0, y0) = (start.0.min(end.0), start.1.min(end.1));
    let dragged = ((start.0 - end.0).abs() / z, (start.1 - end.1).abs() / z);
    let mut control = FormControl::new(kind);
    if dragged.0 >= 4.0 && dragged.1 >= 4.0 {
        control.size = dragged;
        // A scroll bar drawn wider than tall lies down.
        control.horizontal = kind == ControlKind::ScrollBar && dragged.0 > dragged.1;
    }
    let (row, col) = (layout.row_at(y0), layout.col_at(x0));
    control.anchor = (row, col);
    control.offset = (((x0 - layout.col_x(col)) / z).max(0.0), ((y0 - layout.row_y(row)) / z).max(0.0));
    let sheet = app.sheet;
    let id = app.wb.add_control(sheet, control)?;
    app.commit(t("Insert Control", "Insérer un contrôle"), cx);
    app.select_object(Some(object_id(&Controls, id)), cx);
    if kind == ControlKind::Button && app.wb.has_vba_project() {
        dialogs::open_assign_macro(app, window, cx);
    }
    Some(id)
}

// ------------------------------------------------------- working a control

/// What a control is called in the undo list when it is worked.
fn work_label(kind: ControlKind) -> &'static str {
    kind_label(kind)
}

/// True when the cell the control is linked to may be written.
fn may_write_link(app: &mut ExcelApp, control: &FormControl, cx: &mut Cx) -> bool {
    match app.wb.control_link(app.sheet, control) {
        Some((sheet, row, col)) if sheet == app.sheet => app.may(Intent::EditCells(Rect::cell(row, col)), cx),
        _ => true,
    }
}

/// After a control was worked: a control assigned to a macro says that
/// macros are kept, not run.
fn after_work(app: &mut ExcelApp, control: &FormControl, cx: &mut Cx) {
    if control.macro_name.is_some() {
        app.alert(automate::not_run_alert(), cx);
    }
}

/// Tick or untick a check box, choose an option button, press a button:
/// what a click released over the control does.
pub(crate) fn click(app: &mut ExcelApp, id: u32, cx: &mut Cx) {
    let sheet = app.sheet;
    let Some(control) = app.wb.sheets[sheet].control(id).cloned() else {
        return;
    };
    match control.kind {
        ControlKind::CheckBox => {
            if !may_write_link(app, &control, cx) {
                return;
            }
            let now = app.wb.control_state(sheet, &control).checked;
            let next = if now == CheckState::Checked { CheckState::Unchecked } else { CheckState::Checked };
            app.wb.set_control_checked(id, next);
            app.commit(work_label(control.kind), cx);
        }
        ControlKind::OptionButton => {
            let linked = app.wb.sheets[sheet].option_group(id).into_iter().find_map(|member| app.wb.sheets[sheet].control(member).filter(|c| c.link.is_some()).cloned());
            if linked.is_some_and(|linked| !may_write_link(app, &linked, cx)) {
                return;
            }
            app.wb.choose_option(id);
            app.commit(work_label(control.kind), cx);
        }
        _ => {}
    }
    after_work(app, &control, cx);
}

/// Choose a line of a combo or list box (1 = the first).
pub(crate) fn choose(app: &mut ExcelApp, id: u32, line: u32, cx: &mut Cx) {
    let Some(control) = app.wb.sheets[app.sheet].control(id).cloned() else {
        return;
    };
    if !may_write_link(app, &control, cx) {
        return;
    }
    if app.wb.set_control_selected(id, line) {
        app.commit(work_label(control.kind), cx);
    }
    after_work(app, &control, cx);
}

/// Give a spin button or a scroll bar a value; `commit` false while its
/// thumb is dragged (the release commits).
pub(crate) fn set_value(app: &mut ExcelApp, id: u32, value: i32, commit: bool, cx: &mut Cx) {
    let sheet = app.sheet;
    let Some(control) = app.wb.sheets[sheet].control(id).cloned() else {
        return;
    };
    let now = app.wb.control_state(sheet, &control).value;
    if stepped(&control, value, 0) != now {
        if !may_write_link(app, &control, cx) {
            return;
        }
        app.wb.set_control_value(id, value);
        if commit {
            app.commit(work_label(control.kind), cx);
        }
    }
    if commit {
        after_work(app, &control, cx);
    }
    cx.notify();
}

/// The list of a combo box, hanging under it.
fn open_combo(app: &mut ExcelApp, hit: &ControlZone, cx: &mut Cx) {
    let sheet = app.sheet;
    let Some(control) = app.wb.sheets[sheet].control(hit.id) else {
        return;
    };
    let now = app.wb.control_state(sheet, control);
    let id = hit.id;
    let entries: Vec<MenuEntry> = app
        .wb
        .control_items(sheet, control)
        .into_iter()
        .enumerate()
        .map(|(k, item)| MenuEntry::new(item, move |app, _, cx| choose(app, id, k as u32 + 1, cx)).checked(now.selected == k as u32 + 1))
        .collect();
    if entries.is_empty() {
        return;
    }
    let Some(layout) = app.current_layout() else {
        return;
    };
    let lines = control.drop_lines.clamp(1, 30) as f32;
    let width = ((hit.rect.2 - hit.rect.0).max(80.0)).min(480.0);
    app.popup = Some(Popup::new(PopupKind::Menu(entries), layout.to_window(hit.rect.0, hit.rect.3), width).max_height(lines * 26.0 + 10.0));
    cx.notify();
}

/// A press on a control that is not selected: it is worked.
fn press(app: &mut ExcelApp, hit: &ControlZone, ev: &ZoneEvent, cx: &mut Cx) -> ZoneResponse {
    let sheet = app.sheet;
    let Some(control) = app.wb.sheets[sheet].control(hit.id).cloned() else {
        return ZoneResponse::Ignored;
    };
    // Working a control lets go of whatever object was selected.
    if app.object_sel.is_some() {
        app.deselect_object(cx);
    }
    app.popup = None;
    let now = app.wb.control_state(sheet, &control);
    let geometry = paint::Geometry::of(&now, hit.rect, hit.zoom, app.wb.control_items(sheet, &control).len() as u32, list_top(app, hit.id));
    let part = geometry.part_at(ev.x, ev.y);
    let id = hit.id;
    match part {
        Part::Face => {
            app.ext_mut::<ControlsUi>().press = Some(Press::Part(id, part));
            ZoneResponse::Capture
        }
        Part::Drop => {
            open_combo(app, hit, cx);
            ZoneResponse::Handled
        }
        Part::Line(line) => {
            choose(app, id, line, cx);
            ZoneResponse::Handled
        }
        Part::Less | Part::More | Part::PageLess | Part::PageMore => {
            let delta = match part {
                Part::Less => -control.inc,
                Part::More => control.inc,
                Part::PageLess => -control.page,
                _ => control.page,
            };
            // The upper arrow of a spin button adds, the upper arrow of a
            // scroll bar takes away.
            let delta = if control.kind == ControlKind::SpinButton { -delta } else { delta };
            set_value(app, id, stepped(&control, now.value, delta), true, cx);
            ZoneResponse::Handled
        }
        Part::Thumb => {
            let along = geometry.along(ev.x, ev.y) - geometry.thumb_start();
            app.ext_mut::<ControlsUi>().press = Some(Press::Thumb(id, along));
            ZoneResponse::Capture
        }
        Part::Nothing => {
            // A label or a group box does nothing, but for its macro.
            after_work(app, &control, cx);
            ZoneResponse::Handled
        }
    }
}

// ------------------------------------------------------------------ feature

impl Feature for Controls {
    fn name(&self) -> &'static str {
        "controls"
    }

    fn context_tabs(&self, _app: &ExcelApp) -> Vec<ContextTab> {
        if !tab_shown() {
            return Vec::new();
        }
        vec![ContextTab {
            id: TAB,
            title: t("Developer", "Développeur"),
        }]
    }

    fn render_tab(&self, app: &mut ExcelApp, _tab: &'static str, _window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
        vec![
            group(t("Code", "Code"), row(vec![app.cmd_large("dev-macros", Cmd::Macros, false, cx)])).into_any_element(),
            group(
                t("Form Controls", "Contrôles de formulaire"),
                row(vec![
                    rows(vec![
                        app.cmd_labeled("dev-button", Cmd::Button, false, cx),
                        app.cmd_labeled("dev-check", Cmd::CheckBox, false, cx),
                        app.cmd_labeled("dev-option", Cmd::OptionButton, false, cx),
                    ])
                    .into_any_element(),
                    rows(vec![
                        app.cmd_labeled("dev-combo", Cmd::ComboBox, false, cx),
                        app.cmd_labeled("dev-list", Cmd::ListBox, false, cx),
                        app.cmd_labeled("dev-spin", Cmd::SpinButton, false, cx),
                    ])
                    .into_any_element(),
                    rows(vec![
                        app.cmd_labeled("dev-scroll", Cmd::ScrollBar, false, cx),
                        app.cmd_labeled("dev-group", Cmd::GroupBox, false, cx),
                        app.cmd_labeled("dev-label", Cmd::Label, false, cx),
                    ])
                    .into_any_element(),
                ]),
            )
            .into_any_element(),
            group(
                t("Controls", "Contrôles"),
                rows(vec![
                    app.cmd_labeled("dev-format", Cmd::FormatControl, false, cx),
                    app.cmd_labeled("dev-assign", Cmd::AssignMacro, false, cx),
                    app.cmd_labeled("dev-text", Cmd::EditText, false, cx),
                ]),
            )
            .border_r_0()
            .into_any_element(),
        ]
    }

    /// While a control waits to be drawn: the crosshair over the cells,
    /// and the box as it is dragged out.
    fn paint(&self, p: &mut Painter) {
        if p.layer != PaintLayer::Top {
            return;
        }
        let Some(ui) = p.app.ext::<ControlsUi>().filter(|ui| ui.tool.is_some()) else {
            return;
        };
        if let Some(Press::Draw { start, end }) = ui.press {
            let (x0, y0, x1, y1) = (start.0.min(end.0), start.1.min(end.1), start.0.max(end.0), start.1.max(end.1));
            p.frame(x0, y0, x1, y1, 1.0, crate::theme::c(0x595959));
        }
        let layout = p.layout;
        p.zone((layout.header_w, layout.header_h, layout.width(), layout.height()), Canvas).cursor(CursorStyle::Crosshair);
    }

    fn zone_mouse_down(&self, app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) -> ZoneResponse {
        let armed = app.ext::<ControlsUi>().and_then(|ui| ui.tool).is_some();
        let hit = zone.payload::<ControlZone>();
        if zone.payload::<Canvas>().is_none() && hit.is_none() {
            return ZoneResponse::Ignored;
        }
        // A control in hand: the press starts its box, wherever it lands.
        if armed {
            if ev.button != MouseButton::Left {
                disarm(app, cx);
                return ZoneResponse::Handled;
            }
            app.ext_mut::<ControlsUi>().press = Some(Press::Draw {
                start: (ev.x, ev.y),
                end: (ev.x, ev.y),
            });
            return ZoneResponse::Capture;
        }
        let Some(hit) = hit else {
            return ZoneResponse::Ignored;
        };
        // The other buttons, ⌃, ⌘ and ⇧ select the control; once selected
        // it is an object like any other.
        let selecting = ev.button != MouseButton::Left || ev.modifiers.control || ev.modifiers.platform || ev.modifiers.shift;
        if selecting || app.is_object_selected(object_id(self, hit.id)) || app.object_pointer() {
            return ZoneResponse::Ignored;
        }
        if app.is_editing() && !app.commit_edit(None, window, cx) {
            return ZoneResponse::Handled;
        }
        let response = press(app, hit, ev, cx);
        cx.notify();
        response
    }

    fn zone_drag(&self, app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, _window: &mut Window, cx: &mut Cx) {
        let Some(press) = app.ext::<ControlsUi>().and_then(|ui| ui.press) else {
            return;
        };
        match press {
            Press::Draw { start, .. } => {
                app.ext_mut::<ControlsUi>().press = Some(Press::Draw { start, end: (ev.x, ev.y) });
            }
            Press::Thumb(id, grabbed) => {
                let (Some(hit), Some(control)) = (zone.payload::<ControlZone>(), app.wb.sheets[app.sheet].control(id).cloned()) else {
                    return;
                };
                let now = app.wb.control_state(app.sheet, &control);
                let geometry = paint::Geometry::of(&now, hit.rect, hit.zoom, 0, 0);
                let value = geometry.value_at(&control, geometry.along(ev.x, ev.y) - grabbed);
                set_value(app, id, value, false, cx);
            }
            Press::Part(..) => {}
        }
        cx.notify();
    }

    fn zone_mouse_up(&self, app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) {
        let Some(press) = app.ext_mut::<ControlsUi>().press.take() else {
            return;
        };
        match press {
            Press::Draw { start, .. } => {
                if let Some(kind) = app.ext_mut::<ControlsUi>().tool.take() {
                    insert(app, kind, start, (ev.x, ev.y), window, cx);
                }
            }
            Press::Thumb(id, _) => {
                if let Some(control) = app.wb.sheets[app.sheet].control(id).cloned() {
                    // The drag changed the control without committing.
                    app.commit(work_label(control.kind), cx);
                    after_work(app, &control, cx);
                }
            }
            Press::Part(id, _) => {
                if zone.contains(ev.x, ev.y) {
                    click(app, id, cx);
                }
            }
        }
        cx.notify();
    }

    /// The wheel over a list box turns its lines.
    fn zone_scroll(&self, app: &mut ExcelApp, zone: &Zone, delta: (f32, f32), _cx: &mut Cx) -> bool {
        let Some(hit) = zone.payload::<ControlZone>() else {
            return false;
        };
        let Some(control) = app.wb.sheets[app.sheet].control(hit.id).filter(|c| c.kind == ControlKind::ListBox) else {
            return false;
        };
        let lines = app.wb.control_items(app.sheet, control).len() as u32;
        let shown = paint::Geometry::of(control, hit.rect, hit.zoom, lines, 0).lines_shown();
        let hidden = lines.saturating_sub(shown);
        let top = list_top(app, hit.id) as i64 + if delta.1 < 0.0 { 1 } else { -1 };
        app.ext_mut::<ControlsUi>().list_top.insert(hit.id, top.clamp(0, hidden as i64) as u32);
        true
    }

    fn context_menu(&self, app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
        let MenuTarget::Object(id) = target else {
            return;
        };
        if id.owner != super::index_of(self) {
            return;
        }
        if app.wb.sheets[app.sheet].control(id.id).is_some_and(|control| control.kind.has_text()) {
            menu.add(MenuSection::Edit, MenuEntry::command(Cmd::EditText, app));
        }
        menu.add(MenuSection::Data, MenuEntry::command(Cmd::AssignMacro, app));
        menu.add(MenuSection::Format, MenuEntry::command(Cmd::FormatControl, app));
    }

    fn objects(&self, app: &ExcelApp, sheet: usize) -> Vec<ObjectInfo> {
        app.wb.sheets[sheet]
            .controls
            .iter()
            .map(|control| ObjectInfo {
                id: control.id,
                place: Placement {
                    anchor: control.anchor,
                    offset: control.offset,
                    size: control.size,
                },
                min_size: (8.0, 8.0),
                keep_aspect: false,
            })
            .collect()
    }

    fn paint_object(&self, p: &mut Painter, object: &ObjectInfo, rect: (f32, f32, f32, f32)) {
        paint::control(p, object.id, rect);
    }

    fn place_object(&self, app: &mut ExcelApp, id: u32, place: Placement) {
        app.wb.update_control(id, |control| {
            control.anchor = place.anchor;
            control.offset = place.offset;
            control.size = place.size;
        });
    }

    fn object_label(&self, _id: u32, verb: ObjectVerb) -> &'static str {
        match verb {
            ObjectVerb::Move => t("Move Object", "Déplacer l'objet"),
            ObjectVerb::Resize => t("Size Object", "Dimensionner l'objet"),
            ObjectVerb::Delete => t("Delete", "Supprimer"),
            ObjectVerb::Paste => t("Paste", "Coller"),
            ObjectVerb::Reorder => t("Change Order", "Changer l'ordre"),
        }
    }

    fn delete_object(&self, app: &mut ExcelApp, id: u32) {
        app.wb.remove_control(id);
    }

    fn copy_object(&self, app: &ExcelApp, id: u32) -> Option<Rc<dyn Any>> {
        let control = app.wb.sheets[app.sheet].control(id)?.clone();
        Some(Rc::new(control))
    }

    fn paste_object(&self, app: &mut ExcelApp, data: &dyn Any, at: ((u32, u32), (f32, f32))) -> Option<u32> {
        let mut control = data.downcast_ref::<FormControl>()?.clone();
        (control.anchor, control.offset) = at;
        // The copy is a control of its own, under a name of its own.
        control.name = String::new();
        control.source = None;
        app.wb.add_control(app.sheet, control)
    }

    fn reorder_object(&self, app: &mut ExcelApp, id: u32, mv: ZMove) {
        let sheet = app.sheet;
        app.wb.snapshot();
        let controls = &mut app.wb.sheets[sheet].controls;
        let Some(from) = controls.iter().position(|control| control.id == id) else {
            return;
        };
        let last = controls.len() - 1;
        let to = match mv {
            ZMove::ToFront => last,
            ZMove::Forward => (from + 1).min(last),
            ZMove::Backward => from.saturating_sub(1),
            ZMove::ToBack => 0,
        };
        let control = controls.remove(from);
        controls.insert(to, control);
    }

    /// A double click on a selected control opens its format.
    fn object_double_click(&self, app: &mut ExcelApp, _id: u32, _ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) {
        dialogs::open_format(app, window, cx);
    }

    fn cancel(&self, app: &mut ExcelApp, cx: &mut Cx) -> bool {
        disarm(app, cx)
    }
}
