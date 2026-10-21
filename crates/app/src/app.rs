//! The workbook window: state, selection, editing, clipboard, formatting.
//!
//! Features (filters, tables, comments, ...) are not written here: this
//! file calls the hooks of `features/mod.rs` at the right moments.

use crate::features::{
    self, CellEdit, ChangeCause, Command, ContextMenu, Intent, MenuSection, MenuTarget, ObjectId,
    PendingEdit, Verdict, ViewMode, Zone, ZoneEvent, ZoneResponse,
};
use crate::grid::{FontResolver, GridLayout, Selection};
use crate::i18n::t;
use crate::shell::{Alert, ModalEntry, PaneEntry};
use crate::text_input::{InputEvent, NavKey, TextInput};
use crate::theme;
use gpui::{
    AnyView, App, Bounds, ClipboardItem, Context, Entity, EntityInputHandler, FocusHandle,
    Focusable, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point,
    ScrollWheelEvent, SharedString, Subscription, Task, UTF16Selection, Window, actions, point,
    prelude::*, px,
};
use opensheet_engine::format;
use opensheet_engine::ops::{BorderPreset, Clip, Direction, PasteMode, SelectionStats};
use opensheet_engine::parser;
use opensheet_engine::refs::{MAX_COLS, MAX_ROWS, cell_name};
use opensheet_engine::style::{Border, Color, HAlign, Style, Underline, VAlign};
use opensheet_engine::workbook::ClearMode;
use opensheet_engine::{Locale, Rect, Workbook};
use std::any::{Any, TypeId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

actions!(
    opensheet,
    [
        MoveUp,
        MoveDown,
        MoveLeft,
        MoveRight,
        ExtendUp,
        ExtendDown,
        ExtendLeft,
        ExtendRight,
        JumpUp,
        JumpDown,
        JumpLeft,
        JumpRight,
        ExtendJumpUp,
        ExtendJumpDown,
        ExtendJumpLeft,
        ExtendJumpRight,
        NextCell,
        PrevCell,
        EnterDown,
        EnterUp,
        EditCell,
        ClearContents,
        Copy,
        Cut,
        Paste,
        PasteValues,
        Undo,
        Redo,
        SelectAll,
        Bold,
        Italic,
        UnderlineText,
        Save,
        SaveAs,
        Open,
        NewWorkbook,
        Find,
        FillDown,
        FillRight,
        PageUp,
        PageDown,
        GoRowStart,
        GoStart,
        GoEnd,
        SelectColumn,
        SelectRow,
        InsertDate,
        AutoSum,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        NextSheet,
        PrevSheet,
        Cancel,
        InsertRows,
        DeleteRows,
        Quit,
        FormatCells,
        Print,
        ToggleFilter,
        InsertTable,
        NewNote,
        NewComment,
        NameManager,
        InsertLink,
        OpenDropDown,
    ]
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditMode {
    /// Started by typing: arrow keys commit and move.
    Enter,
    /// Started with F2 / double click / the formula bar: arrows move the caret.
    Edit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditSource {
    Cell,
    FormulaBar,
}

/// A reference inserted by pointing at cells while typing a formula.
#[derive(Clone, Debug)]
pub struct PointState {
    /// Byte range of the reference text inside the editor.
    pub range: Range<usize>,
    pub anchor: (u32, u32),
    pub lead: (u32, u32),
}

pub struct Editing {
    pub sheet: usize,
    pub row: u32,
    pub col: u32,
    pub mode: EditMode,
    pub source: EditSource,
    pub point: Option<PointState>,
}

pub struct SheetView {
    pub sel: Selection,
    pub scroll_x: f64,
    pub scroll_y: f64,
    /// Normal, Page Layout or Page Break Preview.
    pub mode: ViewMode,
}

impl Default for SheetView {
    fn default() -> Self {
        SheetView {
            sel: Selection::single(0, 0),
            scroll_x: 0.0,
            scroll_y: 0.0,
            mode: ViewMode::Normal,
        }
    }
}

pub struct ClipState {
    pub clip: Clip,
    pub cut: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Hit {
    None,
    Corner,
    ColHeader(u32),
    ColResize(u32),
    RowHeader(u32),
    RowResize(u32),
    Cell(u32, u32),
    FillHandle,
    VThumb,
    VTrack,
    HThumb,
    HTrack,
    /// The body of a chart, picture or shape.
    Object(ObjectId),
    /// A resize handle of the selected object: 0 = top-left, then clockwise.
    ObjectHandle(ObjectId, u8),
    /// An area a feature made sensitive (index into `ExcelApp::zones`).
    Zone(usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum SelectKind {
    Cells,
    Cols,
    Rows,
}

#[derive(Clone)]
pub(crate) enum Drag {
    Select(SelectKind),
    Point(SelectKind),
    /// Picking cells for the range field of a dialog.
    Pick(SelectKind),
    Fill {
        src: Rect,
        dest: Rect,
    },
    ColResize {
        col: u32,
        start_x: f32,
        start_w: f32,
    },
    RowResize {
        row: u32,
        start_y: f32,
        start_h: f32,
    },
    VScroll {
        grab: f32,
    },
    HScroll {
        grab: f32,
    },
    /// `origin`: the object's top-left corner in sheet pixels at 100 %.
    ObjectMove {
        id: ObjectId,
        start: (f32, f32),
        origin: (f64, f64),
        size: (f32, f32),
    },
    /// `origin`: x, y, width, height in sheet pixels at 100 %.
    ObjectResize {
        id: ObjectId,
        handle: u8,
        start: (f32, f32),
        origin: (f64, f64, f64, f64),
    },
    /// A press a feature captured on one of its zones.
    Zone(Zone),
}

pub type MenuAction = Rc<dyn Fn(&mut ExcelApp, &mut Window, &mut Context<ExcelApp>)>;

#[derive(Clone)]
pub struct MenuEntry {
    pub label: SharedString,
    pub icon: Option<&'static str>,
    pub shortcut: Option<&'static str>,
    pub action: Option<MenuAction>,
    pub separator: bool,
    pub checked: bool,
    pub disabled: bool,
    /// The entries of the menu this one opens beside itself.
    pub submenu: Option<Vec<MenuEntry>>,
}

impl MenuEntry {
    pub fn new(
        label: impl Into<SharedString>,
        action: impl Fn(&mut ExcelApp, &mut Window, &mut Context<ExcelApp>) + 'static,
    ) -> MenuEntry {
        MenuEntry {
            label: label.into(),
            icon: None,
            shortcut: None,
            action: Some(Rc::new(action)),
            separator: false,
            checked: false,
            disabled: false,
            submenu: None,
        }
    }

    pub fn separator() -> MenuEntry {
        MenuEntry {
            label: SharedString::default(),
            icon: None,
            shortcut: None,
            action: None,
            separator: true,
            checked: false,
            disabled: false,
            submenu: None,
        }
    }

    /// An entry that opens another menu to its right.
    pub fn submenu(label: impl Into<SharedString>, entries: Vec<MenuEntry>) -> MenuEntry {
        MenuEntry {
            label: label.into(),
            icon: None,
            shortcut: None,
            action: None,
            separator: false,
            checked: false,
            disabled: false,
            submenu: Some(entries),
        }
    }

    /// The entry of a feature command: its label, icon and current state.
    pub fn command<C: Command>(cmd: C, app: &ExcelApp) -> MenuEntry {
        let mut entry = MenuEntry::new(cmd.label_in(app), move |a, window, cx| {
            cmd.run(a, window, cx)
        })
        .checked(cmd.checked(app))
        .disabled(!cmd.enabled(app));
        if !cmd.icon().is_empty() {
            entry.icon = Some(cmd.icon());
        }
        entry
    }

    pub fn icon(mut self, icon: &'static str) -> MenuEntry {
        self.icon = Some(icon);
        self
    }

    pub fn shortcut(mut self, s: &'static str) -> MenuEntry {
        self.shortcut = Some(s);
        self
    }

    pub fn checked(mut self, on: bool) -> MenuEntry {
        self.checked = on;
        self
    }

    pub fn disabled(mut self, on: bool) -> MenuEntry {
        self.disabled = on;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorTarget {
    Fill,
    Font,
    Tab,
}

pub enum PopupKind {
    Menu(Vec<MenuEntry>),
    Palette(ColorTarget),
    /// Anything a feature shows in a drop-down (a filter list, a date
    /// picker): the view sizes itself.
    View(AnyView),
}

/// Where the panels of the open pop-up were painted.
#[derive(Default)]
pub struct PopupGeometry {
    /// One per open menu level.
    pub panels: Vec<Bounds<Pixels>>,
    /// Entries that open a submenu, by (level, index).
    pub items: HashMap<(usize, usize), Bounds<Pixels>>,
}

pub struct Popup {
    /// The ribbon control it hangs from (None for context menus).
    pub anchor: Option<&'static str>,
    pub pos: Point<Pixels>,
    pub kind: PopupKind,
    pub width: f32,
    pub max_height: f32,
    /// The entries whose submenus are open, outermost first.
    pub open_path: Vec<usize>,
    pub geometry: Rc<RefCell<PopupGeometry>>,
}

impl Popup {
    pub fn new(kind: PopupKind, pos: Point<Pixels>, width: f32) -> Popup {
        Popup {
            anchor: None,
            pos,
            kind,
            width,
            max_height: 420.0,
            open_path: Vec::new(),
            geometry: Rc::default(),
        }
    }

    pub fn anchored(mut self, anchor: &'static str) -> Popup {
        self.anchor = Some(anchor);
        self
    }

    pub fn max_height(mut self, height: f32) -> Popup {
        self.max_height = height;
        self
    }
}

/// Cells being picked on the sheet for the range field of a dialog.
pub struct RangePick {
    pub input: Entity<TextInput>,
    pub style: crate::shell::RefStyle,
    pub rect: Rect,
    anchor: (u32, u32),
}

pub struct ExcelApp {
    pub wb: Workbook,
    pub path: Option<PathBuf>,
    pub title: String,
    pub sheet: usize,
    pub views: Vec<SheetView>,
    pub zoom: f32,
    pub fonts: Rc<FontResolver>,
    pub grid_focus: FocusHandle,
    pub cell_editor: Entity<TextInput>,
    pub formula_input: Entity<TextInput>,
    pub name_box: Entity<TextInput>,
    pub rename_input: Entity<TextInput>,
    pub find_input: Entity<TextInput>,
    pub editing: Option<Editing>,
    /// The selected chart, picture or shape of the current sheet; when
    /// several are selected, the one selected last.
    pub object_sel: Option<ObjectId>,
    /// The other selected objects (see `objects.rs`).
    pub object_more: Vec<ObjectId>,
    /// The object put aside by Copy / Cut: its feature and its data.
    pub(crate) object_clip: Option<(usize, Rc<dyn Any>)>,
    pub clip: Option<ClipState>,
    pub(crate) drag: Option<Drag>,
    pub layout: Option<GridLayout>,
    /// Areas of the grid features made sensitive, as last painted.
    pub zones: Vec<Zone>,
    /// Tooltip of the zone under the pointer, once it has rested there.
    pub(crate) zone_tip: Option<(Point<Pixels>, SharedString)>,
    zone_tip_task: Option<Task<()>>,
    /// Feature owning the zone under the pointer.
    hover_owner: Option<usize>,
    pub ribbon_tab: usize,
    /// The contextual tab shown instead of `ribbon_tab`, while it exists.
    pub context_tab: Option<&'static str>,
    pub popup: Option<Popup>,
    /// Anchor of the pop-up the current mouse press just closed: clicking a
    /// control again while its menu is open closes it instead of reopening.
    pub popup_dismissed: Option<&'static str>,
    /// The ribbon control whose command is running (its drop-down hangs
    /// from it, see `open_command_menu`).
    pub command_anchor: Option<&'static str>,
    /// Open dialogs, the one in front last.
    pub(crate) modals: Vec<ModalEntry>,
    /// Panes docked right of the grid; `active_pane` is the one shown.
    pub(crate) panes: Vec<PaneEntry>,
    pub(crate) active_pane: usize,
    pub(crate) pane_width: f32,
    /// Pane border being dragged: pointer x and width at the start.
    pub(crate) pane_resize: Option<(f32, f32)>,
    /// Range being picked for a dialog.
    pub pick: Option<RangePick>,
    /// State features keep between events, by type.
    ext: HashMap<TypeId, Box<dyn Any>>,
    /// Selection last reported to the features.
    last_selection: Option<(usize, Selection)>,
    /// Bounds of ribbon controls, used to anchor their drop-downs.
    pub anchors: Rc<RefCell<HashMap<&'static str, Bounds<Pixels>>>>,
    pub hover: Hit,
    pub status_message: Option<SharedString>,
    pub renaming_sheet: Option<usize>,
    pub last_fill: Option<Color>,
    pub last_font_color: Option<Color>,
    pub last_border: BorderPreset,
    pub show_formula_bar: bool,
    pub show_find: bool,
    /// Column where a run of Tab entries started (Enter returns below it).
    tab_origin: Option<u32>,
    /// Formats waiting to be applied by the format painter.
    pub painter: Option<Clip>,
    /// Coloured ranges of the formula being edited, on the visible sheet.
    pub ref_highlights: Vec<(Rect, u32)>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for ExcelApp {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.grid_focus.clone()
    }
}

fn dir_delta(dir: Direction) -> (i64, i64) {
    match dir {
        Direction::Up => (-1, 0),
        Direction::Down => (1, 0),
        Direction::Left => (0, -1),
        Direction::Right => (0, 1),
    }
}

impl ExcelApp {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut wb = Workbook::new(crate::i18n::lang().locale());
        features::preferences::apply_to_workbook(&mut wb);
        let fonts = Rc::new(FontResolver::new(cx.text_system().clone()));
        let cell_editor = cx.new(TextInput::new);
        let formula_input = cx.new(TextInput::new);
        let name_box = cx.new(|cx| {
            let mut input = TextInput::new(cx);
            input.select_on_focus = true;
            input
        });
        let rename_input = cx.new(TextInput::new);
        let find_input = cx.new(|cx| {
            let mut input = TextInput::new(cx);
            input.placeholder = t("Find", "Rechercher").into();
            input
        });
        let subscriptions = vec![
            cx.subscribe_in(&cell_editor, window, |this, _, ev, window, cx| {
                this.on_editor_event(EditSource::Cell, ev, window, cx)
            }),
            cx.subscribe_in(&formula_input, window, |this, _, ev, window, cx| {
                this.on_editor_event(EditSource::FormulaBar, ev, window, cx)
            }),
            cx.subscribe_in(&name_box, window, Self::on_name_box_event),
            cx.subscribe_in(&rename_input, window, Self::on_rename_event),
            cx.subscribe_in(&find_input, window, Self::on_find_event),
        ];
        let grid_focus = cx.focus_handle();
        window.focus(&grid_focus);
        // Closing the window asks before throwing away unsaved changes.
        let weak = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            let Some(app) = weak.upgrade() else {
                return true;
            };
            if !app.read(cx).wb.modified {
                return true;
            }
            app.update(cx, |app, cx| {
                app.confirm_discard(window, cx, |_, window, _| window.remove_window());
            });
            false
        });
        let mut app = ExcelApp {
            title: format!("{}1", t("Book", "Classeur")),
            wb,
            path: None,
            sheet: 0,
            views: vec![SheetView::default()],
            zoom: 1.0,
            fonts,
            grid_focus,
            cell_editor,
            formula_input,
            name_box,
            rename_input,
            find_input,
            editing: None,
            object_sel: None,
            object_more: Vec::new(),
            object_clip: None,
            clip: None,
            drag: None,
            layout: None,
            zones: Vec::new(),
            zone_tip: None,
            zone_tip_task: None,
            hover_owner: None,
            ribbon_tab: 1,
            context_tab: None,
            popup: None,
            popup_dismissed: None,
            command_anchor: None,
            modals: Vec::new(),
            panes: Vec::new(),
            active_pane: 0,
            pane_width: 320.0,
            pane_resize: None,
            pick: None,
            ext: HashMap::new(),
            last_selection: None,
            anchors: Rc::new(RefCell::new(HashMap::new())),
            hover: Hit::None,
            status_message: None,
            renaming_sheet: None,
            last_fill: Some(Color(0xFFFF00)),
            last_font_color: Some(Color(0xFF0000)),
            last_border: BorderPreset::Bottom,
            show_formula_bar: true,
            show_find: false,
            tab_origin: None,
            painter: None,
            ref_highlights: Vec::new(),
            _subscriptions: subscriptions,
        };
        app.refresh(cx);
        app
    }

    /// Replace the workbook (New / Open).
    pub fn set_workbook(
        &mut self,
        wb: Workbook,
        path: Option<PathBuf>,
        title: String,
        cx: &mut Context<Self>,
    ) {
        self.views = wb.sheets.iter().map(|_| SheetView::default()).collect();
        // A1 inside a merged area selects the whole area, as a click on it.
        for (view, sheet) in self.views.iter_mut().zip(&wb.sheets) {
            if let Some(merge) = sheet.merges.iter().find(|m| m.contains(0, 0)) {
                view.sel.ranges = vec![*merge];
            }
        }
        self.wb = wb;
        features::preferences::apply_to_workbook(&mut self.wb);
        self.path = path;
        self.title = title;
        self.sheet = 0;
        self.editing = None;
        self.object_sel = None;
        self.object_more.clear();
        self.object_clip = None;
        self.context_tab = None;
        self.clip = None;
        self.drag = None;
        self.popup = None;
        self.pick = None;
        // What features remembered belonged to the previous workbook.
        self.ext.clear();
        self.panes.clear();
        self.zones.clear();
        self.last_selection = None;
        self.ref_highlights.clear();
        self.refresh(cx);
    }

    // ---- state of the features -----------------------------------------------

    /// What a feature keeps between events (`None` until it first asked for
    /// it with `ext_mut`).
    pub fn ext<T: 'static>(&self) -> Option<&T> {
        self.ext
            .get(&TypeId::of::<T>())
            .and_then(|b| b.downcast_ref::<T>())
    }

    /// The state of a feature, created on first use. It is dropped when
    /// another workbook is opened.
    pub fn ext_mut<T: Default + 'static>(&mut self) -> &mut T {
        self.ext
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(T::default()))
            .downcast_mut::<T>()
            .expect("the entry is keyed by its own type")
    }

    /// Ask the features whether a change may be made; when one of them
    /// refuses, its message is shown and the answer is no.
    pub fn may(&mut self, intent: Intent, cx: &mut Context<Self>) -> bool {
        match features::guard(self, &intent) {
            None => true,
            Some(message) => {
                // A range with a password is opened rather than refused.
                if !features::protection::password_wanted(self, &intent, cx) {
                    self.alert(Alert::new("OpenSheet", message).warning(), cx);
                }
                false
            }
        }
    }

    /// Like `may`, for every range of the selection.
    pub(crate) fn may_all(
        &mut self,
        intent: impl Fn(Rect) -> Intent,
        cx: &mut Context<Self>,
    ) -> bool {
        let ranges = self.sel().ranges.clone();
        ranges.into_iter().all(|r| self.may(intent(r), cx))
    }

    pub fn view_mode(&self) -> ViewMode {
        self.views[self.sheet].mode
    }

    /// Switch the current sheet between Normal, Page Layout and Page Break
    /// Preview.
    pub fn set_view_mode(&mut self, mode: ViewMode, cx: &mut Context<Self>) {
        if self.view_mode() != mode {
            self.views[self.sheet].mode = mode;
            features::view_mode_changed(self, cx);
        }
        self.status_message = None;
        cx.notify();
    }

    // ---- small accessors --------------------------------------------------

    pub fn sel(&self) -> &Selection {
        &self.views[self.sheet].sel
    }

    fn sel_mut(&mut self) -> &mut Selection {
        &mut self.views[self.sheet].sel
    }

    pub fn active(&self) -> (u32, u32) {
        self.sel().active
    }

    pub fn active_style(&self) -> &Style {
        let (r, c) = self.active();
        self.wb.style_at(self.sheet, r, c)
    }

    /// True while a dialog is open: the workbook behind it does not react.
    pub fn modal_open(&self) -> bool {
        !self.modals.is_empty()
    }

    pub fn is_editing(&self) -> bool {
        self.editing.is_some()
    }

    fn sync_views(&mut self) {
        let n = self.wb.sheets.len();
        while self.views.len() < n {
            self.views.push(SheetView::default());
        }
        self.views.truncate(n.max(1));
        if self.sheet >= n {
            self.sheet = n - 1;
        }
        // A hidden sheet cannot be in front (it was hidden, or an undo
        // took its tab away): the nearest tab takes its place.
        self.sheet = self.wb.nearest_visible_sheet(self.sheet);
    }

    /// Bring derived UI state (formula bar, name box) back in line after the
    /// selection or the data changed.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.sync_views();
        if self.editing.is_none() {
            let (r, c) = self.active();
            // Nothing for a formula hidden on a protected sheet.
            let text = self.wb.formula_bar_text(self.sheet, r, c);
            self.formula_input.update(cx, |input, cx| {
                if input.text() != text {
                    input.set_text(text, cx);
                }
                input.highlights.clear();
                input.navigate_mode = false;
            });
            self.update_name_box(cx);
        }
        // Tell the features when the selection moved.
        let now = (self.sheet, self.sel().clone());
        if self.last_selection.as_ref() != Some(&now) {
            let first = self.last_selection.is_none();
            self.last_selection = Some(now);
            if !first {
                features::selection_changed(self, cx);
            }
        }
        cx.notify();
    }

    fn update_name_box(&mut self, cx: &mut Context<Self>) {
        let sel = self.sel();
        let text = match &self.drag {
            Some(Drag::Select(_)) if !sel.current().is_single() => {
                let cur = sel.current();
                let rows = if cur.is_whole_cols() {
                    MAX_ROWS
                } else {
                    cur.rows()
                };
                let cols = if cur.is_whole_rows() {
                    MAX_COLS
                } else {
                    cur.cols()
                };
                format!(
                    "{}{} x {}{}",
                    rows,
                    t("R", "L"),
                    cols,
                    crate::i18n::t_ctx("column", "C", "C")
                )
            }
            // The name of the selected range, else the active cell.
            _ => features::names::name_box_label(self),
        };
        self.name_box.update(cx, |input, cx| {
            if input.text() != text {
                input.set_text(text, cx);
            }
        });
    }

    pub fn set_status(&mut self, msg: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.status_message = Some(msg.into());
        cx.notify();
    }

    pub fn not_implemented(&mut self, what: &str, cx: &mut Context<Self>) {
        self.set_status(
            format!(
                "{} — {}",
                what,
                t(
                    "not available yet in this version",
                    "pas encore disponible dans cette version"
                )
            ),
            cx,
        );
    }

    pub fn selection_stats(&self) -> SelectionStats {
        self.wb.selection_stats(self.sheet, &self.sel().ranges)
    }

    /// Layout for the current sheet using the last painted bounds.
    pub fn make_layout(&self, bounds: Bounds<Pixels>) -> GridLayout {
        let view = &self.views[self.sheet];
        let mut layout = GridLayout::new(
            &self.wb.sheets[self.sheet],
            bounds,
            self.zoom,
            view.scroll_x,
            view.scroll_y,
        );
        features::adjust_layout(self, &mut layout);
        layout
    }

    /// The layout of the grid as it is on screen (`None` before the first
    /// paint).
    pub fn current_layout(&self) -> Option<GridLayout> {
        self.layout.as_ref().map(|l| self.make_layout(l.bounds))
    }

    /// Window position of the bottom-left corner of a range, where a
    /// drop-down hanging from it opens.
    pub fn cell_anchor(&self, rect: Rect) -> Point<Pixels> {
        match self.current_layout() {
            Some(layout) => {
                let (x0, _, _, y1) = layout.rect_px(rect);
                layout.to_window(x0, y1)
            }
            None => Point::default(),
        }
    }

    /// Pixel size of the scrollable content (used range plus one screen).
    pub fn content_size(&self, layout: &GridLayout) -> (f64, f64) {
        let (er, ec) = self.wb.sheets[self.sheet].content_extent();
        let sel = self.sel();
        let er = er.max(sel.active.0);
        let ec = ec.max(sel.active.1);
        (
            layout.cols.end(ec.min(MAX_COLS - 1)) + layout.view_w() as f64,
            layout.rows.end(er.min(MAX_ROWS - 1)) + layout.view_h() as f64,
        )
    }

    // ---- selection ----------------------------------------------------------

    pub(crate) fn expand_merges(&self, mut rect: Rect) -> Rect {
        let sheet = &self.wb.sheets[self.sheet];
        if sheet.merges.is_empty() {
            return rect;
        }
        loop {
            let mut grown = false;
            for m in &sheet.merges {
                if m.intersects(&rect) {
                    let u = rect.union(m);
                    if u != rect {
                        rect = u;
                        grown = true;
                    }
                }
            }
            if !grown {
                return rect;
            }
        }
    }

    pub fn select_cell(&mut self, row: u32, col: u32, cx: &mut Context<Self>) {
        let row = row.min(MAX_ROWS - 1);
        let col = col.min(MAX_COLS - 1);
        if !features::can_select(self, row, col) {
            return;
        }
        self.deselect_object(cx);
        let rect = self.expand_merges(Rect::cell(row, col));
        let sel = self.sel_mut();
        sel.active = (rect.r0, rect.c0);
        sel.anchor = (row, col);
        sel.ranges = vec![rect];
        self.ensure_visible(row, col);
        self.refresh(cx);
    }

    pub fn select_rect(&mut self, rect: Rect, cx: &mut Context<Self>) {
        self.deselect_object(cx);
        let rect = self.expand_merges(rect);
        let sel = self.sel_mut();
        sel.active = (rect.r0, rect.c0);
        sel.anchor = (rect.r0, rect.c0);
        sel.ranges = vec![rect];
        self.ensure_visible(rect.r0, rect.c0);
        self.refresh(cx);
    }

    /// Extend the current range from the anchor to (row, col).
    fn extend_to(&mut self, row: u32, col: u32, cx: &mut Context<Self>) {
        if !features::can_select(self, row, col) {
            return;
        }
        let anchor = self.sel().anchor;
        let rect = self.expand_merges(Rect::new(anchor.0, anchor.1, row, col));
        self.sel_mut().set_current(rect);
        self.ensure_visible(row, col);
        self.refresh(cx);
    }

    /// The corner of the current range that moves with Shift+arrows.
    fn lead(&self) -> (u32, u32) {
        let sel = self.sel();
        let cur = sel.current();
        let row = if sel.anchor.0 <= cur.r0 {
            cur.r1
        } else {
            cur.r0
        };
        let col = if sel.anchor.1 <= cur.c0 {
            cur.c1
        } else {
            cur.c0
        };
        (row, col)
    }

    pub fn ensure_visible(&mut self, row: u32, col: u32) {
        if let Some(layout) = self.current_layout() {
            let (sx, sy) = layout.scroll_to_show(row, col);
            let view = &mut self.views[self.sheet];
            view.scroll_x = sx;
            view.scroll_y = sy;
        }
    }

    /// Step from a cell in a direction, skipping hidden rows / columns and
    /// treating merged areas as one cell.
    fn step(&self, from: (u32, u32), dir: Direction) -> (u32, u32) {
        let sheet = &self.wb.sheets[self.sheet];
        let (mut r, mut c) = from;
        if let Some(m) = sheet.merge_at(r, c) {
            match dir {
                Direction::Up => r = m.r0,
                Direction::Down => r = m.r1,
                Direction::Left => c = m.c0,
                Direction::Right => c = m.c1,
            }
        }
        let (dr, dc) = dir_delta(dir);
        loop {
            let nr = r as i64 + dr;
            let nc = c as i64 + dc;
            if nr < 0 || nc < 0 || nr >= MAX_ROWS as i64 || nc >= MAX_COLS as i64 {
                return from;
            }
            r = nr as u32;
            c = nc as u32;
            let hidden =
                (dr != 0 && sheet.is_row_hidden(r)) || (dc != 0 && sheet.hidden_cols.contains(&c));
            if !hidden {
                return (r, c);
            }
        }
    }

    pub fn move_selection(&mut self, dir: Direction, extend: bool, cx: &mut Context<Self>) {
        if dir == Direction::Up || dir == Direction::Left {
            self.tab_origin = None;
        }
        if extend {
            let lead = self.lead();
            let (r, c) = self.step(lead, dir);
            self.extend_to(r, c, cx);
        } else {
            let from = self.active();
            let (r, c) =
                features::next_cell(self, from, dir, false).unwrap_or_else(|| self.step(from, dir));
            self.select_cell(r, c, cx);
        }
    }

    fn jump_selection(&mut self, dir: Direction, extend: bool, cx: &mut Context<Self>) {
        let from = if extend { self.lead() } else { self.active() };
        let (r, c) = self.wb.data_edge(self.sheet, from.0, from.1, dir);
        if extend {
            self.extend_to(r, c, cx);
        } else {
            self.select_cell(r, c, cx);
        }
    }

    /// Move the active cell after Enter / Tab: inside a multi-cell selection
    /// it cycles through the selection, otherwise it moves the selection.
    fn advance(&mut self, dir: Direction, cx: &mut Context<Self>) {
        // Tab goes through a table row after row.
        if features::tables::advance(self, dir, cx) {
            return;
        }
        let sel = self.sel().clone();
        let cur = sel.current();
        let merged = self.wb.sheets[self.sheet].merge_at(sel.active.0, sel.active.1) == Some(cur);
        if sel.ranges.len() == 1 && (cur.is_single() || merged) {
            // A feature may know better where an entry leads (Tab on a
            // protected sheet).
            match features::next_cell(self, sel.active, dir, true) {
                Some((r, c)) => self.select_cell(r, c, cx),
                None => self.move_selection(dir, false, cx),
            }
            return;
        }
        let cur = self.wb.clamp_to_used(self.sheet, cur);
        let (mut r, mut c) = sel.active;
        match dir {
            Direction::Down => {
                if r < cur.r1 {
                    r += 1;
                } else {
                    r = cur.r0;
                    c = if c < cur.c1 { c + 1 } else { cur.c0 };
                }
            }
            Direction::Up => {
                if r > cur.r0 {
                    r -= 1;
                } else {
                    r = cur.r1;
                    c = if c > cur.c0 { c - 1 } else { cur.c1 };
                }
            }
            Direction::Right => {
                if c < cur.c1 {
                    c += 1;
                } else {
                    c = cur.c0;
                    r = if r < cur.r1 { r + 1 } else { cur.r0 };
                }
            }
            Direction::Left => {
                if c > cur.c0 {
                    c -= 1;
                } else {
                    c = cur.c1;
                    r = if r > cur.r0 { r - 1 } else { cur.r1 };
                }
            }
        }
        self.sel_mut().active = (r, c);
        self.ensure_visible(r, c);
        self.refresh(cx);
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        let active = self.active();
        let sel = self.sel_mut();
        sel.ranges = vec![Rect::new(0, 0, MAX_ROWS - 1, MAX_COLS - 1)];
        sel.active = active;
        sel.anchor = (0, 0);
        self.refresh(cx);
    }

    pub fn switch_sheet(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.wb.sheets.len()
            || index == self.sheet
            || !self.wb.sheets[index].is_visible()
        {
            return;
        }
        if self.editing.is_some() && !self.commit_edit(None, window, cx) {
            return;
        }
        self.deselect_object(cx);
        self.sheet = index;
        self.popup = None;
        self.refresh(cx);
    }

    /// What Return does in the name box: go to the reference, the name or
    /// the table typed there, or give a new name to the selection.
    pub fn go_to(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        // A table name or a structured reference is the tables feature's;
        // references, names and new names are the names feature's.
        features::tables::go_to(self, text, cx) || features::names::name_box_entered(self, text, cx)
    }

    // ---- editing ------------------------------------------------------------

    pub(crate) fn editor(&self) -> &Entity<TextInput> {
        match self.editing.as_ref().map(|e| e.source) {
            Some(EditSource::FormulaBar) => &self.formula_input,
            _ => &self.cell_editor,
        }
    }

    /// Put the keyboard focus (back) into the field being edited.
    pub fn focus_editor(&self, window: &mut Window, cx: &App) {
        let handle = self.editor().read(cx).focus_handle(cx);
        window.focus(&handle);
    }

    /// While a cell is being edited, a key that reached the grid (the
    /// editor lost focus) is handled as if typed in the editor. Returns
    /// true when the key was consumed.
    fn editing_key(
        &mut self,
        dir: Direction,
        extend: bool,
        commit: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.editing.is_none() {
            return false;
        }
        if !commit && self.can_point(cx) {
            self.point_step(dir, extend, window, cx);
            self.focus_editor(window, cx);
        } else {
            self.commit_edit(Some(dir), window, cx);
        }
        true
    }

    pub fn edit_text(&self, cx: &App) -> String {
        self.editor().read(cx).text().to_string()
    }

    /// Begin editing the active cell. `initial` replaces the content (typing
    /// over a cell); `None` edits the existing content.
    pub fn start_edit(
        &mut self,
        initial: Option<&str>,
        mode: EditMode,
        source: EditSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing.is_some() || self.modal_open() {
            return;
        }
        let (row, col) = self.active();
        // A protected cell or a pivot table refuses before anything is typed.
        if !self.may(Intent::EditCells(Rect::cell(row, col)), cx) {
            return;
        }
        let text = match initial {
            Some(s) => s.to_string(),
            // Like the formula bar, the cell does not show a hidden formula.
            None => self.wb.formula_bar_text(self.sheet, row, col),
        };
        self.ensure_visible(row, col);
        self.editing = Some(Editing {
            sheet: self.sheet,
            row,
            col,
            mode,
            source,
            point: None,
        });
        self.popup = None;
        self.status_message = None;
        let navigate = mode == EditMode::Enter;
        for input in [&self.cell_editor, &self.formula_input] {
            input.update(cx, |input, cx| {
                input.set_text(text.clone(), cx);
                input.navigate_mode = navigate;
            });
        }
        let handle = self.editor().read(cx).focus_handle(cx);
        window.focus(&handle);
        self.update_ref_highlights(cx);
        cx.notify();
    }

    fn sync_other_editor(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = &self.editing else {
            return;
        };
        let text = self.editor().read(cx).text().to_string();
        let other = match edit.source {
            EditSource::Cell => &self.formula_input,
            EditSource::FormulaBar => &self.cell_editor,
        };
        other.update(cx, |input, cx| input.set_text(text, cx));
    }

    fn update_ref_highlights(&mut self, cx: &mut Context<Self>) {
        self.ref_highlights.clear();
        let Some(edit) = &self.editing else {
            return;
        };
        let text = self.editor().read(cx).text().to_string();
        let mut highlights: Vec<(Range<usize>, gpui::Hsla)> = Vec::new();
        if let Some(body) = text.strip_prefix('=') {
            // Structured references, names standing for a range and `A1#`
            // are coloured with the cells they cover.
            let spans = parser::scan_refs_in(body, &self.wb, edit.sheet, edit.row, edit.col);
            let mut seen: Vec<String> = Vec::new();
            for span in spans {
                let key = span.reference.to_text();
                let idx = match seen.iter().position(|k| *k == key) {
                    Some(i) => i,
                    None => {
                        seen.push(key);
                        seen.len() - 1
                    }
                };
                let color = theme::REF_COLORS[idx % theme::REF_COLORS.len()];
                highlights.push((span.range.start + 1..span.range.end + 1, theme::c(color)));
                let target = match &span.reference.sheet {
                    None => Some(edit.sheet),
                    Some(name) => self.wb.sheet_index(name),
                };
                if target == Some(self.sheet) {
                    let rect = span.reference.rect();
                    if !self.ref_highlights.iter().any(|(r, _)| *r == rect) {
                        self.ref_highlights.push((rect, color));
                    }
                }
            }
        }
        for input in [&self.cell_editor, &self.formula_input] {
            let h = highlights.clone();
            input.update(cx, |input, cx| {
                input.highlights = h;
                cx.notify();
            });
        }
    }

    fn on_editor_event(
        &mut self,
        source: EditSource,
        ev: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match ev {
            InputEvent::Focused => {
                // Clicking into the formula bar starts editing the cell.
                if source == EditSource::FormulaBar && self.editing.is_none() {
                    let text = self.formula_input.read(cx).text().to_string();
                    let (row, col) = self.active();
                    // A protected cell refuses here as it does in the grid.
                    if !self.may(Intent::EditCells(Rect::cell(row, col)), cx) {
                        window.focus(&self.grid_focus);
                        return;
                    }
                    self.editing = Some(Editing {
                        sheet: self.sheet,
                        row,
                        col,
                        mode: EditMode::Edit,
                        source,
                        point: None,
                    });
                    self.cell_editor
                        .update(cx, |input, cx| input.set_text(text, cx));
                    self.update_ref_highlights(cx);
                    cx.notify();
                } else if let Some(edit) = &mut self.editing {
                    if edit.source != source {
                        edit.source = source;
                        edit.mode = EditMode::Edit;
                        cx.notify();
                    }
                }
            }
            InputEvent::Blurred => {}
            InputEvent::Clicked => {
                // Placing the caret with the mouse leaves "Enter" mode:
                // the arrows now move inside the text.
                if let Some(edit) = &mut self.editing {
                    if edit.mode != EditMode::Edit {
                        edit.mode = EditMode::Edit;
                        cx.notify();
                    }
                }
            }
            InputEvent::Changed => {
                if let Some(edit) = &mut self.editing {
                    if edit.source == source {
                        edit.point = None;
                        features::preferences::autocorrect::text_typed(self, cx);
                        self.sync_other_editor(cx);
                        self.update_ref_highlights(cx);
                        cx.notify();
                    }
                }
            }
            InputEvent::Enter { shift } => {
                // Where Return leads is a preference; it may lead nowhere.
                let dir = features::preferences::enter_direction(*shift);
                let origin = self.tab_origin.take();
                if self.commit_edit(dir, window, cx) {
                    // After a run of Tab entries, Enter goes back under the
                    // first one.
                    if let (Some(col), false, true) = (origin, *shift, self.sel().is_single_cell())
                    {
                        let (r, _) = self.active();
                        self.select_cell(r, col, cx);
                    }
                }
            }
            InputEvent::Tab { shift } => {
                let dir = if *shift {
                    Direction::Left
                } else {
                    Direction::Right
                };
                let col = self.editing.as_ref().map(|e| e.col);
                let origin = self.tab_origin.or(col);
                features::tables::tab_pressed(self);
                if self.commit_edit(Some(dir), window, cx) {
                    self.tab_origin = origin;
                }
            }
            InputEvent::Escape => self.cancel_edit(window, cx),
            InputEvent::Navigate { key, shift } => {
                let dir = match key {
                    NavKey::Left => Direction::Left,
                    NavKey::Right => Direction::Right,
                    NavKey::Up => Direction::Up,
                    NavKey::Down => Direction::Down,
                };
                let Some(edit) = &self.editing else {
                    return;
                };
                if edit.mode == EditMode::Edit {
                    return;
                }
                if self.can_point(cx) {
                    self.point_step(dir, *shift, window, cx);
                } else {
                    self.commit_edit(Some(dir), window, cx);
                }
            }
        }
    }

    /// True when a click on a cell should insert a reference into the
    /// formula being typed rather than end the edit.
    fn can_point(&self, cx: &App) -> bool {
        let Some(edit) = &self.editing else {
            return false;
        };
        if edit.point.is_some() {
            return true;
        }
        let input = self.editor().read(cx);
        let text = input.text();
        if !(text.starts_with('=') || text.starts_with('+') || text.starts_with('-')) {
            return false;
        }
        let cursor = input.selection().start.min(text.len());
        let before = text[..cursor].trim_end();
        match before.chars().last() {
            None => false,
            Some(ch) => "=+-*/^&(,;<>:".contains(ch),
        }
    }

    /// Insert (or replace) the pointed reference in the formula.
    fn point_at(
        &mut self,
        anchor: (u32, u32),
        lead: (u32, u32),
        kind: SelectKind,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = &self.editing else {
            return;
        };
        let rect = match kind {
            SelectKind::Cells => self.expand_merges(Rect::new(anchor.0, anchor.1, lead.0, lead.1)),
            SelectKind::Cols => Rect::new(0, anchor.1, MAX_ROWS - 1, lead.1),
            SelectKind::Rows => Rect::new(anchor.0, 0, lead.0, MAX_COLS - 1),
        };
        let is_merge = self.wb.sheets[self.sheet].merge_at(rect.r0, rect.c0) == Some(rect);
        let mut text = String::new();
        if edit.sheet != self.sheet {
            opensheet_engine::ast::write_sheet_name(&self.wb.sheets[self.sheet].name, &mut text);
            text.push('!');
        }
        if is_merge {
            text.push_str(&cell_name(rect.r0, rect.c0));
        } else {
            text.push_str(&rect.name());
        }
        // Cells of a table are written as Excel does: `Table1[Sales]`.
        if kind == SelectKind::Cells {
            if let Some(reference) = features::tables::pointed_reference(self, rect) {
                text = reference;
            }
        }
        let editor = self.editor().clone();
        let range = match &edit.point {
            Some(p) => p.range.clone(),
            None => editor.read(cx).selection(),
        };
        let start = range.start;
        editor.update(cx, |input, cx| input.replace_range(range, &text, cx));
        if let Some(edit) = &mut self.editing {
            edit.point = Some(PointState {
                range: start..start + text.len(),
                anchor,
                lead,
            });
        }
        self.sync_other_editor(cx);
        self.update_ref_highlights(cx);
        self.ensure_visible(lead.0, lead.1);
        cx.notify();
    }

    fn point_step(
        &mut self,
        dir: Direction,
        extend: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = &self.editing else {
            return;
        };
        let (anchor, lead) = match &edit.point {
            Some(p) => (p.anchor, p.lead),
            None => ((edit.row, edit.col), (edit.row, edit.col)),
        };
        let next = self.step(lead, dir);
        if extend && edit.point.is_some() {
            self.point_at(anchor, next, SelectKind::Cells, cx);
        } else {
            self.point_at(next, next, SelectKind::Cells, cx);
        }
    }

    /// Store the edited text into the cell. Returns false when the entry was
    /// rejected (invalid formula, data validation) and editing continues.
    pub fn commit_edit(
        &mut self,
        advance: Option<Direction>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(edit) = &self.editing else {
            return true;
        };
        let (sheet, row, col) = (edit.sheet, edit.row, edit.col);
        let text = self.edit_text(cx);
        let pending = PendingEdit {
            sheet,
            row,
            col,
            text: &text,
            advance,
        };
        if features::may_commit(self, &pending, window, cx) == Verdict::Hold {
            cx.notify();
            return false;
        }
        self.commit_edit_unchecked(advance, window, cx)
    }

    /// `commit_edit` without asking the features first: what the "Yes"
    /// button of a validation warning does.
    pub fn commit_edit_unchecked(
        &mut self,
        advance: Option<Direction>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(edit) = &self.editing else {
            return true;
        };
        let (sheet, row, col) = (edit.sheet, edit.row, edit.col);
        let mut text = self.edit_text(cx);
        let mut result = self.wb.set_cell_text(sheet, row, col, &text);
        if result.is_err() && text.starts_with('=') {
            // Like Excel, offer the obvious fix: close open parentheses.
            let open = text.matches('(').count();
            let close = text.matches(')').count();
            if open > close {
                let fixed = format!("{}{}", text, ")".repeat(open - close));
                if self.wb.set_cell_text(sheet, row, col, &fixed).is_ok() {
                    text = fixed;
                    result = Ok(());
                }
            }
        }
        let _ = text;
        if let Err(err) = result {
            self.alert(
                Alert::new(
                    "OpenSheet",
                    format!(
                    "{}\n\n({} {})",
                    t(
                        "There's a problem with this formula.\nNot trying to type a formula? When the first character is an equal (=) or minus (-) sign, it is treated as a formula. To avoid this, type an apostrophe (') first.",
                        "Un problème a été rencontré avec cette formule.\nVous ne souhaitez pas saisir de formule ? Lorsque le premier caractère est le signe égal (=) ou moins (-), il est considéré comme une formule. Pour éviter cela, tapez d'abord une apostrophe (').",
                    ),
                    err.message,
                    err.pos + 1
                    ),
                ),
                cx,
            );
            return false;
        }
        let edit = CellEdit {
            sheet,
            rect: Rect::cell(row, col),
            cause: ChangeCause::Typing,
        };
        features::cells_written(self, &edit, cx);
        self.wb.commit(t("Typing", "Saisie"));
        self.editing = None;
        self.ref_highlights.clear();
        self.clip = None;
        if self.sheet != sheet {
            self.sheet = sheet;
        }
        if !self.modal_open() {
            window.focus(&self.grid_focus);
        }
        features::cells_changed(self, &edit, cx);
        if let Some(dir) = advance {
            self.advance(dir, cx);
        }
        self.refresh(cx);
        true
    }

    pub fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(edit) = self.editing.take() {
            self.sheet = edit.sheet;
        }
        self.ref_highlights.clear();
        if !self.modal_open() {
            window.focus(&self.grid_focus);
        }
        self.refresh(cx);
    }

    // ---- name box / rename / find events ----------------------------------

    fn on_name_box_event(
        &mut self,
        _: &Entity<TextInput>,
        ev: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match ev {
            InputEvent::Enter { .. } => {
                let text = self.name_box.read(cx).text().to_string();
                if !self.go_to(&text, cx) {
                    self.update_name_box(cx);
                }
                window.focus(&self.grid_focus);
            }
            InputEvent::Escape | InputEvent::Tab { .. } => {
                self.update_name_box(cx);
                window.focus(&self.grid_focus);
            }
            InputEvent::Blurred => self.update_name_box(cx),
            _ => {}
        }
    }

    pub fn begin_rename_sheet(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if index >= self.wb.sheets.len() || !self.may(Intent::RenameSheet, cx) {
            return;
        }
        self.renaming_sheet = Some(index);
        let name = self.wb.sheets[index].name.clone();
        self.rename_input.update(cx, |input, cx| {
            input.set_text(name, cx);
            input.select_all_text(cx);
        });
        let handle = self.rename_input.read(cx).focus_handle(cx);
        window.focus(&handle);
        cx.notify();
    }

    fn on_rename_event(
        &mut self,
        _: &Entity<TextInput>,
        ev: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.renaming_sheet else {
            return;
        };
        match ev {
            InputEvent::Enter { .. } | InputEvent::Tab { .. } | InputEvent::Blurred => {
                let name = self.rename_input.read(cx).text().to_string();
                self.renaming_sheet = None;
                if self.wb.rename_sheet(index, &name) {
                    self.wb.commit(t("Rename Sheet", "Renommer la feuille"));
                } else if !name.trim().is_empty() {
                    self.set_status(
                        t(
                            "That name is already taken or not valid.",
                            "Ce nom est déjà utilisé ou n'est pas valide.",
                        ),
                        cx,
                    );
                }
                if !matches!(ev, InputEvent::Blurred) {
                    window.focus(&self.grid_focus);
                }
                self.refresh(cx);
            }
            InputEvent::Escape => {
                self.renaming_sheet = None;
                window.focus(&self.grid_focus);
                cx.notify();
            }
            _ => {}
        }
    }

    fn on_find_event(
        &mut self,
        _: &Entity<TextInput>,
        ev: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match ev {
            InputEvent::Enter { .. } => self.find_next(cx),
            InputEvent::Escape => {
                self.show_find = false;
                window.focus(&self.grid_focus);
                cx.notify();
            }
            _ => {}
        }
    }

    pub fn find_next(&mut self, cx: &mut Context<Self>) {
        let needle = self.find_input.read(cx).text().to_string();
        if needle.is_empty() {
            return;
        }
        features::edit_tools::find_bar_next(self, &needle, cx);
    }

    pub fn toggle_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_find = !self.show_find;
        if self.show_find {
            let handle = self.find_input.read(cx).focus_handle(cx);
            window.focus(&handle);
            self.find_input
                .update(cx, |input, cx| input.select_all_text(cx));
        } else {
            window.focus(&self.grid_focus);
        }
        cx.notify();
    }

    // ---- data operations ----------------------------------------------------

    /// Finish a workbook transaction and refresh the view.
    pub fn commit(&mut self, label: &str, cx: &mut Context<Self>) {
        self.wb.commit(label);
        self.refresh(cx);
    }

    /// Finish a transaction that changed cells of the current sheet: the
    /// features first add what belongs to the same undo step, then hear
    /// about the committed change.
    pub fn commit_cells(
        &mut self,
        label: &str,
        rects: &[Rect],
        cause: ChangeCause,
        cx: &mut Context<Self>,
    ) {
        let sheet = self.sheet;
        let edits: Vec<CellEdit> = rects
            .iter()
            .map(|rect| CellEdit {
                sheet,
                rect: *rect,
                cause,
            })
            .collect();
        for edit in &edits {
            features::cells_written(self, edit, cx);
        }
        self.wb.commit(label);
        for edit in &edits {
            features::cells_changed(self, edit, cx);
        }
        self.refresh(cx);
    }

    pub fn undo(&mut self, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            // While typing, Undo takes back the last automatic correction.
            if features::preferences::autocorrect::undo_correction(self, cx) {
                self.sync_other_editor(cx);
                cx.notify();
            }
            return;
        }
        self.wb.undo();
        self.clip = None;
        // An object the step took away is no longer selected.
        self.prune_object_selection(cx);
        self.refresh(cx);
    }

    pub fn redo(&mut self, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            return;
        }
        self.wb.redo();
        self.clip = None;
        // An object the step took away is no longer selected.
        self.prune_object_selection(cx);
        self.refresh(cx);
    }

    pub fn clear(&mut self, mode: ClearMode, cx: &mut Context<Self>) {
        let allowed = match mode {
            ClearMode::Contents => self.may_all(Intent::EditCells, cx),
            ClearMode::Formats => self.may_all(Intent::FormatCells, cx),
            ClearMode::Comments => self.may(Intent::EditComment, cx),
            // Clear All takes the formats away with the contents.
            ClearMode::All => {
                self.may_all(Intent::EditCells, cx) && self.may_all(Intent::FormatCells, cx)
            }
        };
        if !allowed {
            return;
        }
        let ranges = self.sel().ranges.clone();
        for r in &ranges {
            self.wb.clear(self.sheet, *r, mode);
        }
        self.commit_cells(t("Clear", "Effacer"), &ranges, ChangeCause::Clear(mode), cx);
    }

    pub fn copy(&mut self, cut: bool, cx: &mut Context<Self>) {
        if self.object_sel.is_some() {
            self.copy_object(cut, cx);
            return;
        }
        if cut && !self.may_all(Intent::EditCells, cx) {
            return;
        }
        self.object_clip = None;
        if self.sel().ranges.len() > 1 {
            self.set_status(
                t(
                    "This action won't work on multiple selections.",
                    "Cette action ne peut pas être effectuée sur des sélections multiples.",
                ),
                cx,
            );
            return;
        }
        let clip = self.wb.copy(self.sheet, self.sel().current());
        cx.write_to_clipboard(ClipboardItem::new_string(clip.text.clone()));
        self.clip = Some(ClipState { clip, cut });
        cx.notify();
    }

    pub fn paste(&mut self, mode: PasteMode, cx: &mut Context<Self>) {
        // A picture copied in another application.
        if mode == PasteMode::All && features::paste_clipboard(self, cx) {
            return;
        }
        if self.object_clip.is_some() {
            self.paste_object(cx);
            return;
        }
        let system = cx.read_from_clipboard().and_then(|item| item.text());
        let dest = self.sel().current();
        let internal = match (&self.clip, &system) {
            (Some(state), Some(text)) => state.clip.text == *text,
            (Some(_), None) => true,
            _ => false,
        };
        // A paste may cover more cells than are selected: ask for them all.
        let targets = self.paste_targets(dest, internal, system.as_deref(), mode);
        let intent = |target: Rect| match mode {
            PasteMode::Formats => Intent::FormatCells(target),
            _ => Intent::EditCells(target),
        };
        if !targets
            .into_iter()
            .all(|target| self.may(intent(target), cx))
        {
            return;
        }
        // Where the cells come from, for the features that follow them
        // (comments, validation, conditional formats).
        let mut from = None;
        let mut cut = false;
        let written = if internal {
            let state = self.clip.take().unwrap();
            from = Some((state.clip.sheet, state.clip.rect));
            if state.cut && mode == PasteMode::All {
                cut = true;
                let out = self.wb.move_range(
                    state.clip.sheet,
                    state.clip.rect,
                    self.sheet,
                    dest.r0,
                    dest.c0,
                );
                Some(out)
            } else {
                let out = self.wb.paste(self.sheet, dest, &state.clip, mode);
                self.clip = Some(state);
                Some(out)
            }
        } else if let Some(text) = system {
            Some(self.wb.paste_text(self.sheet, dest.r0, dest.c0, &text))
        } else {
            None
        };
        if let Some(rect) = written {
            let active = (rect.r0, rect.c0);
            let sel = self.sel_mut();
            sel.ranges = vec![rect];
            sel.active = active;
            sel.anchor = active;
            self.commit_cells(
                t("Paste", "Coller"),
                &[rect],
                ChangeCause::Paste { from, cut, mode },
                cx,
            );
        }
    }

    /// The cells a paste at `dest` will write: the block on the clipboard,
    /// repeated when the selection is a multiple of it, without the rows a
    /// filter hides.
    fn paste_targets(
        &self,
        dest: Rect,
        internal: bool,
        text: Option<&str>,
        mode: PasteMode,
    ) -> Vec<Rect> {
        let block = |rows: u32, cols: u32| {
            vec![Rect::new(
                dest.r0,
                dest.c0,
                (dest.r0 + rows.max(1) - 1).min(MAX_ROWS - 1),
                (dest.c0 + cols.max(1) - 1).min(MAX_COLS - 1),
            )]
        };
        match (&self.clip, text) {
            (Some(state), _) if internal && state.cut && mode == PasteMode::All => {
                let src = self.wb.clamp_to_used(state.clip.sheet, state.clip.rect);
                block(src.rows(), src.cols())
            }
            (Some(state), _) if internal => self.wb.paste_blocks(self.sheet, dest, &state.clip),
            (_, Some(text)) => {
                let lines = opensheet_engine::ops::parse_tsv(text);
                let widest = lines.iter().map(Vec::len).max().unwrap_or(1);
                block(lines.len() as u32, widest as u32)
            }
            _ => vec![dest],
        }
    }

    /// Apply a formatting change to the whole selection.
    pub fn apply_style(&mut self, label: &str, cx: &mut Context<Self>, f: impl Fn(&mut Style)) {
        // A selected shape takes the change for its own text.
        if let Some(id) = self.object_sel {
            if features::all()[id.owner].object_style(self, id.id, label, &f, cx) {
                return;
            }
        }
        if !self.may_all(Intent::FormatCells, cx) {
            return;
        }
        let ranges = self.sel().ranges.clone();
        for r in ranges {
            self.wb.update_style(self.sheet, r, &f);
        }
        self.commit(label, cx);
    }

    pub fn toggle_bold(&mut self, cx: &mut Context<Self>) {
        let on = !self.active_style().font.bold;
        self.apply_style(t("Bold", "Gras"), cx, move |s| s.font.bold = on);
    }

    pub fn toggle_italic(&mut self, cx: &mut Context<Self>) {
        let on = !self.active_style().font.italic;
        self.apply_style(t("Italic", "Italique"), cx, move |s| s.font.italic = on);
    }

    pub fn toggle_underline(&mut self, cx: &mut Context<Self>) {
        let on = self.active_style().font.underline == Underline::None;
        self.apply_style(t("Underline", "Souligné"), cx, move |s| {
            s.font.underline = if on {
                Underline::Single
            } else {
                Underline::None
            }
        });
    }

    pub fn toggle_strike(&mut self, cx: &mut Context<Self>) {
        let on = !self.active_style().font.strike;
        self.apply_style(t("Strikethrough", "Barré"), cx, move |s| {
            s.font.strike = on
        });
    }

    pub fn set_font_name(&mut self, name: &str, cx: &mut Context<Self>) {
        // "Aptos Narrow (Body)" is the body font of the theme, whatever it
        // becomes; any other name is that font.
        let name: Arc<str> = Arc::from(name);
        let document = self.wb.theme_arc();
        self.apply_style(t("Font", "Police"), cx, move |s| {
            theme::set_font_choice(&mut s.font, &name, &document)
        });
    }

    pub fn set_font_size(&mut self, pt: f32, cx: &mut Context<Self>) {
        let pt = pt.clamp(1.0, 409.0);
        // A selected object takes the size for its own text.
        if let Some(id) = self.object_sel {
            let label = t("Font Size", "Taille de police");
            if features::all()[id.owner].object_style(
                self,
                id.id,
                label,
                &|s| s.font.set_size_pt(pt),
                cx,
            ) {
                return;
            }
        }
        if !self.may_all(Intent::FormatCells, cx) {
            return;
        }
        let ranges = self.sel().ranges.clone();
        for r in &ranges {
            self.wb
                .update_style(self.sheet, *r, &|s| s.font.set_size_pt(pt));
        }
        self.grow_rows_for_font(pt);
        self.commit(t("Font Size", "Taille de police"), cx);
    }

    /// Rows of the selection grow to fit larger text, as Excel's automatic
    /// row height does.
    pub(crate) fn grow_rows_for_font(&mut self, pt: f32) {
        let ranges = self.sel().ranges.clone();
        let needed = (crate::grid::pt_to_px(pt) * 1.25).ceil() + 3.0;
        for r in &ranges {
            let r = self.wb.clamp_to_used(self.sheet, *r);
            if r.rows() > 5000 {
                continue;
            }
            for row in r.r0..=r.r1 {
                // Hidden rows (height 0) stay as they are.
                let cur = self.wb.sheets[self.sheet].row_height(row);
                if cur > 0.0 && needed > cur {
                    self.wb.set_row_height(self.sheet, row, Some(needed));
                }
            }
        }
    }

    pub const FONT_SIZES: [f32; 16] = [
        8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0, 24.0, 26.0, 28.0, 36.0, 48.0,
        72.0,
    ];

    pub fn step_font_size(&mut self, grow: bool, cx: &mut Context<Self>) {
        // The text of a selected chart element is what grows and shrinks.
        let cur = features::charts::home_text_size(self)
            .unwrap_or_else(|| self.active_style().font.size_pt());
        let next = if grow {
            Self::FONT_SIZES
                .iter()
                .copied()
                .find(|s| *s > cur + 0.01)
                .unwrap_or((cur + 10.0).min(409.0))
        } else {
            Self::FONT_SIZES
                .iter()
                .rev()
                .copied()
                .find(|s| *s < cur - 0.01)
                .unwrap_or((cur - 1.0).max(1.0))
        };
        self.set_font_size(next, cx);
    }

    pub fn set_h_align(&mut self, align: HAlign, cx: &mut Context<Self>) {
        let align = if self.active_style().h_align == align {
            HAlign::General
        } else {
            align
        };
        self.apply_style(t("Alignment", "Alignement"), cx, move |s| s.h_align = align);
    }

    pub fn set_v_align(&mut self, align: VAlign, cx: &mut Context<Self>) {
        self.apply_style(t("Alignment", "Alignement"), cx, move |s| s.v_align = align);
    }

    pub fn toggle_wrap(&mut self, cx: &mut Context<Self>) {
        let on = !self.active_style().wrap;
        self.apply_style(t("Wrap Text", "Renvoyer à la ligne"), cx, move |s| {
            s.wrap = on
        });
    }

    pub fn change_indent(&mut self, delta: i32, cx: &mut Context<Self>) {
        self.apply_style(t("Indent", "Retrait"), cx, move |s| {
            s.indent = (s.indent as i32 + delta).clamp(0, 250) as u8;
            if s.indent > 0 && s.h_align == HAlign::General {
                s.h_align = HAlign::Left;
            }
        });
    }

    pub fn set_fill(&mut self, color: Option<Color>, cx: &mut Context<Self>) {
        if color.is_some() {
            self.last_fill = color;
        }
        self.apply_style(t("Fill Color", "Couleur de remplissage"), cx, move |s| {
            s.fill = color
        });
    }

    pub fn set_font_color(&mut self, color: Option<Color>, cx: &mut Context<Self>) {
        if color.is_some() {
            self.last_font_color = color;
        }
        self.apply_style(t("Font Color", "Couleur de police"), cx, move |s| {
            s.font.color = color
        });
    }

    pub fn set_number_format(&mut self, code: &str, cx: &mut Context<Self>) {
        let code: Arc<str> = Arc::from(code);
        self.apply_style(t("Number Format", "Format de nombre"), cx, move |s| {
            s.num_fmt = code.clone()
        });
    }

    pub fn change_decimals(&mut self, delta: i32, cx: &mut Context<Self>) {
        // Start from what the active cell shows when it is still "General".
        let (r, c) = self.active();
        let base = self.active_style().num_fmt.to_string();
        let base = if base.eq_ignore_ascii_case("General") {
            match self.wb.value_at(self.sheet, r, c) {
                opensheet_engine::Value::Number(n) => {
                    let shown = format::format_general(n, Locale::en(), 11);
                    let decimals = shown.split('.').nth(1).map(|d| d.len()).unwrap_or(0);
                    if decimals == 0 {
                        "0".to_string()
                    } else {
                        format!("0.{}", "0".repeat(decimals))
                    }
                }
                _ => "0".to_string(),
            }
        } else {
            base
        };
        let code = format::adjust_decimals(&base, delta);
        self.set_number_format(&code, cx);
    }

    pub fn apply_border(&mut self, preset: BorderPreset, cx: &mut Context<Self>) {
        if !self.may_all(Intent::FormatCells, cx) {
            return;
        }
        self.last_border = preset;
        let ranges = self.sel().ranges.clone();
        for r in ranges {
            self.wb.apply_borders(self.sheet, r, preset, Border::thin());
        }
        self.commit(t("Borders", "Bordures"), cx);
    }

    pub fn toggle_merge(&mut self, center: bool, cx: &mut Context<Self>) {
        let rect = self.sel().current();
        let sheet = self.sheet;
        if !self.may(Intent::ChangeSheet, cx) {
            return;
        }
        if self.wb.sheets[sheet]
            .merges
            .iter()
            .any(|m| rect.intersects(m))
        {
            self.wb.unmerge(sheet, rect);
        } else if !rect.is_single() {
            self.wb.merge(sheet, rect);
            if center {
                self.wb
                    .update_style(sheet, Rect::cell(rect.r0, rect.c0), &|s| {
                        s.h_align = HAlign::Center
                    });
            }
        }
        self.commit(t("Merge Cells", "Fusionner les cellules"), cx);
        let active = self.active();
        self.select_cell(active.0, active.1, cx);
    }

    pub fn insert_rows(&mut self, cx: &mut Context<Self>) {
        let r = self.sel().current();
        let count = r.rows().min(MAX_ROWS - r.r0);
        if !self.may(Intent::InsertRows { at: r.r0, count }, cx) {
            return;
        }
        self.wb.insert_rows(self.sheet, r.r0, count);
        self.commit(t("Insert Rows", "Insérer des lignes"), cx);
    }

    pub fn delete_rows(&mut self, cx: &mut Context<Self>) {
        let r = self.sel().current();
        if !self.may(
            Intent::DeleteRows {
                first: r.r0,
                last: r.r1,
            },
            cx,
        ) {
            return;
        }
        self.wb.delete_rows(self.sheet, r.r0, r.rows());
        self.commit(t("Delete Rows", "Supprimer des lignes"), cx);
    }

    pub fn insert_cols(&mut self, cx: &mut Context<Self>) {
        let r = self.sel().current();
        let count = r.cols().min(MAX_COLS - r.c0);
        if !self.may(Intent::InsertColumns { at: r.c0, count }, cx) {
            return;
        }
        self.wb.insert_cols(self.sheet, r.c0, count);
        self.commit(t("Insert Columns", "Insérer des colonnes"), cx);
    }

    pub fn delete_cols(&mut self, cx: &mut Context<Self>) {
        let r = self.sel().current();
        if !self.may(
            Intent::DeleteColumns {
                first: r.c0,
                last: r.c1,
            },
            cx,
        ) {
            return;
        }
        self.wb.delete_cols(self.sheet, r.c0, r.cols());
        self.commit(t("Delete Columns", "Supprimer des colonnes"), cx);
    }

    pub fn hide_rows(&mut self, hidden: bool, cx: &mut Context<Self>) {
        if !self.may(Intent::FormatRows, cx) {
            return;
        }
        let r = self.sel().current();
        self.wb.set_rows_hidden(self.sheet, r.r0, r.r1, hidden);
        self.commit(t("Hide Rows", "Masquer les lignes"), cx);
    }

    pub fn hide_cols(&mut self, hidden: bool, cx: &mut Context<Self>) {
        if !self.may(Intent::FormatColumns, cx) {
            return;
        }
        let r = self.sel().current();
        self.wb.set_cols_hidden(self.sheet, r.c0, r.c1, hidden);
        self.commit(t("Hide Columns", "Masquer les colonnes"), cx);
    }

    pub fn auto_sum(&mut self, function: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            return;
        }
        let (row, col) = self.active();
        let name = self.wb.locale.fn_to_local(function).to_string();
        let text = match self.wb.autosum_range(self.sheet, row, col) {
            Some(rect) => format!("={}({})", name, rect.name()),
            None => format!("={}()", name),
        };
        self.start_edit(Some(&text), EditMode::Edit, EditSource::Cell, window, cx);
        // Leave the caret inside the parentheses.
        let len = text.len();
        let open = text.find('(').map(|i| i + 1).unwrap_or(len);
        self.cell_editor
            .update(cx, |input, cx| input.set_selection(open..len - 1, cx));
    }

    /// Start a formula with the given function (Formulas tab).
    pub fn insert_function(&mut self, function: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            return;
        }
        let name = self.wb.locale.fn_to_local(function).to_string();
        let text = format!("={}()", name);
        self.start_edit(Some(&text), EditMode::Edit, EditSource::Cell, window, cx);
        let len = text.len();
        self.cell_editor
            .update(cx, |input, cx| input.set_selection(len - 1..len - 1, cx));
    }

    pub fn set_zoom(&mut self, zoom: f32, cx: &mut Context<Self>) {
        let zoom = (zoom * 100.0).round().clamp(10.0, 400.0) / 100.0;
        let ratio = (zoom / self.zoom) as f64;
        for v in &mut self.views {
            v.scroll_x *= ratio;
            v.scroll_y *= ratio;
        }
        self.zoom = zoom;
        cx.notify();
    }

    pub fn toggle_freeze(&mut self, cx: &mut Context<Self>) {
        let sheet = &self.wb.sheets[self.sheet];
        if sheet.frozen_rows > 0 || sheet.frozen_cols > 0 {
            self.wb.set_frozen(self.sheet, 0, 0);
        } else {
            let (r, c) = self.active();
            self.wb.set_frozen(self.sheet, r, c);
            let view = &mut self.views[self.sheet];
            view.scroll_x = 0.0;
            view.scroll_y = 0.0;
        }
        self.commit(t("Freeze Panes", "Figer les volets"), cx);
    }

    pub fn add_sheet(&mut self, cx: &mut Context<Self>) {
        if !self.may(Intent::InsertSheet, cx) {
            return;
        }
        let index = self.wb.add_sheet(self.sheet + 1);
        self.views.insert(index, SheetView::default());
        self.wb.commit(t("Insert Sheet", "Insérer une feuille"));
        self.sheet = index;
        self.refresh(cx);
    }

    pub fn delete_sheet(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.may(Intent::DeleteSheet, cx) {
            return;
        }
        if self.wb.delete_sheet(index) {
            if index < self.views.len() {
                self.views.remove(index);
            }
            self.wb.commit(t("Delete Sheet", "Supprimer la feuille"));
            if self.sheet >= self.wb.sheets.len() {
                self.sheet = self.wb.sheets.len() - 1;
            }
            self.refresh(cx);
        } else if self.wb.sheets.len() > 1 {
            // Hidden sheets remain: this one is the last with a tab.
            self.alert(
                Alert::new("OpenSheet", features::sheets::last_visible_message()).warning(),
                cx,
            );
        }
    }

    pub fn duplicate_sheet(&mut self, index: usize, cx: &mut Context<Self>) {
        if !self.may(Intent::CopySheet, cx) {
            return;
        }
        if let Some(new) = self.wb.duplicate_sheet(index) {
            self.views.insert(new, SheetView::default());
            self.wb.commit(t("Duplicate Sheet", "Dupliquer la feuille"));
            self.sheet = new;
            self.refresh(cx);
        }
    }

    pub fn autofit_col(&mut self, col: u32, window: &mut Window, cx: &mut Context<Self>) {
        if !self.may(Intent::FormatColumns, cx) {
            return;
        }
        let ts = window.text_system().clone();
        let width = crate::grid::autofit_col_width(&self.wb, self.sheet, col, &self.fonts, &ts);
        self.wb.set_col_width(self.sheet, col, width);
        self.commit(t("AutoFit Column", "Ajuster la largeur"), cx);
    }

    // ---- mouse ----------------------------------------------------------------

    pub fn hit_test(&self, layout: &GridLayout, pos: Point<Pixels>) -> Hit {
        self.hit_test_inner(layout, pos, true)
    }

    /// The topmost zone of a layer group under a point: those drawn over
    /// everything (`top`), or those drawn with the cells.
    fn zone_at(&self, x: f32, y: f32, top: bool) -> Option<usize> {
        self.zones
            .iter()
            .rposition(|z| (z.layer == features::PaintLayer::Top) == top && z.contains(x, y))
    }

    /// `zones`: false gives what lies under the feature zones.
    fn hit_test_inner(&self, layout: &GridLayout, pos: Point<Pixels>, zones: bool) -> Hit {
        let (x, y) = layout.local(pos);
        let (w, h) = (layout.width(), layout.height());
        if x < 0.0 || y < 0.0 || x >= w || y >= h {
            return Hit::None;
        }
        let (content_w, content_h) = self.content_size(layout);
        // Scrollbars sit on top of the cells.
        if x >= w - theme::SCROLLBAR && y >= layout.header_h {
            let (_, ty, len, pos, tl) = layout.v_scrollbar(content_h);
            if y <= ty + len {
                return if y >= ty + pos && y <= ty + pos + tl {
                    Hit::VThumb
                } else {
                    Hit::VTrack
                };
            }
        }
        if y >= h - theme::SCROLLBAR && x >= layout.header_w {
            let (tx, _, len, pos, tl) = layout.h_scrollbar(content_w);
            if x <= tx + len {
                return if x >= tx + pos && x <= tx + pos + tl {
                    Hit::HThumb
                } else {
                    Hit::HTrack
                };
            }
        }
        if zones {
            if let Some(i) = self.zone_at(x, y, true) {
                return Hit::Zone(i);
            }
        }
        let in_col_header = y < layout.header_h;
        let in_row_header = x < layout.header_w;
        if in_col_header && in_row_header {
            return Hit::Corner;
        }
        if in_col_header {
            let col = layout.col_at(x);
            let left = layout.col_x(col);
            let right = left + layout.col_w(col);
            if right - x <= 4.0 {
                return Hit::ColResize(col);
            }
            if x - left <= 4.0 && col > 0 {
                // The border on the left belongs to the previous visible column.
                let mut prev = col - 1;
                while prev > 0
                    && layout.col_w(prev) <= 0.0
                    && self.wb.sheets[self.sheet].hidden_cols.contains(&prev)
                {
                    prev -= 1;
                }
                return Hit::ColResize(prev);
            }
            return Hit::ColHeader(col);
        }
        if in_row_header {
            let row = layout.row_at(y);
            let top = layout.row_y(row);
            let bottom = top + layout.row_h(row);
            if bottom - y <= 3.0 {
                return Hit::RowResize(row);
            }
            if y - top <= 3.0 && row > 0 {
                return Hit::RowResize(row - 1);
            }
            return Hit::RowHeader(row);
        }
        // Charts, pictures and shapes lie on top of the cells.
        if let Some(hit) = self.hit_object(layout, x, y) {
            return hit;
        }
        if self.editing.is_none() && self.object_sel.is_none() && self.sel().ranges.len() == 1 {
            let (_, _, x1, y1) = layout.rect_px(self.sel().current());
            if (x - x1).abs() <= 5.0 && (y - y1).abs() <= 5.0 {
                return Hit::FillHandle;
            }
        }
        if zones {
            if let Some(i) = self.zone_at(x, y, false) {
                return Hit::Zone(i);
            }
        }
        Hit::Cell(layout.row_at(y), layout.col_at(x))
    }

    /// The cell under a point, clamped to the visible cell area.
    fn cell_at_clamped(&self, layout: &GridLayout, pos: Point<Pixels>) -> (u32, u32) {
        let (x, y) = layout.local(pos);
        let x = x.clamp(layout.header_w + 1.0, layout.width() - 1.0);
        let y = y.clamp(layout.header_h + 1.0, layout.height() - 1.0);
        (layout.row_at(y), layout.col_at(x))
    }

    /// Scroll a little when dragging past the edge of the grid.
    fn autoscroll(&mut self, layout: &GridLayout, pos: Point<Pixels>) {
        let (x, y) = layout.local(pos);
        let step = (20.0 * self.zoom) as f64;
        let view = &mut self.views[self.sheet];
        if x > layout.width() - 8.0 {
            view.scroll_x += step * 3.0;
        } else if x < layout.header_w + layout.frozen_w + 4.0 && layout.frozen_cols == 0 {
            view.scroll_x = (view.scroll_x - step * 3.0).max(0.0);
        }
        if y > layout.height() - 8.0 {
            view.scroll_y += step;
        } else if y < layout.header_h + layout.frozen_h + 4.0 && layout.frozen_rows == 0 {
            view.scroll_y = (view.scroll_y - step).max(0.0);
        }
    }

    /// The mouse event a feature receives for a zone or an object.
    fn zone_event(
        &self,
        layout: &GridLayout,
        position: Point<Pixels>,
        button: MouseButton,
        modifiers: gpui::Modifiers,
        click_count: usize,
    ) -> ZoneEvent {
        let (x, y) = layout.local(position);
        ZoneEvent {
            position,
            x,
            y,
            cell: self.cell_at_clamped(layout, position),
            button,
            modifiers,
            click_count,
        }
    }

    /// After a feature handled a press itself: the grid keeps the keyboard
    /// unless the feature gave it to something of its own (a field, a
    /// drop-down).
    fn settle_focus(&self, before: Option<FocusHandle>, window: &mut Window, cx: &App) {
        if window.focused(cx) == before && self.editing.is_none() && !self.modal_open() {
            window.focus(&self.grid_focus);
        }
        window.prevent_default();
    }

    pub fn grid_mouse_down(
        &mut self,
        ev: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.grid_mouse_down_inner(ev, window, cx);
        if self.editing.is_some() && !self.modal_open() {
            // Whatever the click did (double-click to edit, pointing at a
            // cell, scrolling), an edit in progress keeps the keyboard: the
            // grid must not take the focus back for itself.
            self.focus_editor(window, cx);
            window.prevent_default();
        }
    }

    fn grid_mouse_down_inner(
        &mut self,
        ev: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(layout) = self.current_layout() else {
            return;
        };
        let mut hit = self.hit_test(&layout, ev.position);
        self.hide_zone_tip();
        // A feature's zone gets the press first and may let it through.
        if let Hit::Zone(i) = hit {
            let zone = self.zones[i].clone();
            let event = self.zone_event(
                &layout,
                ev.position,
                ev.button,
                ev.modifiers,
                ev.click_count,
            );
            let focus = window.focused(cx);
            match features::all()[zone.owner].zone_mouse_down(self, &zone, &event, window, cx) {
                ZoneResponse::Ignored => hit = self.hit_test_inner(&layout, ev.position, false),
                response => {
                    if response == ZoneResponse::Capture {
                        self.drag = Some(Drag::Zone(zone));
                    }
                    self.settle_focus(focus, window, cx);
                    cx.notify();
                    return;
                }
            }
        }
        if ev.button == MouseButton::Right {
            self.grid_context_menu(hit, ev.position, window, cx);
            return;
        }
        if ev.button != MouseButton::Left {
            return;
        }
        self.popup = None;
        self.status_message = None;
        self.tab_origin = None;
        if self.editing.is_some()
            && matches!(
                hit,
                Hit::None | Hit::VThumb | Hit::HThumb | Hit::VTrack | Hit::HTrack | Hit::FillHandle
            )
        {
            // Scrolling while typing must not take the keyboard away from
            // the editor (the grid would otherwise grab focus on click).
            window.prevent_default();
        }
        let (lx, ly) = layout.local(ev.position);
        let shift = ev.modifiers.shift;
        let add = ev.modifiers.platform || ev.modifiers.control;

        match hit {
            Hit::None | Hit::Zone(_) => {}
            Hit::Object(id) | Hit::ObjectHandle(id, _) => {
                if self.editing.is_some() && !self.commit_edit(None, window, cx) {
                    return;
                }
                // Shift or ⌘ adds the object to the selection, or takes
                // it out: then there is nothing to drag.
                if !self.click_object(id, shift || add, cx) {
                    window.focus(&self.grid_focus);
                    self.refresh(cx);
                    return;
                }
                if ev.click_count >= 2 && matches!(hit, Hit::Object(_)) {
                    let event = self.zone_event(
                        &layout,
                        ev.position,
                        ev.button,
                        ev.modifiers,
                        ev.click_count,
                    );
                    let focus = window.focused(cx);
                    features::all()[id.owner].object_double_click(self, id.id, &event, window, cx);
                    // The feature may have put the keyboard in a field.
                    self.settle_focus(focus, window, cx);
                    return;
                }
                window.focus(&self.grid_focus);
                if features::guard(self, &Intent::EditObject(Some(id))).is_none() {
                    let handle = match hit {
                        Hit::ObjectHandle(_, handle) => Some(handle),
                        _ => None,
                    };
                    self.begin_object_drag(&layout, id, handle, (lx, ly));
                }
                self.refresh(cx);
            }
            Hit::VThumb => {
                let (_, ty, _, pos, _) = layout.v_scrollbar(self.content_size(&layout).1);
                self.drag = Some(Drag::VScroll {
                    grab: ly - (ty + pos),
                });
            }
            Hit::HThumb => {
                let (tx, _, _, pos, _) = layout.h_scrollbar(self.content_size(&layout).0);
                self.drag = Some(Drag::HScroll {
                    grab: lx - (tx + pos),
                });
            }
            Hit::VTrack => {
                let (_, ty, _, pos, _) = layout.v_scrollbar(self.content_size(&layout).1);
                let page = (layout.view_h() * 0.9) as f64;
                let view = &mut self.views[self.sheet];
                view.scroll_y = if ly < ty + pos {
                    (view.scroll_y - page).max(0.0)
                } else {
                    view.scroll_y + page
                };
            }
            Hit::HTrack => {
                let (tx, _, _, pos, _) = layout.h_scrollbar(self.content_size(&layout).0);
                let page = (layout.view_w() * 0.9) as f64;
                let view = &mut self.views[self.sheet];
                view.scroll_x = if lx < tx + pos {
                    (view.scroll_x - page).max(0.0)
                } else {
                    view.scroll_x + page
                };
            }
            Hit::ColResize(col) => {
                if self.editing.is_some() && !self.commit_edit(None, window, cx) {
                    return;
                }
                if ev.click_count >= 2 {
                    self.autofit_col(col, window, cx);
                } else if self.may(Intent::FormatColumns, cx) {
                    self.drag = Some(Drag::ColResize {
                        col,
                        start_x: lx,
                        start_w: self.wb.sheets[self.sheet].col_width(col),
                    });
                }
            }
            Hit::RowResize(row) => {
                if self.editing.is_some() && !self.commit_edit(None, window, cx) {
                    return;
                }
                if !self.may(Intent::FormatRows, cx) {
                    return;
                }
                if ev.click_count >= 2 {
                    self.wb.set_row_height(self.sheet, row, None);
                    self.commit(t("AutoFit Row", "Ajuster la hauteur"), cx);
                } else {
                    self.drag = Some(Drag::RowResize {
                        row,
                        start_y: ly,
                        start_h: self.wb.sheets[self.sheet].row_height(row),
                    });
                }
            }
            Hit::Corner => {
                if self.editing.is_some() && !self.commit_edit(None, window, cx) {
                    return;
                }
                self.select_all(cx);
                window.focus(&self.grid_focus);
            }
            Hit::ColHeader(col) => {
                if self.editing.is_some() {
                    if self.can_point(cx) {
                        self.point_at((0, col), (0, col), SelectKind::Cols, cx);
                        self.drag = Some(Drag::Point(SelectKind::Cols));
                        self.focus_editor(window, cx);
                        window.prevent_default();
                        return;
                    }
                    if !self.commit_edit(None, window, cx) {
                        return;
                    }
                }
                let top = layout.scrolled_rows().start;
                let sel = self.sel_mut();
                if shift {
                    let a = sel.anchor.1;
                    sel.set_current(Rect::new(0, a, MAX_ROWS - 1, col));
                } else {
                    let rect = Rect::new(0, col, MAX_ROWS - 1, col);
                    if add {
                        sel.ranges.push(rect);
                    } else {
                        sel.ranges = vec![rect];
                    }
                    sel.active = (top, col);
                    sel.anchor = (0, col);
                }
                self.drag = Some(Drag::Select(SelectKind::Cols));
                window.focus(&self.grid_focus);
                self.refresh(cx);
            }
            Hit::RowHeader(row) => {
                if self.editing.is_some() {
                    if self.can_point(cx) {
                        self.point_at((row, 0), (row, 0), SelectKind::Rows, cx);
                        self.drag = Some(Drag::Point(SelectKind::Rows));
                        self.focus_editor(window, cx);
                        window.prevent_default();
                        return;
                    }
                    if !self.commit_edit(None, window, cx) {
                        return;
                    }
                }
                let left = layout.scrolled_cols().start;
                let sel = self.sel_mut();
                if shift {
                    let a = sel.anchor.0;
                    sel.set_current(Rect::new(a, 0, row, MAX_COLS - 1));
                } else {
                    let rect = Rect::new(row, 0, row, MAX_COLS - 1);
                    if add {
                        sel.ranges.push(rect);
                    } else {
                        sel.ranges = vec![rect];
                    }
                    sel.active = (row, left);
                    sel.anchor = (row, 0);
                }
                self.drag = Some(Drag::Select(SelectKind::Rows));
                window.focus(&self.grid_focus);
                self.refresh(cx);
            }
            Hit::FillHandle => {
                let src = self.sel().current();
                if ev.click_count >= 2 {
                    self.fill_down_to_neighbour(src, cx);
                } else {
                    self.drag = Some(Drag::Fill { src, dest: src });
                }
            }
            Hit::Cell(row, col) => {
                if self.editing.is_some() {
                    if self.can_point(cx) {
                        if shift && self.editing.as_ref().is_some_and(|e| e.point.is_some()) {
                            let anchor = self
                                .editing
                                .as_ref()
                                .unwrap()
                                .point
                                .as_ref()
                                .unwrap()
                                .anchor;
                            self.point_at(anchor, (row, col), SelectKind::Cells, cx);
                        } else {
                            self.point_at((row, col), (row, col), SelectKind::Cells, cx);
                        }
                        self.drag = Some(Drag::Point(SelectKind::Cells));
                        // The click inserted a reference: typing goes on in
                        // the formula, so the editor keeps the keyboard.
                        self.focus_editor(window, cx);
                        window.prevent_default();
                        return;
                    }
                    if !self.commit_edit(None, window, cx) {
                        return;
                    }
                }
                window.focus(&self.grid_focus);
                if shift {
                    self.extend_to(row, col, cx);
                } else if add {
                    let rect = self.expand_merges(Rect::cell(row, col));
                    let sel = self.sel_mut();
                    sel.ranges.push(rect);
                    sel.active = (rect.r0, rect.c0);
                    sel.anchor = (row, col);
                    self.refresh(cx);
                } else {
                    self.select_cell(row, col, cx);
                    if ev.click_count >= 2 {
                        self.start_edit(None, EditMode::Edit, EditSource::Cell, window, cx);
                        self.caret_under_pointer(&layout, lx, window, cx);
                        return;
                    }
                }
                self.drag = Some(Drag::Select(SelectKind::Cells));
            }
        }
        cx.notify();
    }

    /// After a double click on a cell: put the caret at the character
    /// under the pointer, where the editor now shows it.
    fn caret_under_pointer(
        &mut self,
        layout: &GridLayout,
        lx: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(edit) = &self.editing else {
            return;
        };
        let sheet = &self.wb.sheets[self.sheet];
        let rect = sheet
            .merge_at(edit.row, edit.col)
            .unwrap_or(Rect::cell(edit.row, edit.col));
        let (x0, _, _, _) = layout.rect_px(rect);
        let style = self
            .wb
            .display(self.sheet, edit.row, edit.col)
            .style
            .into_owned();
        let font_px = (crate::grid::pt_to_px(style.font.size_pt()) * self.zoom).max(1.0);
        let pad = (3.0 * self.zoom).round().max(1.0);
        let font = crate::grid::cell_font(&style, &self.fonts);
        let text = self.cell_editor.read(cx).text().to_string();
        let index =
            TextInput::index_for_offset(&text, px(lx - x0 - pad), font, px(font_px), window);
        self.cell_editor
            .update(cx, |input, cx| input.set_selection(index..index, cx));
    }

    /// The pointer went from one part of the grid to another: tell the
    /// feature whose zone was left or entered, and arm its tooltip.
    fn hover_changed(
        &mut self,
        was: Hit,
        now: Hit,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.hide_zone_tip();
        if let (Hit::Zone(_), Some(owner)) = (was, self.hover_owner.take()) {
            features::all()[owner].zone_hover(self, None, cx);
        }
        let Hit::Zone(i) = now else {
            return;
        };
        let Some(zone) = self.zones.get(i).cloned() else {
            return;
        };
        self.hover_owner = Some(zone.owner);
        features::all()[zone.owner].zone_hover(self, Some(&zone), cx);
        if let Some(text) = zone.tooltip {
            self.zone_tip_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(600))
                    .await;
                this.update(cx, |app, cx| {
                    app.zone_tip = Some((position, text));
                    cx.notify();
                })
                .ok();
            }));
        }
    }

    fn hide_zone_tip(&mut self) {
        self.zone_tip_task = None;
        self.zone_tip = None;
    }

    pub fn grid_mouse_move(
        &mut self,
        ev: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(layout) = self.current_layout() else {
            return;
        };
        let Some(drag) = self.drag.clone() else {
            let hit = self.hit_test(&layout, ev.position);
            if hit != self.hover {
                // Only repaint when the pointer zone changes kind (or goes
                // from one feature zone or handle to another: they may
                // differ in cursor).
                let kind = |h: Hit| std::mem::discriminant(&h);
                let changed = kind(hit) != kind(self.hover)
                    || matches!(hit, Hit::Zone(_) | Hit::ObjectHandle(..));
                let was = self.hover;
                self.hover = hit;
                self.hover_changed(was, hit, ev.position, cx);
                if changed {
                    cx.notify();
                }
            }
            return;
        };
        if ev.pressed_button != Some(MouseButton::Left) {
            // The button was released outside the window.
            self.end_drag(ev.position, window, cx);
            return;
        }
        let (lx, ly) = layout.local(ev.position);
        match drag {
            Drag::Zone(zone) => {
                let event =
                    self.zone_event(&layout, ev.position, MouseButton::Left, ev.modifiers, 1);
                features::all()[zone.owner].zone_drag(self, &zone, &event, window, cx);
            }
            Drag::Pick(kind) => {
                self.autoscroll(&layout, ev.position);
                let lead = self.cell_at_clamped(&layout, ev.position);
                self.pick_to(lead, kind, cx);
            }
            Drag::Select(kind) => {
                self.autoscroll(&layout, ev.position);
                let (row, col) = self.cell_at_clamped(&layout, ev.position);
                let anchor = self.sel().anchor;
                match kind {
                    SelectKind::Cells => {
                        let rect = self.expand_merges(Rect::new(anchor.0, anchor.1, row, col));
                        self.sel_mut().set_current(rect);
                    }
                    SelectKind::Cols => {
                        self.sel_mut()
                            .set_current(Rect::new(0, anchor.1, MAX_ROWS - 1, col))
                    }
                    SelectKind::Rows => {
                        self.sel_mut()
                            .set_current(Rect::new(anchor.0, 0, row, MAX_COLS - 1))
                    }
                }
                self.update_name_box(cx);
            }
            Drag::Point(kind) => {
                self.autoscroll(&layout, ev.position);
                let (row, col) = self.cell_at_clamped(&layout, ev.position);
                if let Some(anchor) = self
                    .editing
                    .as_ref()
                    .and_then(|e| e.point.as_ref())
                    .map(|p| p.anchor)
                {
                    self.point_at(anchor, (row, col), kind, cx);
                }
            }
            Drag::Fill { src, .. } => {
                self.autoscroll(&layout, ev.position);
                let (row, col) = self.cell_at_clamped(&layout, ev.position);
                // Extend along the axis the pointer moved furthest.
                let dr = if row > src.r1 {
                    row as i64 - src.r1 as i64
                } else if row < src.r0 {
                    row as i64 - src.r0 as i64
                } else {
                    0
                };
                let dc = if col > src.c1 {
                    col as i64 - src.c1 as i64
                } else if col < src.c0 {
                    col as i64 - src.c0 as i64
                } else {
                    0
                };
                let dest = if dr == 0 && dc == 0 {
                    src
                } else if dr.abs() >= dc.abs() {
                    Rect::new(src.r0.min(row), src.c0, src.r1.max(row), src.c1)
                } else {
                    Rect::new(src.r0, src.c0.min(col), src.r1, src.c1.max(col))
                };
                self.drag = Some(Drag::Fill { src, dest });
            }
            Drag::ColResize {
                col,
                start_x,
                start_w,
            } => {
                let w = (start_w + (lx - start_x) / self.zoom).round().max(0.0);
                self.wb.set_col_width(self.sheet, col, Some(w));
                self.set_status(
                    crate::i18n::labelled(t("Width", "Largeur"), format!("{w:.0} px")),
                    cx,
                );
            }
            Drag::RowResize {
                row,
                start_y,
                start_h,
            } => {
                let h = (start_h + (ly - start_y) / self.zoom).round().max(0.0);
                self.wb.set_row_height(self.sheet, row, Some(h));
                self.set_status(
                    crate::i18n::labelled(t("Height", "Hauteur"), format!("{h:.0} px")),
                    cx,
                );
            }
            Drag::ObjectMove {
                id,
                start,
                origin,
                size,
            } => {
                self.drag_object_move(&layout, id, start, origin, size, (lx, ly));
            }
            Drag::ObjectResize {
                id,
                handle,
                start,
                origin,
            } => {
                self.drag_object_resize(&layout, id, handle, start, origin, (lx, ly));
            }
            Drag::VScroll { grab } => {
                let content = self.content_size(&layout).1;
                let (_, ty, len, _, tl) = layout.v_scrollbar(content);
                let view = layout.view_h() as f64;
                let range = content.max(layout.scroll_y + view).max(view + 1.0);
                let frac = ((ly - grab - ty) / (len - tl).max(1.0)).clamp(0.0, 1.0) as f64;
                self.views[self.sheet].scroll_y = frac * (range - view);
            }
            Drag::HScroll { grab } => {
                let content = self.content_size(&layout).0;
                let (tx, _, len, _, tl) = layout.h_scrollbar(content);
                let view = layout.view_w() as f64;
                let range = content.max(layout.scroll_x + view).max(view + 1.0);
                let frac = ((lx - grab - tx) / (len - tl).max(1.0)).clamp(0.0, 1.0) as f64;
                self.views[self.sheet].scroll_x = frac * (range - view);
            }
        }
        cx.notify();
    }

    /// The press that started a drag ended at `position`.
    fn end_drag(&mut self, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(Drag::Zone(zone)) = &self.drag {
            let zone = zone.clone();
            self.drag = None;
            if let Some(layout) = self.current_layout() {
                let event =
                    self.zone_event(&layout, position, MouseButton::Left, window.modifiers(), 1);
                features::all()[zone.owner].zone_mouse_up(self, &zone, &event, window, cx);
            }
            cx.notify();
            return;
        }
        self.finish_drag(cx);
    }

    fn finish_drag(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        match drag {
            Drag::Fill { src, dest } => {
                if dest != src && self.may(Intent::EditCells(dest), cx) {
                    self.wb.autofill(self.sheet, src, dest);
                    self.commit_cells(
                        t("Auto Fill", "Recopie incrémentée"),
                        &[dest],
                        ChangeCause::Fill,
                        cx,
                    );
                    let active = self.active();
                    let sel = self.sel_mut();
                    sel.ranges = vec![dest];
                    sel.active = if dest.contains(active.0, active.1) {
                        active
                    } else {
                        (dest.r0, dest.c0)
                    };
                    sel.anchor = (dest.r0, dest.c0);
                }
            }
            Drag::ColResize { .. } => {
                self.wb.commit(t("Column Width", "Largeur de colonne"));
                self.status_message = None;
            }
            Drag::RowResize { .. } => {
                self.wb.commit(t("Row Height", "Hauteur de ligne"));
                self.status_message = None;
            }
            Drag::ObjectMove { id, .. } => self.commit_object(id, features::ObjectVerb::Move, cx),
            Drag::ObjectResize { id, .. } => {
                self.commit_object(id, features::ObjectVerb::Resize, cx)
            }
            Drag::Pick(_) => self.end_pick(cx),
            Drag::Select(_) => {
                // The format painter applies to the range just selected.
                if let Some(clip) = self.painter.take() {
                    let dest = self.sel().current();
                    if !self.may(Intent::FormatCells(dest), cx) {
                        self.refresh(cx);
                        return;
                    }
                    self.wb.paste(self.sheet, dest, &clip, PasteMode::Formats);
                    self.wb
                        .commit(t("Format Painter", "Reproduire la mise en forme"));
                    self.status_message = None;
                }
            }
            _ => {}
        }
        self.refresh(cx);
    }

    /// Double-click on the fill handle: fill down as far as the column next
    /// to the selection has data.
    fn fill_down_to_neighbour(&mut self, src: Rect, cx: &mut Context<Self>) {
        let sheet = &self.wb.sheets[self.sheet];
        let mut last = src.r1;
        for col in [
            src.c0.checked_sub(1),
            (src.c1 + 1 < MAX_COLS).then_some(src.c1 + 1),
        ]
        .into_iter()
        .flatten()
        {
            let mut r = src.r1;
            while r + 1 < MAX_ROWS && !sheet.is_blank(r + 1, col) {
                r += 1;
            }
            last = last.max(r);
        }
        if last > src.r1 {
            let dest = Rect::new(src.r0, src.c0, last, src.c1);
            if !self.may(Intent::EditCells(dest), cx) {
                return;
            }
            self.wb.autofill(self.sheet, src, dest);
            self.sel_mut().ranges = vec![dest];
            self.commit_cells(
                t("Auto Fill", "Recopie incrémentée"),
                &[dest],
                ChangeCause::Fill,
                cx,
            );
        }
    }

    pub fn start_format_painter(&mut self, cx: &mut Context<Self>) {
        let clip = self.wb.copy(self.sheet, self.sel().current());
        self.painter = Some(clip);
        self.set_status(
            t(
                "Select the cells to format.",
                "Sélectionnez les cellules à mettre en forme.",
            ),
            cx,
        );
    }

    pub fn grid_mouse_up(
        &mut self,
        ev: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.end_drag(ev.position, window, cx);
    }

    pub fn fill_preview(&self) -> Option<Rect> {
        match &self.drag {
            Some(Drag::Fill { dest, src }) if dest != src => Some(*dest),
            _ => None,
        }
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    pub fn hot_scrollbar(&self) -> u8 {
        match (&self.drag, self.hover) {
            (Some(Drag::VScroll { .. }), _) | (None, Hit::VThumb) => 1,
            (Some(Drag::HScroll { .. }), _) | (None, Hit::HThumb) => 2,
            _ => 0,
        }
    }

    pub fn cursor_style(&self) -> gpui::CursorStyle {
        use gpui::CursorStyle;
        // The object handle being dragged or hovered, if any.
        let handle = match (&self.drag, self.hover) {
            (Some(Drag::ObjectResize { handle, .. }), _) => Some(*handle),
            (None, Hit::ObjectHandle(_, handle)) => Some(handle),
            _ => None,
        };
        if let Some(handle) = handle {
            return match handle {
                0 | 4 => CursorStyle::ResizeUpLeftDownRight,
                2 | 6 => CursorStyle::ResizeUpRightDownLeft,
                1 | 5 => CursorStyle::ResizeUpDown,
                _ => CursorStyle::ResizeLeftRight,
            };
        }
        match (&self.drag, self.hover) {
            (Some(Drag::Zone(zone)), _) => zone.cursor.unwrap_or(CursorStyle::Arrow),
            (None, Hit::Zone(i)) => self
                .zones
                .get(i)
                .and_then(|zone| zone.cursor)
                .unwrap_or(CursorStyle::Arrow),
            (Some(Drag::ColResize { .. }), _) | (None, Hit::ColResize(_)) => {
                CursorStyle::ResizeLeftRight
            }
            (Some(Drag::RowResize { .. }), _) | (None, Hit::RowResize(_)) => {
                CursorStyle::ResizeUpDown
            }
            (Some(Drag::Fill { .. }), _) | (None, Hit::FillHandle) => CursorStyle::Crosshair,
            _ if self.painter.is_some() => CursorStyle::Crosshair,
            _ => CursorStyle::Arrow,
        }
    }

    pub fn clear_hover(&mut self, cx: &mut Context<Self>) {
        if self.hover != Hit::None {
            let was = self.hover;
            self.hover = Hit::None;
            self.hover_changed(was, Hit::None, Point::default(), cx);
            cx.notify();
        }
    }

    pub fn grid_scroll(
        &mut self,
        ev: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = ev.delta.pixel_delta(px(20.0 * self.zoom));
        let (mut dx, mut dy) = (f32::from(delta.x) as f64, f32::from(delta.y) as f64);
        // A feature's zone may scroll what it shows (the buttons of a
        // slicer) instead of the sheet.
        if let Some(layout) = self.current_layout() {
            let (x, y) = layout.local(ev.position);
            if let Some(zone) = self
                .zone_at(x, y, true)
                .or_else(|| self.zone_at(x, y, false))
                .map(|i| self.zones[i].clone())
                && features::all()[zone.owner].zone_scroll(self, &zone, (dx as f32, dy as f32), cx)
            {
                cx.notify();
                return;
            }
        }
        if ev.modifiers.platform || ev.modifiers.control {
            // Zoom with Cmd/Ctrl + wheel.
            if dy.abs() > 0.5 {
                let step = if dy > 0.0 { 0.05 } else { -0.05 };
                self.set_zoom(self.zoom + step, cx);
            }
            return;
        }
        if ev.modifiers.shift && dx == 0.0 {
            dx = dy;
            dy = 0.0;
        }
        let Some(layout) = self.current_layout() else {
            return;
        };
        let view = &mut self.views[self.sheet];
        view.scroll_x = (view.scroll_x - dx).clamp(0.0, layout.max_scroll_x());
        view.scroll_y = (view.scroll_y - dy).clamp(0.0, layout.max_scroll_y());
        cx.notify();
    }

    fn grid_context_menu(
        &mut self,
        hit: Hit,
        pos: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing.is_some() && !self.commit_edit(None, window, cx) {
            return;
        }
        window.focus(&self.grid_focus);
        let mut menu = ContextMenu::default();
        let clipboard = |menu: &mut ContextMenu| {
            let mut add = |entry: MenuEntry| menu.add(MenuSection::Clipboard, entry);
            add(
                MenuEntry::new(t("Cut", "Couper"), |a, _, cx| a.copy(true, cx))
                    .icon("cut")
                    .shortcut("⌘X"),
            );
            add(
                MenuEntry::new(t("Copy", "Copier"), |a, _, cx| a.copy(false, cx))
                    .icon("copy")
                    .shortcut("⌘C"),
            );
            add(
                MenuEntry::new(t("Paste", "Coller"), |a, _, cx| a.paste(PasteMode::All, cx))
                    .icon("paste")
                    .shortcut("⌘V"),
            );
        };
        let paste_special = |menu: &mut ContextMenu| {
            menu.add(
                MenuSection::Clipboard,
                MenuEntry::new(t("Paste Values", "Coller les valeurs"), |a, _, cx| {
                    a.paste(PasteMode::Values, cx)
                })
                .shortcut("⇧⌘V"),
            );
            menu.add(
                MenuSection::Clipboard,
                MenuEntry::new(
                    t("Paste Formatting", "Coller la mise en forme"),
                    |a, _, cx| a.paste(PasteMode::Formats, cx),
                ),
            );
        };
        let clear = || {
            MenuEntry::new(t("Clear Contents", "Effacer le contenu"), |a, _, cx| {
                a.clear(ClearMode::Contents, cx)
            })
        };
        let target = match hit {
            Hit::Object(id) | Hit::ObjectHandle(id, _) => {
                self.click_object(id, false, cx);
                clipboard(&mut menu);
                let order = |label: &'static str, mv: features::ZMove| {
                    MenuEntry::new(
                        label,
                        move |a: &mut ExcelApp, _: &mut Window, cx: &mut Context<ExcelApp>| {
                            a.reorder_object(mv, cx)
                        },
                    )
                };
                menu.add(
                    MenuSection::Format,
                    MenuEntry::submenu(
                        t("Bring to Front", "Mettre au premier plan"),
                        vec![
                            order(
                                t("Bring to Front", "Mettre au premier plan"),
                                features::ZMove::ToFront,
                            ),
                            order(t("Bring Forward", "Avancer"), features::ZMove::Forward),
                        ],
                    ),
                );
                menu.add(
                    MenuSection::Format,
                    MenuEntry::submenu(
                        t("Send to Back", "Mettre à l'arrière-plan"),
                        vec![
                            order(
                                t("Send to Back", "Mettre à l'arrière-plan"),
                                features::ZMove::ToBack,
                            ),
                            order(t("Send Backward", "Reculer"), features::ZMove::Backward),
                        ],
                    ),
                );
                MenuTarget::Object(id)
            }
            Hit::ColHeader(col) | Hit::ColResize(col) => {
                let cur = self.sel().current();
                if !(cur.is_whole_cols() && col >= cur.c0 && col <= cur.c1) {
                    self.select_rect(Rect::new(0, col, MAX_ROWS - 1, col), cx);
                }
                clipboard(&mut menu);
                paste_special(&mut menu);
                let mut add = |entry: MenuEntry| menu.add(MenuSection::Edit, entry);
                add(
                    MenuEntry::new(t("Insert", "Insertion"), |a, _, cx| a.insert_cols(cx))
                        .icon("insert-cells"),
                );
                add(
                    MenuEntry::new(t("Delete", "Supprimer"), |a, _, cx| a.delete_cols(cx))
                        .icon("delete-cells"),
                );
                add(clear());
                let mut add = |entry: MenuEntry| menu.add(MenuSection::Format, entry);
                add(MenuEntry::new(
                    t("AutoFit Column Width", "Ajuster la largeur de colonne"),
                    |a, window, cx| {
                        let r = a.sel().current();
                        // One refusal, not one for each column.
                        if !a.may(Intent::FormatColumns, cx) {
                            return;
                        }
                        for col in r.c0..=r.c1.min(r.c0 + 500) {
                            a.autofit_col(col, window, cx);
                        }
                    },
                ));
                add(MenuEntry::new(t("Hide", "Masquer"), |a, _, cx| {
                    a.hide_cols(true, cx)
                }));
                add(MenuEntry::new(t("Unhide", "Afficher"), |a, _, cx| {
                    a.hide_cols(false, cx)
                }));
                MenuTarget::ColHeader(col)
            }
            Hit::RowHeader(row) | Hit::RowResize(row) => {
                let cur = self.sel().current();
                if !(cur.is_whole_rows() && row >= cur.r0 && row <= cur.r1) {
                    self.select_rect(Rect::new(row, 0, row, MAX_COLS - 1), cx);
                }
                clipboard(&mut menu);
                paste_special(&mut menu);
                let mut add = |entry: MenuEntry| menu.add(MenuSection::Edit, entry);
                add(
                    MenuEntry::new(t("Insert", "Insertion"), |a, _, cx| a.insert_rows(cx))
                        .icon("insert-cells"),
                );
                add(
                    MenuEntry::new(t("Delete", "Supprimer"), |a, _, cx| a.delete_rows(cx))
                        .icon("delete-cells"),
                );
                add(clear());
                let mut add = |entry: MenuEntry| menu.add(MenuSection::Format, entry);
                add(MenuEntry::new(t("Hide", "Masquer"), |a, _, cx| {
                    a.hide_rows(true, cx)
                }));
                add(MenuEntry::new(t("Unhide", "Afficher"), |a, _, cx| {
                    a.hide_rows(false, cx)
                }));
                MenuTarget::RowHeader(row)
            }
            Hit::Cell(row, col) => {
                if !self.sel().contains(row, col) {
                    self.select_cell(row, col, cx);
                }
                clipboard(&mut menu);
                paste_special(&mut menu);
                let mut add = |entry: MenuEntry| menu.add(MenuSection::Edit, entry);
                // A table cell gets the Insert and Delete submenus of its
                // table instead (`features/tables`).
                if self.wb.table_at(self.sheet, row, col).is_none() {
                    add(
                        MenuEntry::new(t("Insert Rows", "Insérer des lignes"), |a, _, cx| {
                            a.insert_rows(cx)
                        })
                        .icon("insert-cells"),
                    );
                    add(MenuEntry::new(
                        t("Insert Columns", "Insérer des colonnes"),
                        |a, _, cx| a.insert_cols(cx),
                    ));
                    add(
                        MenuEntry::new(t("Delete Rows", "Supprimer les lignes"), |a, _, cx| {
                            a.delete_rows(cx)
                        })
                        .icon("delete-cells"),
                    );
                    add(MenuEntry::new(
                        t("Delete Columns", "Supprimer les colonnes"),
                        |a, _, cx| a.delete_cols(cx),
                    ));
                }
                add(clear());
                menu.add(
                    MenuSection::Format,
                    MenuEntry::new(
                        t("Merge / Unmerge Cells", "Fusionner / annuler la fusion"),
                        |a, _, cx| a.toggle_merge(false, cx),
                    ),
                );
                MenuTarget::Cell(row, col)
            }
            _ => return,
        };
        features::context_menu(self, &target, &mut menu);
        self.open_context_menu(pos, menu.build(), cx);
    }

    /// Open a context menu at a point of the window.
    pub fn open_context_menu(
        &mut self,
        pos: Point<Pixels>,
        entries: Vec<MenuEntry>,
        cx: &mut Context<Self>,
    ) {
        if entries.is_empty() {
            return;
        }
        self.popup = Some(Popup::new(PopupKind::Menu(entries), pos, 250.0).max_height(600.0));
        cx.notify();
    }

    /// Open the drop-down of the command that is running: under the ribbon
    /// control that started it, or at the pointer when it came from a menu
    /// or the keyboard.
    pub fn open_command_menu(
        &mut self,
        entries: Vec<MenuEntry>,
        width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.command_anchor {
            Some(anchor) => self.open_menu(anchor, entries, width, window, cx),
            None => {
                self.popup = Some(Popup::new(
                    PopupKind::Menu(entries),
                    window.mouse_position(),
                    width,
                ));
                cx.notify();
            }
        }
    }

    /// Show a view of a feature as a drop-down at a point of the window
    /// (see `cell_anchor`). It closes on a click outside it, or with
    /// `close_popup`.
    pub fn open_popup_view(
        &mut self,
        pos: Point<Pixels>,
        view: impl Into<AnyView>,
        cx: &mut Context<Self>,
    ) {
        self.popup = Some(Popup::new(PopupKind::View(view.into()), pos, 0.0).max_height(600.0));
        cx.notify();
    }

    pub fn close_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(popup) = self.popup.take() {
            // A view may have held the keyboard (its search box).
            if matches!(popup.kind, PopupKind::View(_)) && !self.modal_open() {
                match self.editing {
                    Some(_) => self.focus_editor(window, cx),
                    None => window.focus(&self.grid_focus),
                }
            }
        }
        cx.notify();
    }

    // ---- picking a range for a dialog ---------------------------------------

    /// A press on the sheet while the range field of a dialog has the
    /// keyboard: the cells pointed at are written into the field.
    pub(crate) fn begin_pick(
        &mut self,
        input: Entity<TextInput>,
        style: crate::shell::RefStyle,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(layout) = self.current_layout() else {
            return false;
        };
        let (kind, anchor) = match self.hit_test_inner(&layout, position, false) {
            Hit::ColHeader(col) | Hit::ColResize(col) => (SelectKind::Cols, (0, col)),
            Hit::RowHeader(row) | Hit::RowResize(row) => (SelectKind::Rows, (row, 0)),
            Hit::Cell(..) | Hit::FillHandle | Hit::Object(_) | Hit::ObjectHandle(..) => {
                (SelectKind::Cells, self.cell_at_clamped(&layout, position))
            }
            _ => return false,
        };
        self.pick = Some(RangePick {
            input,
            style,
            rect: Rect::cell(anchor.0, anchor.1),
            anchor,
        });
        self.drag = Some(Drag::Pick(kind));
        self.pick_to(anchor, kind, cx);
        true
    }

    /// Extend the range being picked to `lead` and write it in the field.
    fn pick_to(&mut self, lead: (u32, u32), kind: SelectKind, cx: &mut Context<Self>) {
        let Some(anchor) = self.pick.as_ref().map(|p| p.anchor) else {
            return;
        };
        let rect = match kind {
            SelectKind::Cells => self.expand_merges(Rect::new(anchor.0, anchor.1, lead.0, lead.1)),
            SelectKind::Cols => Rect::new(0, anchor.1, MAX_ROWS - 1, lead.1),
            SelectKind::Rows => Rect::new(anchor.0, 0, lead.0, MAX_COLS - 1),
        };
        let sheet_name = self.wb.sheets[self.sheet].name.clone();
        if let Some(pick) = &mut self.pick {
            pick.rect = rect;
            let text = pick.style.format(rect, &sheet_name);
            pick.input.update(cx, |input, cx| {
                input.set_text(text, cx);
                input.select_all_text(cx);
            });
        }
        self.ensure_visible(lead.0, lead.1);
        cx.notify();
    }
    /// Open a drop-down below the ribbon control registered under `anchor`.
    pub fn open_menu(
        &mut self,
        anchor: &'static str,
        entries: Vec<MenuEntry>,
        width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.toggles_popup_of(anchor, cx) {
            return;
        }
        let pos = self.anchor_pos(anchor, window);
        self.popup = Some(Popup::new(PopupKind::Menu(entries), pos, width).anchored(anchor));
        cx.notify();
    }

    pub fn open_palette(
        &mut self,
        anchor: &'static str,
        target: ColorTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.toggles_popup_of(anchor, cx) {
            return;
        }
        let pos = self.anchor_pos(anchor, window);
        self.popup = Some(Popup::new(PopupKind::Palette(target), pos, 214.0).anchored(anchor));
        cx.notify();
    }

    /// A click on a control whose pop-up is open (or was closed by the
    /// press of this very click) closes it instead of opening it again.
    fn toggles_popup_of(&mut self, anchor: &'static str, cx: &mut Context<Self>) -> bool {
        let open = self
            .popup
            .as_ref()
            .is_some_and(|popup| popup.anchor == Some(anchor));
        let just_closed = self.popup_dismissed.take() == Some(anchor);
        if open {
            self.popup = None;
        }
        if open || just_closed {
            cx.notify();
        }
        open || just_closed
    }

    fn anchor_pos(&self, anchor: &'static str, window: &Window) -> Point<Pixels> {
        match self.anchors.borrow().get(anchor) {
            Some(b) => point(b.origin.x, b.origin.y + b.size.height + px(2.)),
            None => window.mouse_position(),
        }
    }

    pub fn pick_color(
        &mut self,
        target: ColorTarget,
        color: Option<Color>,
        cx: &mut Context<Self>,
    ) {
        self.popup = None;
        match target {
            ColorTarget::Fill => self.set_fill(color, cx),
            ColorTarget::Font => self.set_font_color(color, cx),
            ColorTarget::Tab => {
                if self.may(Intent::TabColor, cx) {
                    self.wb.set_tab_color(self.sheet, color);
                    self.commit(t("Tab Color", "Couleur d'onglet"), cx);
                }
            }
        }
    }
}

