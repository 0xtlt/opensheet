//! How a feature plugs its user interface into the application.
//!
//! Every feature owns one file of this directory and nothing else: the
//! central files (`app.rs`, `view.rs`, `ribbon.rs`, `grid.rs`, `shell.rs`,
//! `objects.rs`) already call into it at the places listed below, so a
//! feature is written without touching them. This file is the contract.
//!
//! # The two things a module provides
//!
//! * **A [`Feature`]**: a unit struct listed in [`all`]. Each method of the
//!   trait is a hook the central code calls for every feature; all of them
//!   default to "nothing to say", so a module implements only what it uses.
//! * **Commands**: a `Cmd` enum (declared with `commands!`) implementing
//!   [`Command`]. The ribbon, the menus and the keyboard shortcuts already
//!   run them (`Cmd::Toggle.run(app, window, cx)`); a command that is not
//!   written yet ends in `app.not_implemented(self.label(), cx)`. A command
//!   that opens a drop-down builds a `Vec<MenuEntry>` (`MenuEntry::command`,
//!   `MenuEntry::submenu`) and calls `app.open_command_menu`. A label that
//!   follows the workbook ("Unprotect Sheet") is given by `label_in`.
//!
//! State a feature keeps between events lives in `app.ext_mut::<MyState>()`
//! (any `Default` type, created on first use, dropped when another workbook
//! is opened; `app.ext::<MyState>()` reads it). The workbook itself is
//! `app.wb`: the engine functions take the undo snapshot themselves and the
//! feature ends with `app.commit(label, cx)`.
//!
//! # What the hooks cover
//!
//! | Need | Hook / API |
//! |---|---|
//! | Ribbon button, menu entry, shortcut | [`Command`] |
//! | Contextual ribbon tab ("Table Design") | [`Feature::context_tabs`] + [`Feature::render_tab`], built with the `pub` helpers of `ribbon.rs`: `group`, `launcher_group`, `rows`, `row`, `sep`, `app.cmd_large`, `app.cmd_labeled`, `app.cmd_icon`, `app.cmd_check`, `app.combo`, `app.arrange_group` |
//! | Modal dialog | `app.open_modal(window, cx, \|modal, window, cx\| MyDialog::new(modal, ..))`; the view is drawn with `widgets::dialog(&self.modal, title)` and reaches the application with `modal.update_app` / `modal.read_app` |
//! | Message box | `app.alert(Alert::new(title, text).warning().button(label, \|app, window, cx\| ..), cx)` |
//! | Side pane right of the grid | `app.open_pane(id, title, view, cx)`, `app.close_pane(id, cx)`, `app.pane_open(id)` |
//! | Drop-down hanging from a cell | `app.open_popup_view(app.cell_anchor(rect), view, cx)`, `app.close_popup(window, cx)` |
//! | Cell appearance | the grid paints `Workbook::display` (table, pivot and conditional styles); [`condfmt::paint_adornment`] draws data bars and icons |
//! | Drawing under / over the cells | [`Feature::paint`] with a [`Painter`], once per pane and [`PaintLayer`] |
//! | Clickable things drawn on the grid | [`Painter::zone`], then [`Feature::zone_mouse_down`], [`Feature::zone_drag`], [`Feature::zone_mouse_up`], [`Feature::zone_hover`]; cursor and tooltip are set on the [`Zone`] |
//! | Real elements over the grid (a note being typed) | [`Feature::overlay_elements`] |
//! | Refusing a change (protection, pivot cells) | [`Feature::guard`] with an [`Intent`]; a feature asks before its own changes with `app.may(intent, cx)` |
//! | Refusing a selection (protection) | [`Feature::can_select`] |
//! | Where the arrows, Tab and Enter lead (a protected sheet walks its unlocked cells) | [`Feature::next_cell`] |
//! | Checking an entry (validation) | [`Feature::may_commit`] → [`Verdict`]; the alert's buttons call `app.commit_edit_unchecked`, `app.focus_editor` or `app.cancel_edit` |
//! | Following edits | [`Feature::cells_written`] (same undo step), [`Feature::cells_changed`] (afterwards), [`Feature::selection_changed`] |
//! | Alt+Down on a cell | [`Feature::cell_drop_down`] |
//! | Context menus | [`Feature::context_menu`] with a [`MenuTarget`] |
//! | Status bar | [`Feature::status_items`] |
//! | Pictures and shapes | [`Feature::objects`] and the `*_object` hooks; selecting, moving, resizing, nudging, deleting, copy / paste and stacking order come from `objects.rs` |
//! | Objects that are not their frame (a line, a turned shape) | [`Feature::object_hit`], [`Feature::object_extent`], [`Feature::object_own_handles`] |
//! | Typing, Home-tab formatting, Escape and Paste while an object is selected | [`Feature::object_typed`], [`Feature::object_style`], [`Feature::cancel`], [`Feature::object_clipboard`], [`Feature::paste_clipboard`] |
//! | Page Layout view / Page Break Preview | `app.view_mode()`, [`Feature::view_mode_changed`], [`Feature::adjust_layout`] (`Axis::set_gaps` leaves room between pages) |
//!
//! Row heights and the arrow keys already skip rows a filter hides
//! (`Sheet::rows_not_shown`), the formula bar shows
//! `Workbook::formula_bar_text`, and references are coloured while typing
//! with `parser::scan_refs_in` (tables, names and `A1#` included).
//!
//! # Rules
//!
//! * A hook runs for every feature: start by returning when the event is
//!   not yours (another layer, a zone whose payload is of another type, an
//!   object another feature owns).
//! * `paint` runs at every repaint: draw only what `p.visible` shows and
//!   read caches rather than recomputing.
//! * GPUI forbids nested `deferred` elements. Dialogs, panes and
//!   `open_popup_view` are not deferred, so widgets that open a list
//!   (`Select`, `ComboBox`, `ColorButton`) work in them; never wrap a view
//!   holding such widgets in `deferred` yourself.
//! * User-visible strings are `t("English", "Français")`.
//!
//! # A worked example
//!
//! A feature that marks cells holding a note with a red corner, shows the
//! note in a tooltip and edits it in a dialog on double-click:
//!
//! ```ignore
//! pub struct Notes;
//!
//! commands! {
//!     /// What the user can ask of notes.
//!     pub enum Cmd {
//!         Edit = "comment", t("Edit Note", "Modifier la note");
//!     }
//! }
//!
//! impl Command for Cmd {
//!     fn label(self) -> &'static str { self.face().1 }
//!     fn icon(self) -> &'static str { self.face().0 }
//!     fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
//!         // A protected sheet answers through `guard` and the user is told why.
//!         if !app.may(Intent::EditComment, cx) { return; }
//!         let (row, col) = app.active();
//!         let text = app.wb.comment_at(app.sheet, row, col)
//!             .and_then(|c| c.as_note()).map(|n| n.text()).unwrap_or_default();
//!         app.open_modal(window, cx, move |modal, _, cx| NoteDialog::new(modal, row, col, text, cx));
//!     }
//! }
//!
//! /// What a zone of this module stands for.
//! struct Corner(u32, u32);
//!
//! impl Feature for Notes {
//!     fn name(&self) -> &'static str { "notes" }
//!
//!     fn paint(&self, p: &mut Painter) {
//!         if p.layer != PaintLayer::Over { return; }
//!         let sheet = &p.app.wb.sheets[p.app.sheet];
//!         for ((row, col), comment) in sheet.comments.range((p.visible.r0, 0)..=(p.visible.r1, u32::MAX)) {
//!             let Some(note) = comment.as_note() else { continue };
//!             if !p.visible.contains(*row, *col) { continue; }
//!             let (_, y0, x1, _) = p.cell_box(Rect::cell(*row, *col));
//!             p.polygon(&[(x1 - 7.0, y0), (x1 - 1.0, y0), (x1 - 1.0, y0 + 6.0)], c(0xC00000));
//!             p.zone((x1 - 8.0, y0, x1, y0 + 8.0), Corner(*row, *col)).tooltip(note.text());
//!         }
//!     }
//!
//!     fn zone_mouse_down(&self, app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) -> ZoneResponse {
//!         let Some(Corner(row, col)) = zone.payload::<Corner>() else { return ZoneResponse::Ignored };
//!         if ev.click_count < 2 { return ZoneResponse::Ignored; } // a single click selects the cell
//!         app.select_cell(*row, *col, cx);
//!         Cmd::Edit.run(app, window, cx);
//!         ZoneResponse::Handled
//!     }
//!
//!     fn context_menu(&self, app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
//!         if matches!(target, MenuTarget::Cell(..)) {
//!             menu.add(MenuSection::Review, MenuEntry::command(Cmd::Edit, app));
//!         }
//!     }
//!
//!     // Pasted cells bring their notes along, in the undo step of the paste.
//!     fn cells_written(&self, app: &mut ExcelApp, edit: &CellEdit, _cx: &mut Cx) {
//!         if let ChangeCause::Paste { from: Some((sheet, rect)), cut, mode: PasteMode::All } = edit.cause {
//!             let (r0, c0) = (edit.rect.r0, edit.rect.c0);
//!             if cut { app.wb.move_comments(sheet, rect, edit.sheet, r0, c0) }
//!             else { app.wb.copy_comments(sheet, rect, edit.sheet, r0, c0) }
//!         }
//!     }
//! }
//!
//! struct NoteDialog { modal: Modal, row: u32, col: u32, text: Entity<TextInput> }
//!
//! impl NoteDialog {
//!     fn new(modal: Modal, row: u32, col: u32, text: String, cx: &mut Context<Self>) -> Self {
//!         NoteDialog { modal, row, col, text: widgets::input(cx, text) }
//!     }
//!     fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
//!         let (row, col, text) = (self.row, self.col, self.text.read(cx).text().to_string());
//!         self.modal.update_app(cx, |app, cx| {
//!             let sheet = app.sheet;
//!             app.wb.set_note(sheet, row, col, "Me", &text);
//!             app.commit(t("Edit Note", "Modifier la note"), cx);
//!         });
//!         self.modal.close(window, cx);
//!     }
//! }
//!
//! impl Render for NoteDialog {
//!     fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
//!         widgets::dialog(&self.modal, t("Edit Note", "Modifier la note"))
//!             .width(360.0)
//!             .on_ok(cx.listener(Self::ok))
//!             .child(div().flex().child(widgets::TextField::new(&self.text)))
//!             .footer(widgets::ok_cancel(&self.modal, cx.listener(Self::ok)))
//!     }
//! }
//! ```
//!
//! Working code to read next: `format_cells.rs` (a full dialog),
//! `charts.rs` (floating objects, a contextual tab, a context menu, an
//! element over the grid, state in `app.ext`), and `widgets/gallery.rs`
//! (every widget, a pane, a drop-down, a message box, a zone; shown by
//! `opensheet --widgets`).

use crate::app::{ExcelApp, MenuEntry};
use crate::grid::{Draw, GridLayout};
use crate::theme::c;
use gpui::{
    AnyElement, BorderStyle, Bounds, ClipboardItem, Context, CursorStyle, Font, FontStyle, FontWeight, Hsla, Modifiers,
    MouseButton, PathBuilder, Pixels, Point, ShapedLine, SharedString, TextRun, Window, WindowTextSystem, fill, px, quad,
    size,
};
use opensheet_engine::{Rect, Style};
use opensheet_engine::ops::{Direction, PasteMode};
use opensheet_engine::workbook::ClearMode;
use std::any::Any;
use std::rc::Rc;

/// Declares the commands of a feature: for each one, an icon of `icons.rs`
/// (`""` for none) and the label shown on the ribbon and in menus.
///
/// ```ignore
/// commands! {
///     /// What the user can ask of AutoFilter.
///     pub enum Cmd {
///         Toggle = "filter", t("Filter", "Filtrer");
///         Clear = "clear", t("Clear", "Effacer");
///     }
/// }
///
/// impl Command for Cmd {
///     fn label(self) -> &'static str { self.face().1 }
///     fn icon(self) -> &'static str { self.face().0 }
///     fn run(self, app: &mut ExcelApp, _: &mut Window, cx: &mut Cx) { app.not_implemented(self.label(), cx) }
/// }
/// ```
macro_rules! commands {
    ($(#[$meta:meta])* pub enum $name:ident { $($variant:ident = $icon:literal, $label:expr;)+ }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name {
            $($variant,)+
        }

        impl $name {
            /// Icon and label of the command.
            fn face(self) -> (&'static str, &'static str) {
                match self {
                    $($name::$variant => ($icon, $label),)+
                }
            }
        }
    };
}