// Typing while the grid has focus starts editing the active cell.
impl EntityInputHandler for ExcelApp {
    fn text_for_range(
        &mut self,
        _range: Range<usize>,
        _adjusted: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        Some(String::new())
    }

    fn selected_text_range(
        &mut self,
        _ignore: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        None
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {}

    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if text.is_empty() || self.modal_open() {
            return;
        }
        if text.chars().all(|c| c.is_control()) {
            return;
        }
        if let (Some(id), None) = (self.object_sel, &self.editing) {
            // Typing with a chart or a picture selected does not go into a
            // cell; a shape takes it as its text.
            features::all()[id.owner].object_typed(self, id.id, text, window, cx);
            return;
        }
        if self.editing.is_some() {
            // The editor lost the keyboard (a click on the grid): hand the
            // text over and give it the focus back.
            let editor = self.editor().clone();
            self.focus_editor(window, cx);
            editor.update(cx, |input, cx| input.insert(text, window, cx));
            return;
        }
        self.start_edit(Some(text), EditMode::Enter, EditSource::Cell, window, cx);
        self.sync_other_editor(cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        new_selected_range: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if new_text.is_empty()
            || self.editing.is_some()
            || self.modal_open()
            || self.object_sel.is_some()
        {
            return;
        }
        // IME composition: hand over to the cell editor.
        self.start_edit(Some(""), EditMode::Enter, EditSource::Cell, window, cx);
        if self.editing.is_none() {
            return;
        }
        self.cell_editor.update(cx, |input, cx| {
            input.replace_and_mark_text_in_range(None, new_text, new_selected_range, window, cx)
        });
    }

    fn bounds_for_range(
        &mut self,
        _range: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let layout = self.current_layout()?;
        let (r, c) = self.active();
        let (x0, y0, x1, y1) = layout.rect_px(Rect::cell(r, c));
        let _ = element_bounds;
        Some(Bounds::new(
            layout.to_window(x0, y0),
            gpui::size(px(x1 - x0), px(y1 - y0)),
        ))
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

// Keyboard actions.
impl ExcelApp {
    pub fn on_move_up(&mut self, _: &MoveUp, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing_key(Direction::Up, false, false, window, cx)
            || self.nudge_object(Direction::Up, cx)
        {
            return;
        }
        self.move_selection(Direction::Up, false, cx);
    }
    pub fn on_move_down(&mut self, _: &MoveDown, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing_key(Direction::Down, false, false, window, cx)
            || self.nudge_object(Direction::Down, cx)
        {
            return;
        }
        self.move_selection(Direction::Down, false, cx);
    }
    pub fn on_move_left(&mut self, _: &MoveLeft, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing_key(Direction::Left, false, false, window, cx)
            || self.nudge_object(Direction::Left, cx)
        {
            return;
        }
        self.move_selection(Direction::Left, false, cx);
    }
    pub fn on_move_right(&mut self, _: &MoveRight, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing_key(Direction::Right, false, false, window, cx)
            || self.nudge_object(Direction::Right, cx)
        {
            return;
        }
        self.move_selection(Direction::Right, false, cx);
    }
    pub fn on_extend_up(&mut self, _: &ExtendUp, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing_key(Direction::Up, true, false, window, cx) {
            return;
        }
        self.move_selection(Direction::Up, true, cx);
    }
    pub fn on_extend_down(&mut self, _: &ExtendDown, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing_key(Direction::Down, true, false, window, cx) {
            return;
        }
        self.move_selection(Direction::Down, true, cx);
    }
    pub fn on_extend_left(&mut self, _: &ExtendLeft, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing_key(Direction::Left, true, false, window, cx) {
            return;
        }
        self.move_selection(Direction::Left, true, cx);
    }
    pub fn on_extend_right(
        &mut self,
        _: &ExtendRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing_key(Direction::Right, true, false, window, cx) {
            return;
        }
        self.move_selection(Direction::Right, true, cx);
    }
    pub fn on_jump_up(&mut self, _: &JumpUp, _: &mut Window, cx: &mut Context<Self>) {
        self.jump_selection(Direction::Up, false, cx);
    }
    pub fn on_jump_down(&mut self, _: &JumpDown, _: &mut Window, cx: &mut Context<Self>) {
        self.jump_selection(Direction::Down, false, cx);
    }
    pub fn on_jump_left(&mut self, _: &JumpLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.jump_selection(Direction::Left, false, cx);
    }
    pub fn on_jump_right(&mut self, _: &JumpRight, _: &mut Window, cx: &mut Context<Self>) {
        self.jump_selection(Direction::Right, false, cx);
    }
    pub fn on_extend_jump_up(&mut self, _: &ExtendJumpUp, _: &mut Window, cx: &mut Context<Self>) {
        self.jump_selection(Direction::Up, true, cx);
    }
    pub fn on_extend_jump_down(
        &mut self,
        _: &ExtendJumpDown,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.jump_selection(Direction::Down, true, cx);
    }
    pub fn on_extend_jump_left(
        &mut self,
        _: &ExtendJumpLeft,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.jump_selection(Direction::Left, true, cx);
    }
    pub fn on_extend_jump_right(
        &mut self,
        _: &ExtendJumpRight,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.jump_selection(Direction::Right, true, cx);
    }
    pub fn on_next_cell(&mut self, _: &NextCell, window: &mut Window, cx: &mut Context<Self>) {
        features::tables::tab_pressed(self);
        if self.editing_key(Direction::Right, false, true, window, cx)
            || self.cycle_object(true, cx)
        {
            return;
        }
        if self.tab_origin.is_none() {
            self.tab_origin = Some(self.active().1);
        }
        self.advance(Direction::Right, cx);
    }
    pub fn on_prev_cell(&mut self, _: &PrevCell, window: &mut Window, cx: &mut Context<Self>) {
        features::tables::tab_pressed(self);
        if self.editing_key(Direction::Left, false, true, window, cx)
            || self.cycle_object(false, cx)
        {
            return;
        }
        self.advance(Direction::Left, cx);
    }
    pub fn on_enter_down(&mut self, _: &EnterDown, window: &mut Window, cx: &mut Context<Self>) {
        // Where Return leads is a preference; it may lead nowhere.
        let Some(dir) = features::preferences::enter_direction(false) else {
            self.commit_edit(None, window, cx);
            return;
        };
        if self.editing_key(dir, false, true, window, cx) {
            return;
        }
        // Enter with a copy marquee pastes, like Excel.
        if self.clip.is_some() {
            self.paste(PasteMode::All, cx);
            self.clip = None;
            cx.notify();
            return;
        }
        let origin = self.tab_origin.take();
        self.advance(dir, cx);
        if let (Some(col), true) = (origin, self.sel().is_single_cell()) {
            let (r, _) = self.active();
            self.select_cell(r, col, cx);
        }
    }
    pub fn on_enter_up(&mut self, _: &EnterUp, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dir) = features::preferences::enter_direction(true) else {
            self.commit_edit(None, window, cx);
            return;
        };
        if self.editing_key(dir, false, true, window, cx) {
            return;
        }
        self.advance(dir, cx);
    }
    pub fn on_edit_cell(&mut self, _: &EditCell, window: &mut Window, cx: &mut Context<Self>) {
        self.start_edit(None, EditMode::Edit, EditSource::Cell, window, cx);
    }
    pub fn on_clear_contents(
        &mut self,
        _: &ClearContents,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing.is_some() {
            // Backspace belongs to the text being typed.
            self.focus_editor(window, cx);
            return;
        }
        if self.object_sel.is_some() {
            self.delete_selected_object(cx);
            return;
        }
        self.clear(ClearMode::Contents, cx);
    }
    pub fn on_copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        self.copy(false, cx);
    }
    pub fn on_cut(&mut self, _: &Cut, _: &mut Window, cx: &mut Context<Self>) {
        self.copy(true, cx);
    }
    pub fn on_paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        self.paste(PasteMode::All, cx);
    }
    pub fn on_paste_values(&mut self, _: &PasteValues, _: &mut Window, cx: &mut Context<Self>) {
        self.paste(PasteMode::Values, cx);
    }
    pub fn on_undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        self.undo(cx);
    }
    pub fn on_redo(&mut self, _: &Redo, _: &mut Window, cx: &mut Context<Self>) {
        self.redo(cx);
    }
    pub fn on_select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        // With an object selected: every object of the sheet.
        if self.object_sel.is_some() {
            self.select_all_objects(cx);
            return;
        }
        // First press selects the data region, second the whole sheet.
        let (r, c) = self.active();
        let region = self.wb.current_region(self.sheet, r, c);
        if !region.is_single()
            && self.sel().current() != region
            && !self.sel().current().is_whole_cols()
        {
            let sel = self.sel_mut();
            sel.ranges = vec![region];
            sel.anchor = (region.r0, region.c0);
            self.refresh(cx);
        } else {
            self.select_all(cx);
        }
    }
    pub fn on_bold(&mut self, _: &Bold, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_bold(cx);
    }
    pub fn on_italic(&mut self, _: &Italic, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_italic(cx);
    }
    pub fn on_underline(&mut self, _: &UnderlineText, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_underline(cx);
    }
    pub fn on_find(&mut self, _: &Find, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_find(window, cx);
    }
    pub fn on_fill_down(&mut self, _: &FillDown, _: &mut Window, cx: &mut Context<Self>) {
        self.fill(Direction::Down, cx);
    }
    pub fn on_fill_right(&mut self, _: &FillRight, _: &mut Window, cx: &mut Context<Self>) {
        self.fill(Direction::Right, cx);
    }
    /// Copy the first row of the selection down it, or its first column
    /// across it.
    pub fn fill(&mut self, dir: Direction, cx: &mut Context<Self>) {
        let r = self.wb.clamp_to_used(self.sheet, self.sel().current());
        if !self.may(Intent::EditCells(r), cx) {
            return;
        }
        let label = if dir == Direction::Right {
            self.wb.fill_right(self.sheet, r);
            t("Fill Right", "Recopier à droite")
        } else {
            self.wb.fill_down(self.sheet, r);
            t("Fill Down", "Recopier vers le bas")
        };
        self.commit_cells(label, &[r], ChangeCause::Fill, cx);
    }
    pub fn on_page_up(&mut self, _: &PageUp, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.current_layout().map(|l| l.page_rows()).unwrap_or(20);
        let (r, c) = self.active();
        self.select_cell(r.saturating_sub(n), c, cx);
    }
    pub fn on_page_down(&mut self, _: &PageDown, _: &mut Window, cx: &mut Context<Self>) {
        let n = self.current_layout().map(|l| l.page_rows()).unwrap_or(20);
        let (r, c) = self.active();
        self.select_cell((r + n).min(MAX_ROWS - 1), c, cx);
    }
    pub fn on_go_row_start(&mut self, _: &GoRowStart, _: &mut Window, cx: &mut Context<Self>) {
        let (r, _) = self.active();
        self.select_cell(r, 0, cx);
    }
    pub fn on_go_start(&mut self, _: &GoStart, _: &mut Window, cx: &mut Context<Self>) {
        self.select_cell(0, 0, cx);
    }
    pub fn on_go_end(&mut self, _: &GoEnd, _: &mut Window, cx: &mut Context<Self>) {
        let used = self.wb.sheets[self.sheet]
            .used_rect()
            .unwrap_or(Rect::cell(0, 0));
        self.select_cell(used.r1, used.c1, cx);
    }
    pub fn on_select_column(&mut self, _: &SelectColumn, _: &mut Window, cx: &mut Context<Self>) {
        let cur = self.sel().current();
        let active = self.active();
        let sel = self.sel_mut();
        sel.ranges = vec![Rect::new(0, cur.c0, MAX_ROWS - 1, cur.c1)];
        sel.active = active;
        self.refresh(cx);
    }
    pub fn on_select_row(&mut self, _: &SelectRow, _: &mut Window, cx: &mut Context<Self>) {
        let cur = self.sel().current();
        let active = self.active();
        let sel = self.sel_mut();
        sel.ranges = vec![Rect::new(cur.r0, 0, cur.r1, MAX_COLS - 1)];
        sel.active = active;
        self.refresh(cx);
    }
    pub fn on_insert_date(&mut self, _: &InsertDate, _: &mut Window, cx: &mut Context<Self>) {
        let (r, c) = self.active();
        if !self.may(Intent::EditCells(Rect::cell(r, c)), cx) {
            return;
        }
        let today = opensheet_engine::datetime::now_serial().floor();
        let text =
            format::format_number(today, self.wb.locale.short_date_format, self.wb.locale).text;
        if self.wb.set_cell_text(self.sheet, r, c, &text).is_ok() {
            self.commit_cells(
                t("Typing", "Saisie"),
                &[Rect::cell(r, c)],
                ChangeCause::Typing,
                cx,
            );
        }
    }
    pub fn on_auto_sum(&mut self, _: &AutoSum, window: &mut Window, cx: &mut Context<Self>) {
        self.auto_sum("SUM", window, cx);
    }
    pub fn on_zoom_in(&mut self, _: &ZoomIn, _: &mut Window, cx: &mut Context<Self>) {
        self.set_zoom(self.zoom + 0.1, cx);
    }
    pub fn on_zoom_out(&mut self, _: &ZoomOut, _: &mut Window, cx: &mut Context<Self>) {
        self.set_zoom(self.zoom - 0.1, cx);
    }
    pub fn on_zoom_reset(&mut self, _: &ZoomReset, _: &mut Window, cx: &mut Context<Self>) {
        self.set_zoom(1.0, cx);
    }
    pub fn on_next_sheet(&mut self, _: &NextSheet, window: &mut Window, cx: &mut Context<Self>) {
        let next = features::sheets::neighbour(self, true);
        self.switch_sheet(next, window, cx);
    }
    pub fn on_prev_sheet(&mut self, _: &PrevSheet, window: &mut Window, cx: &mut Context<Self>) {
        let prev = features::sheets::neighbour(self, false);
        self.switch_sheet(prev, window, cx);
    }
    pub fn on_cancel(&mut self, _: &Cancel, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.is_some() {
            self.cancel_edit(window, cx);
            return;
        }
        // The drop-down of a folded ribbon group.
        crate::ribbon_fold::enter(window);
        if crate::ribbon_fold::close_panel() {
            cx.notify();
            return;
        }
        // A tool of a feature waiting for a click on the sheet.
        if features::cancel(self, cx) {
            return;
        }
        if self.object_sel.is_some() {
            self.deselect_object(cx);
            return;
        }
        self.clip = None;
        self.object_clip = None;
        self.painter = None;
        self.popup = None;
        self.status_message = None;
        cx.notify();
    }
    pub fn on_insert_rows(&mut self, _: &InsertRows, _: &mut Window, cx: &mut Context<Self>) {
        if self.sel().current().is_whole_cols() {
            self.insert_cols(cx);
        } else {
            self.insert_rows(cx);
        }
    }
    pub fn on_delete_rows(&mut self, _: &DeleteRows, _: &mut Window, cx: &mut Context<Self>) {
        if self.sel().current().is_whole_cols() {
            self.delete_cols(cx);
        } else {
            self.delete_rows(cx);
        }
    }

    // Shortcuts of commands that live in the feature modules.
    pub fn on_format_cells(
        &mut self,
        _: &FormatCells,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_command(features::format_cells::Cmd::Open, window, cx);
    }
    pub fn on_print(&mut self, _: &Print, window: &mut Window, cx: &mut Context<Self>) {
        self.run_command(features::page::Cmd::Print, window, cx);
    }
    pub fn on_toggle_filter(
        &mut self,
        _: &ToggleFilter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_command(features::filters::Cmd::Toggle, window, cx);
    }
    pub fn on_insert_table(
        &mut self,
        _: &InsertTable,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_command(features::tables::Cmd::Insert, window, cx);
    }
    pub fn on_new_note(&mut self, _: &NewNote, window: &mut Window, cx: &mut Context<Self>) {
        self.run_command(features::comments::Cmd::NewNote, window, cx);
    }
    pub fn on_new_comment(&mut self, _: &NewComment, window: &mut Window, cx: &mut Context<Self>) {
        self.run_command(features::comments::Cmd::NewComment, window, cx);
    }
    pub fn on_name_manager(
        &mut self,
        _: &NameManager,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_command(features::names::Cmd::Manager, window, cx);
    }
    /// Alt+Down: the drop-down a feature hangs from the active cell.
    pub fn on_open_drop_down(
        &mut self,
        _: &OpenDropDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editing.is_none() {
            features::cell_drop_down(self, window, cx);
        }
    }
    pub fn on_insert_link(&mut self, _: &InsertLink, window: &mut Window, cx: &mut Context<Self>) {
        self.run_command(features::edit_tools::Cmd::Link, window, cx);
    }

    /// Run a feature command from the keyboard or the menu bar: an entry
    /// being typed is stored first, and nothing happens behind a dialog.
    pub fn run_command<C: Command>(&mut self, cmd: C, window: &mut Window, cx: &mut Context<Self>) {
        if self.modal_open() || (self.editing.is_some() && !self.commit_edit(None, window, cx)) {
            return;
        }
        if cmd.enabled(self) {
            self.popup = None;
            cmd.run(self, window, cx);
        }
    }
}