pub mod chart_format;
pub mod charts;
pub mod comments;
pub mod condfmt;
pub mod data_tools;
pub mod drawings;
pub mod edit_tools;
pub mod filters;
pub mod format_cells;
pub mod misc;
pub mod names;
pub mod outline;
pub mod page;
pub mod pivot;
pub mod protection;
pub mod tables;
pub mod validation;

/// The context every hook that may change the application receives.
pub type Cx<'a> = Context<'a, ExcelApp>;

/// Every feature, in the order hooks are called. Floating objects are
/// drawn in this order too (the objects of later features on top).
pub fn all() -> &'static [&'static dyn Feature] {
    static ALL: [&dyn Feature; 18] = [
        &format_cells::FormatCells,
        &filters::Filters,
        &tables::Tables,
        &condfmt::CondFormats,
        &validation::Validation,
        &comments::Comments,
        &protection::Protection,
        &page::Page,
        &pivot::Pivots,
        &charts::Charts,
        &chart_format::ChartFormat,
        &drawings::Drawings,
        &names::Names,
        &edit_tools::EditTools,
        &data_tools::DataTools,
        &outline::Outline,
        &misc::Misc,
        &crate::widgets::gallery::GalleryMarks,
    ];
    &ALL
}

/// Position of a feature in [`all`] (what [`Zone::owner`] and
/// [`ObjectId::owner`] hold).
pub fn index_of(feature: &dyn Feature) -> usize {
    let name = feature.name();
    all().iter().position(|f| f.name() == name).unwrap_or(0)
}

/// The id of one of `feature`'s floating objects.
pub fn object_id(feature: &dyn Feature, id: u32) -> ObjectId {
    ObjectId {
        owner: index_of(feature),
        id,
    }
}

// ----------------------------------------------------------------- commands

/// Something the user can ask for from the ribbon, a menu or the keyboard.
///
/// ```ignore
/// #[derive(Clone, Copy)]
/// pub enum Cmd { Toggle, Clear }
///
/// impl Command for Cmd {
///     fn label(self) -> &'static str {
///         match self {
///             Cmd::Toggle => t("Filter", "Filtrer"),
///             Cmd::Clear => t("Clear", "Effacer"),
///         }
///     }
///     fn run(self, app: &mut ExcelApp, _: &mut Window, cx: &mut Cx) {
///         app.not_implemented(self.label(), cx)
///     }
/// }
/// ```
pub trait Command: Copy + 'static {
    /// Text on the ribbon and in menus.
    fn label(self) -> &'static str;

    /// The text when it depends on the state of the workbook ("Protect
    /// Sheet" becomes "Unprotect Sheet").
    fn label_in(self, _app: &ExcelApp) -> &'static str {
        self.label()
    }

    /// Name of an icon of `icons.rs`; `""` for none.
    fn icon(self) -> &'static str {
        ""
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx);

    /// Greyed out when false.
    fn enabled(self, _app: &ExcelApp) -> bool {
        true
    }

    /// Shown pressed on the ribbon and ticked in menus.
    fn checked(self, _app: &ExcelApp) -> bool {
        false
    }
}

// ------------------------------------------------------------------- ribbon

/// A ribbon tab shown only while its object is active.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextTab {
    /// Unique among all tabs; passed back to [`Feature::render_tab`].
    pub id: &'static str,
    pub title: &'static str,
}

// ------------------------------------------------------------------ editing

/// A change about to be made to the current sheet (or to the workbook),
/// submitted to [`Feature::guard`]. The variants are those of the engine's
/// `protect::Action`, so the protection feature answers most of them with
/// `Workbook::permits`.
#[derive(Clone, Copy, Debug, PartialEq)]
// Central code asks the questions of its own operations; the others are
// there for the feature modules to ask before theirs.
#[allow(dead_code)]
pub enum Intent {
    /// Typing into, pasting over, filling or clearing these cells.
    EditCells(Rect),
    /// Changing the formatting of these cells.
    FormatCells(Rect),
    /// Changing the width or the visibility of columns / rows.
    FormatColumns,
    FormatRows,
    InsertColumns { at: u32, count: u32 },
    InsertRows { at: u32, count: u32 },
    /// Deleting the columns / rows `first..=last`.
    DeleteColumns { first: u32, last: u32 },
    DeleteRows { first: u32, last: u32 },
    Sort(Rect),
    /// Choosing what a filter shows with its drop-down buttons.
    UseAutoFilter,
    /// Putting the filter buttons on a range or taking them off.
    ToggleAutoFilter,
    /// Changing, refreshing or creating a pivot table.
    UsePivotTable,
    /// Adding (`None`), changing, moving or deleting a chart, a picture or
    /// a shape.
    EditObject(Option<ObjectId>),
    /// Adding, changing or deleting a note or a comment.
    EditComment,
    /// Creating, resizing or restyling a table.
    ChangeTable,
    /// What a protected sheet never allows: merging cells, locking or
    /// unlocking them, data validation, conditional formats.
    ChangeSheet,
    /// Changing the colour of the sheet tab, which neither a protected
    /// sheet nor a protected workbook allows.
    TabColor,
    InsertSheet,
    DeleteSheet,
    RenameSheet,
    MoveSheet,
    CopySheet,
    HideSheet,
    UnhideSheet,
}

/// An entry waiting to be stored, submitted to [`Feature::may_commit`].
#[derive(Clone, Debug)]
// A record for the feature modules: central code only fills it in.
#[allow(dead_code)]
pub struct PendingEdit<'a> {
    pub sheet: usize,
    pub row: u32,
    pub col: u32,
    /// The text as typed.
    pub text: &'a str,
    /// Where the active cell goes afterwards (pass it on to
    /// `ExcelApp::commit_edit_unchecked`).
    pub advance: Option<Direction>,
}

/// Answer of [`Feature::may_commit`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    Accept,
    /// The entry is not stored and the cell stays in edit mode. The
    /// feature has shown (or will show) an alert whose buttons call
    /// `commit_edit_unchecked` ("Yes"), give the editor the focus back
    /// ("Retry") or `cancel_edit` ("Cancel").
    Hold,
}

/// How cells were changed, for [`Feature::cells_written`] and
/// [`Feature::cells_changed`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ChangeCause {
    Typing,
    /// `from`: the cells that were copied or cut (sheet, range) when they
    /// come from this workbook; `cut`: they were moved, not copied.
    Paste {
        from: Option<(usize, Rect)>,
        cut: bool,
        mode: PasteMode,
    },
    Fill,
    Clear(ClearMode),
}

/// Cells the user just changed.
#[derive(Clone, Copy, Debug, PartialEq)]
// A record for the feature modules: central code only fills it in.
#[allow(dead_code)]
pub struct CellEdit {
    pub sheet: usize,
    pub rect: Rect,
    pub cause: ChangeCause,
}

// -------------------------------------------------------------------- menus

/// What a context menu was opened on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MenuTarget {
    Cell(u32, u32),
    RowHeader(u32),
    ColHeader(u32),
    SheetTab(usize),
    Object(ObjectId),
}

/// Where an entry goes in a context menu; sections appear in this order,
/// separated by a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MenuSection {
    /// Cut, Copy, Paste.
    Clipboard,
    /// Insert, Delete, Clear Contents, Rename.
    Edit,
    /// Filter, Sort, tables, pivot tables.
    Data,
    /// Comments and notes.
    Review,
    /// Format Cells, sizes, Hide / Unhide, protection, tab colour.
    Format,
    /// Names, links and whatever fits nowhere else.
    Extra,
}

/// A context menu being assembled.
#[derive(Default)]
pub struct ContextMenu {
    sections: Vec<(MenuSection, Vec<MenuEntry>)>,
}

impl ContextMenu {
    pub fn add(&mut self, section: MenuSection, entry: MenuEntry) {
        match self.sections.iter_mut().find(|(s, _)| *s == section) {
            Some((_, entries)) => entries.push(entry),
            None => self.sections.push((section, vec![entry])),
        }
    }

    /// The entries, section after section.
    pub fn build(mut self) -> Vec<MenuEntry> {
        self.sections.sort_by_key(|(s, _)| *s);
        let mut out: Vec<MenuEntry> = Vec::new();
        for (_, entries) in self.sections {
            if entries.is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push(MenuEntry::separator());
            }
            out.extend(entries);
        }
        out
    }
}

/// A piece of text on the status bar, after the mode ("Ready").
pub struct StatusItem {
    pub text: SharedString,
    pub tooltip: Option<SharedString>,
    pub on_click: Option<crate::app::MenuAction>,
}

impl StatusItem {
    pub fn new(text: impl Into<SharedString>) -> StatusItem {
        StatusItem {
            text: text.into(),
            tooltip: None,
            on_click: None,
        }
    }
}

// --------------------------------------------------------- floating objects

/// A chart, picture or shape: `owner` is the feature it belongs to
/// ([`index_of`]), `id` the feature's own identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ObjectId {
    pub owner: usize,
    pub id: u32,
}

/// Where a floating object sits, in the terms the engine stores it: a cell,
/// an offset inside that cell and a size, in pixels at 100 %.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// Cell holding the top-left corner (row, col).
    pub anchor: (u32, u32),
    /// Offset of the corner inside that cell (x, y).
    pub offset: (f32, f32),
    /// Width and height.
    pub size: (f32, f32),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObjectInfo {
    pub id: u32,
    pub place: Placement,
    /// Smallest size the handles can give it.
    pub min_size: (f32, f32),
    /// Corner handles keep the proportions (pictures).
    pub keep_aspect: bool,
}

/// What was done to an object, for the label of the undo step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectVerb {
    Move,
    Resize,
    Delete,
    Paste,
    Reorder,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZMove {
    ToFront,
    Forward,
    Backward,
    ToBack,
}

// -------------------------------------------------------------------- views

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ViewMode {
    #[default]
    Normal,
    PageLayout,
    PageBreakPreview,
}

// ----------------------------------------------------------------- painting

/// When a [`Feature::paint`] call draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaintLayer {
    /// Before anything of the pane: under gridlines, fills and text.
    Under,
    /// Over the cells and the selection of the pane, under charts and
    /// other floating objects.
    Over,
    /// Over everything but the scrollbars, headers included; called once
    /// with the whole grid as its clip.
    Top,
}

/// An area of the grid that reacts to the mouse, registered while painting.
#[derive(Clone)]
pub struct Zone {
    /// The feature that registered it.
    pub owner: usize,
    /// x0, y0, x1, y1 in grid-element coordinates.
    pub bounds: (f32, f32, f32, f32),
    /// Whatever the feature needs to recognise it.
    pub payload: Rc<dyn Any>,
    pub cursor: Option<CursorStyle>,
    /// Shown after the pointer rested on the zone.
    pub tooltip: Option<SharedString>,
    pub layer: PaintLayer,
}

impl Zone {
    pub fn payload<T: 'static>(&self) -> Option<&T> {
        self.payload.downcast_ref::<T>()
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.bounds.0 && x < self.bounds.2 && y >= self.bounds.1 && y < self.bounds.3
    }

    pub fn cursor(&mut self, cursor: CursorStyle) -> &mut Zone {
        self.cursor = Some(cursor);
        self
    }

    pub fn tooltip(&mut self, text: impl Into<SharedString>) -> &mut Zone {
        self.tooltip = Some(text.into());
        self
    }
}

/// A mouse event on a [`Zone`] or on a floating object.
#[derive(Clone, Debug)]
// A record for the feature modules: central code only fills it in.
#[allow(dead_code)]
pub struct ZoneEvent {
    /// Window coordinates.
    pub position: Point<Pixels>,
    /// Grid-element coordinates.
    pub x: f32,
    pub y: f32,
    /// The cell under the pointer (clamped to the visible cells).
    pub cell: (u32, u32),
    pub button: MouseButton,
    pub modifiers: Modifiers,
    pub click_count: usize,
}

/// Answer of [`Feature::zone_mouse_down`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoneResponse {
    /// As if the zone were not there: the click goes to the cell under it.
    Ignored,
    Handled,
    /// Handled, and the moves and the release of this press go to
    /// [`Feature::zone_drag`] and [`Feature::zone_mouse_up`].
    Capture,
}

/// Collects what a feature draws on the grid and the zones it registers.
/// Coordinates are those of the grid element (use [`Painter::cell_box`] and
/// [`Painter::anchored`] to get them from cells).
pub struct Painter<'a> {
    pub app: &'a ExcelApp,
    pub layout: &'a GridLayout,
    pub ts: &'a WindowTextSystem,
    pub layer: PaintLayer,
    /// The cells of the pane being painted.
    pub visible: Rect,
    /// Clip rectangle of the pane (x0, y0, x1, y1).
    pub clip: (f32, f32, f32, f32),
    /// Device pixels per logical pixel (for hairlines).
    #[allow(dead_code)] // no feature draws hairlines yet
    pub scale: f32,
    pub(crate) owner: usize,
    pub(crate) items: Vec<Draw>,
    pub(crate) zones: Vec<Zone>,
}

// The drawing helpers are for the feature modules; some have no caller yet.
#[allow(dead_code)]
impl<'a> Painter<'a> {
    pub(crate) fn new(
        app: &'a ExcelApp,
        layout: &'a GridLayout,
        ts: &'a WindowTextSystem,
        layer: PaintLayer,
        visible: Rect,
        clip: (f32, f32, f32, f32),
        scale: f32,
    ) -> Painter<'a> {
        Painter {
            app,
            layout,
            ts,
            layer,
            visible,
            clip,
            scale,
            owner: 0,
            items: Vec::new(),
            zones: Vec::new(),
        }
    }

    pub fn zoom(&self) -> f32 {
        self.layout.zoom
    }

    /// Pixel box (x0, y0, x1, y1) of a range of cells.
    pub fn cell_box(&self, rect: Rect) -> (f32, f32, f32, f32) {
        self.layout.rect_px(rect)
    }

    /// Pixel position of a point given as a cell and an offset inside it in
    /// pixels at 100 % (how the engine anchors floating things).
    pub fn anchored(&self, anchor: (u32, u32), offset: (f32, f32)) -> (f32, f32) {
        let z = self.layout.zoom;
        (
            self.layout.col_x(anchor.1) + offset.0 * z,
            self.layout.row_y(anchor.0) + offset.1 * z,
        )
    }

    fn bounds(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Bounds<Pixels> {
        Bounds::new(
            self.layout.to_window(x0.min(x1), y0.min(y1)),
            size(px((x1 - x0).abs()), px((y1 - y0).abs())),
        )
    }

    pub fn push(&mut self, draw: Draw) {
        self.items.push(draw);
    }

    pub fn extend(&mut self, draws: impl IntoIterator<Item = Draw>) {
        self.items.extend(draws);
    }

    /// A filled rectangle.
    pub fn rect(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, color: Hsla) {
        self.items.push(Draw::Quad(fill(self.bounds(x0, y0, x1, y1), color)));
    }

    /// A rectangle outline drawn inside the box.
    pub fn frame(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, width: f32, color: Hsla) {
        self.items.push(Draw::Quad(quad(
            self.bounds(x0, y0, x1, y1),
            px(0.),
            gpui::transparent_black(),
            px(width),
            color,
            BorderStyle::Solid,
        )));
    }

    /// A rounded, optionally outlined box.
    pub fn rounded(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, radius: f32, background: Hsla, border: Option<Hsla>) {
        self.items.push(Draw::Quad(quad(
            self.bounds(x0, y0, x1, y1),
            px(radius),
            background,
            px(if border.is_some() { 1.0 } else { 0.0 }),
            border.unwrap_or(gpui::transparent_black()),
            BorderStyle::Solid,
        )));
    }

    /// A horizontal or vertical dashed line, `width` pixels thick.
    pub fn dashed(&mut self, from: (f32, f32), to: (f32, f32), width: f32, dash: f32, gap: f32, color: Hsla) {
        let horizontal = (to.1 - from.1).abs() < (to.0 - from.0).abs();
        let (mut p, end) = if horizontal { (from.0.min(to.0), from.0.max(to.0)) } else { (from.1.min(to.1), from.1.max(to.1)) };
        // Only the part in view: a line may run down a million rows.
        let (lo, hi) = if horizontal { (self.clip.0, self.clip.2) } else { (self.clip.1, self.clip.3) };
        if p < lo {
            p += ((lo - p) / (dash + gap)).floor() * (dash + gap);
        }
        let end = end.min(hi);
        while p < end {
            let e = (p + dash).min(end);
            if horizontal {
                self.rect(p, from.1, e, from.1 + width, color);
            } else {
                self.rect(from.0, p, from.0 + width, e, color);
            }
            p += dash + gap;
        }
    }

    pub fn polygon(&mut self, points: &[(f32, f32)], color: Hsla) {
        if points.len() < 3 {
            return;
        }
        let pts: Vec<Point<Pixels>> = points.iter().map(|(x, y)| self.layout.to_window(*x, *y)).collect();
        let mut builder = PathBuilder::fill();
        builder.add_polygon(&pts, true);
        if let Ok(path) = builder.build() {
            self.items.push(Draw::Path(path, color));
        }
    }

    pub fn polyline(&mut self, points: &[(f32, f32)], width: f32, color: Hsla) {
        if points.len() < 2 {
            return;
        }
        let mut builder = PathBuilder::stroke(px(width));
        builder.move_to(self.layout.to_window(points[0].0, points[0].1));
        for (x, y) in &points[1..] {
            builder.line_to(self.layout.to_window(*x, *y));
        }
        if let Ok(path) = builder.build() {
            self.items.push(Draw::Path(path, color));
        }
    }

    pub fn disc(&mut self, cx: f32, cy: f32, radius: f32, color: Hsla) {
        let n = 20;
        let pts: Vec<(f32, f32)> = (0..n)
            .map(|i| {
                let a = i as f32 / n as f32 * std::f32::consts::TAU;
                (cx + radius * a.cos(), cy + radius * a.sin())
            })
            .collect();
        self.polygon(&pts, color);
    }

    /// Shape one line of text in the interface font.
    pub fn shape(&self, text: &str, size: f32, color: Hsla, bold: bool) -> ShapedLine {
        self.shape_with(text, ui_font(bold), size, color)
    }

    pub fn shape_with(&self, text: &str, font: Font, size: f32, color: Hsla) -> ShapedLine {
        let text: SharedString = text.replace(['\n', '\r'], " ").into();
        let run = TextRun {
            len: text.len(),
            font,
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        self.ts.shape_line(text, px(size), &[run], None)
    }

    /// Draw a shaped line with its top-left corner at (x, y), clipped to
    /// the pane.
    pub fn put(&mut self, line: ShapedLine, x: f32, y: f32, size: f32) {
        let clip = self.bounds(self.clip.0, self.clip.1, self.clip.2, self.clip.3);
        self.items.push(Draw::Text {
            line,
            origin: self.layout.to_window(x.round(), y.round()),
            line_height: px((size * 1.25).round()),
            clip,
        });
    }

    /// Draw text in the interface font; returns its width.
    pub fn text(&mut self, text: &str, x: f32, y: f32, size: f32, color: Hsla) -> f32 {
        let line = self.shape(text, size, color, false);
        let w = f32::from(line.width);
        self.put(line, x, y, size);
        w
    }

    /// The coloured outline and tint of a range a formula or a chart refers
    /// to.
    pub fn highlight_range(&mut self, rect: Rect, color: u32) {
        if !rect.intersects(&self.visible) {
            return;
        }
        let (x0, y0, x1, y1) = self.layout.rect_px(rect);
        self.rect(x0, y0, x1 - 1.0, y1 - 1.0, crate::theme::ca(color, 0.10));
        self.frame(x0 - 1.0, y0 - 1.0, x1, y1, 2.0, c(color));
    }

    /// Make a box react to the mouse. The part outside the pane is cut off.
    /// Chain `.cursor(..)` and `.tooltip(..)` on the result.
    pub fn zone(&mut self, bounds: (f32, f32, f32, f32), payload: impl Any) -> &mut Zone {
        let clipped = (
            bounds.0.max(self.clip.0),
            bounds.1.max(self.clip.1),
            bounds.2.min(self.clip.2),
            bounds.3.min(self.clip.3),
        );
        self.zones.push(Zone {
            owner: self.owner,
            bounds: clipped,
            payload: Rc::new(payload),
            cursor: None,
            tooltip: None,
            layer: self.layer,
        });
        self.zones.last_mut().unwrap()
    }
}

/// The interface font, for labels drawn on the grid.
pub fn ui_font(bold: bool) -> Font {
    Font {
        family: ".SystemUIFont".into(),
        features: Default::default(),
        fallbacks: None,
        weight: if bold { FontWeight::SEMIBOLD } else { FontWeight::NORMAL },
        style: FontStyle::Normal,
    }
}

// -------------------------------------------------------------------- hooks

/// The hooks of a feature. Hooks are called for every feature of [`all`],
/// in order; each default does nothing.
pub trait Feature: Sync + 'static {
    /// A unique short name ("filters").
    fn name(&self) -> &'static str;

    // ---- ribbon ---------------------------------------------------------

    /// Contextual tabs to show right now (after "Help").
    fn context_tabs(&self, _app: &ExcelApp) -> Vec<ContextTab> {
        Vec::new()
    }

    /// The groups of one of this feature's contextual tabs.
    fn render_tab(&self, _app: &mut ExcelApp, _tab: &'static str, _window: &mut Window, _cx: &mut Cx) -> Vec<AnyElement> {
        Vec::new()
    }

    // ---- grid -----------------------------------------------------------

    /// Draw on the grid and register zones. Called for each pane (the
    /// sheet has up to four when panes are frozen) with `Under` then
    /// `Over`, and once with `Top`. Runs at every repaint: keep it cheap.
    fn paint(&self, _p: &mut Painter) {}

    /// Change how the sheet is laid out (page gaps in Page Layout view).
    /// Called whenever a layout is computed, in every view mode.
    fn adjust_layout(&self, _app: &ExcelApp, _layout: &mut GridLayout) {}

    /// Elements positioned over the grid (absolute, in grid-element
    /// coordinates), such as the box in which a note is typed.
    fn overlay_elements(&self, _app: &mut ExcelApp, _layout: &GridLayout, _window: &mut Window, _cx: &mut Cx) -> Vec<AnyElement> {
        Vec::new()
    }

    /// A press on one of this feature's zones (any button; a double click
    /// arrives with `click_count == 2`).
    fn zone_mouse_down(&self, _app: &mut ExcelApp, _zone: &Zone, _ev: &ZoneEvent, _window: &mut Window, _cx: &mut Cx) -> ZoneResponse {
        ZoneResponse::Ignored
    }

    /// The pointer moved during a press that answered `Capture`.
    fn zone_drag(&self, _app: &mut ExcelApp, _zone: &Zone, _ev: &ZoneEvent, _window: &mut Window, _cx: &mut Cx) {}

    /// The press that answered `Capture` ended.
    fn zone_mouse_up(&self, _app: &mut ExcelApp, _zone: &Zone, _ev: &ZoneEvent, _window: &mut Window, _cx: &mut Cx) {}

    /// The pointer entered one of this feature's zones (`Some`) or left
    /// the one it was on (`None`).
    fn zone_hover(&self, _app: &mut ExcelApp, _zone: Option<&Zone>, _cx: &mut Cx) {}

    // ---- editing --------------------------------------------------------

    /// Refuse a change: return the message to show (protected sheet, pivot
    /// table cell). `None` lets it happen.
    fn guard(&self, _app: &ExcelApp, _intent: &Intent) -> Option<SharedString> {
        None
    }

    /// May this entry be stored in its cell?
    fn may_commit(&self, _app: &mut ExcelApp, _edit: &PendingEdit, _window: &mut Window, _cx: &mut Cx) -> Verdict {
        Verdict::Accept
    }

    /// The user just changed cells (typed, pasted, filled, cleared) and
    /// the change is not committed yet: what the feature changes in
    /// `app.wb` now joins the same undo step. This is where a table grows
    /// under what was typed (`Workbook::tables_after_edit`) and where
    /// pasted cells bring their comments and validation along.
    fn cells_written(&self, _app: &mut ExcelApp, _edit: &CellEdit, _cx: &mut Cx) {}

    /// The same change, once committed. What the feature does now is an
    /// undo step of its own (`snapshot`, mutate, `app.commit`).
    fn cells_changed(&self, _app: &mut ExcelApp, _edit: &CellEdit, _cx: &mut Cx) {}

    /// The selection or the current sheet changed.
    fn selection_changed(&self, _app: &mut ExcelApp, _cx: &mut Cx) {}

    /// Where the active cell goes from `from` when the feature has its own
    /// idea of the next cell: with an arrow key, or after an entry (Tab,
    /// Enter and their Shift variants) when `entry` is set. A protected
    /// sheet walks its unlocked cells. `None`: the neighbouring cell.
    fn next_cell(&self, _app: &ExcelApp, _from: (u32, u32), _dir: Direction, _entry: bool) -> Option<(u32, u32)> {
        None
    }

    /// May this cell become the active cell or part of the selection? (A
    /// protected sheet can forbid selecting locked cells.)
    fn can_select(&self, _app: &ExcelApp, _row: u32, _col: u32) -> bool {
        true
    }

    /// Alt+Down on the active cell: open the drop-down the feature hangs
    /// from it (the list of a filter button) and answer true; false when
    /// it has none there.
    fn cell_drop_down(&self, _app: &mut ExcelApp, _window: &mut Window, _cx: &mut Cx) -> bool {
        false
    }

    // ---- menus and status bar ---------------------------------------------

    /// Add entries to a context menu about to open.
    fn context_menu(&self, _app: &ExcelApp, _target: &MenuTarget, _menu: &mut ContextMenu) {}

    /// Texts for the status bar.
    fn status_items(&self, _app: &ExcelApp) -> Vec<StatusItem> {
        Vec::new()
    }

    // ---- floating objects -------------------------------------------------

    /// The objects of a sheet, bottom-most first.
    fn objects(&self, _app: &ExcelApp, _sheet: usize) -> Vec<ObjectInfo> {
        Vec::new()
    }

    /// Draw one object inside `rect` (x0, y0, x1, y1). The selection frame
    /// and the handles are drawn by the caller.
    fn paint_object(&self, _p: &mut Painter, _object: &ObjectInfo, _rect: (f32, f32, f32, f32)) {}

    /// Move / resize an object of the current sheet. Called repeatedly
    /// while dragging; the caller commits when the drag ends.
    fn place_object(&self, _app: &mut ExcelApp, _id: u32, _place: Placement) {}

    /// Label of the undo step for something done to an object.
    fn object_label(&self, _id: u32, _verb: ObjectVerb) -> &'static str {
        ""
    }

    /// Remove an object of the current sheet (the caller commits).
    fn delete_object(&self, _app: &mut ExcelApp, _id: u32) {}

    /// What Copy puts aside for an object.
    fn copy_object(&self, _app: &ExcelApp, _id: u32) -> Option<Rc<dyn Any>> {
        None
    }

    /// Paste what [`Feature::copy_object`] returned on the current sheet
    /// with its corner at `at`; returns the id of the new object (the
    /// caller commits and selects it).
    fn paste_object(&self, _app: &mut ExcelApp, _data: &dyn Any, _at: ((u32, u32), (f32, f32))) -> Option<u32> {
        None
    }

    /// Change the stacking order (the caller commits).
    fn reorder_object(&self, _app: &mut ExcelApp, _id: u32, _mv: ZMove) {}

    fn object_double_click(&self, _app: &mut ExcelApp, _id: u32, _ev: &ZoneEvent, _window: &mut Window, _cx: &mut Cx) {}

    /// One of this feature's objects was selected (`Some`) or the selected
    /// one was deselected (`None`).
    fn object_selected(&self, _app: &mut ExcelApp, _id: Option<u32>, _cx: &mut Cx) {}

    /// Whether a point of the grid element is on an object that does not
    /// fill its frame (a line, a rotated shape, a shape without fill).
    /// `None`: anywhere in the frame.
    fn object_hit(&self, _app: &ExcelApp, _id: u32, _layout: &GridLayout, _x: f32, _y: f32) -> Option<bool> {
        None
    }

    /// The box (x0, y0, x1, y1) an object covers on screen when it is not
    /// its frame (a rotated shape, the tail of a callout): it is drawn as
    /// long as that box shows.
    fn object_extent(&self, _app: &ExcelApp, _id: u32, _layout: &GridLayout) -> Option<(f32, f32, f32, f32)> {
        None
    }

    /// True when the feature draws the selection of this object and
    /// handles its handles itself (zones registered in `paint_object`),
    /// instead of the frame and the eight handles of `objects.rs`: the two
    /// ends of a line, the handles of a rotated shape.
    fn object_own_handles(&self, _app: &ExcelApp, _id: u32) -> bool {
        false
    }

    /// Text typed while one of this feature's objects is selected (a
    /// shape takes it as its text). True when the feature took it.
    fn object_typed(&self, _app: &mut ExcelApp, _id: u32, _text: &str, _window: &mut Window, _cx: &mut Cx) -> bool {
        false
    }

    /// A formatting command of the Home tab (font, colours, alignment)
    /// was run while one of this feature's objects is selected: `change`
    /// is what it does to a cell style. True when the feature applied it
    /// to the object (the text of a shape) and committed.
    fn object_style(&self, _app: &mut ExcelApp, _id: u32, _label: &str, _change: &dyn Fn(&mut Style), _cx: &mut Cx) -> bool {
        false
    }

    /// What Copy leaves on the system clipboard for an object, for other
    /// applications (a picture). `None` leaves it empty.
    fn object_clipboard(&self, _app: &ExcelApp, _id: u32) -> Option<ClipboardItem> {
        None
    }

    /// Paste was asked: true when the feature pasted what the system
    /// clipboard holds itself (a picture copied in another application).
    fn paste_clipboard(&self, _app: &mut ExcelApp, _cx: &mut Cx) -> bool {
        false
    }

    /// Escape was pressed while no cell is being edited: true when the
    /// feature had something of its own to cancel (a tool waiting for a
    /// click on the sheet).
    fn cancel(&self, _app: &mut ExcelApp, _cx: &mut Cx) -> bool {
        false
    }

    // ---- views --------------------------------------------------------------

    /// The view mode of the current sheet changed (`app.view_mode()`).
    fn view_mode_changed(&self, _app: &mut ExcelApp, _cx: &mut Cx) {}
}

// ----------------------------------------------- dispatch (central code only)

pub(crate) fn context_tabs(app: &ExcelApp) -> Vec<(usize, ContextTab)> {
    let mut out = Vec::new();
    for (i, f) in all().iter().enumerate() {
        out.extend(f.context_tabs(app).into_iter().map(|tab| (i, tab)));
    }
    out
}

/// Run the paint hook of every feature for one pane and layer.
pub(crate) fn paint(p: &mut Painter) {
    for (i, f) in all().iter().enumerate() {
        p.owner = i;
        f.paint(p);
    }
}

pub(crate) fn adjust_layout(app: &ExcelApp, layout: &mut GridLayout) {
    for f in all() {
        f.adjust_layout(app, layout);
    }
    layout.clamp_scroll();
}

/// The first refusal, if any feature objects to the change.
pub(crate) fn guard(app: &ExcelApp, intent: &Intent) -> Option<SharedString> {
    all().iter().find_map(|f| f.guard(app, intent))
}

pub(crate) fn may_commit(app: &mut ExcelApp, edit: &PendingEdit, window: &mut Window, cx: &mut Cx) -> Verdict {
    for f in all() {
        if f.may_commit(app, edit, window, cx) == Verdict::Hold {
            return Verdict::Hold;
        }
    }
    Verdict::Accept
}

pub(crate) fn cells_written(app: &mut ExcelApp, edit: &CellEdit, cx: &mut Cx) {
    for f in all() {
        f.cells_written(app, edit, cx);
    }
}

pub(crate) fn cells_changed(app: &mut ExcelApp, edit: &CellEdit, cx: &mut Cx) {
    for f in all() {
        f.cells_changed(app, edit, cx);
    }
}

pub(crate) fn can_select(app: &ExcelApp, row: u32, col: u32) -> bool {
    all().iter().all(|f| f.can_select(app, row, col))
}

/// The first feature that has a drop-down on the active cell opens it.
pub(crate) fn cell_drop_down(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> bool {
    all().iter().any(|f| f.cell_drop_down(app, window, cx))
}

pub(crate) fn selection_changed(app: &mut ExcelApp, cx: &mut Cx) {
    for f in all() {
        f.selection_changed(app, cx);
    }
}

pub(crate) fn next_cell(app: &ExcelApp, from: (u32, u32), dir: Direction, entry: bool) -> Option<(u32, u32)> {
    all().iter().find_map(|f| f.next_cell(app, from, dir, entry))
}

pub(crate) fn context_menu(app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
    for f in all() {
        f.context_menu(app, target, menu);
    }
}

pub(crate) fn status_items(app: &ExcelApp) -> Vec<StatusItem> {
    all().iter().flat_map(|f| f.status_items(app)).collect()
}

pub(crate) fn overlay_elements(app: &mut ExcelApp, layout: &GridLayout, window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
    let mut out = Vec::new();
    for f in all() {
        out.extend(f.overlay_elements(app, layout, window, cx));
    }
    out
}

pub(crate) fn view_mode_changed(app: &mut ExcelApp, cx: &mut Cx) {
    for f in all() {
        f.view_mode_changed(app, cx);
    }
}

/// True when a feature pasted what the system clipboard holds itself.
pub(crate) fn paste_clipboard(app: &mut ExcelApp, cx: &mut Cx) -> bool {
    all().iter().any(|f| f.paste_clipboard(app, cx))
}

/// True when a feature had something to cancel on Escape.
pub(crate) fn cancel(app: &mut ExcelApp, cx: &mut Cx) -> bool {
    all().iter().any(|f| f.cancel(app, cx))
}

/// Every floating object of a sheet, bottom-most first.
pub(crate) fn objects(app: &ExcelApp, sheet: usize) -> Vec<(ObjectId, ObjectInfo)> {
    let mut out = Vec::new();
    for (i, f) in all().iter().enumerate() {
        out.extend(f.objects(app, sheet).into_iter().map(|o| (ObjectId { owner: i, id: o.id }, o)));
    }
    out
}
