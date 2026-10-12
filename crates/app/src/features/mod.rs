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
//! * **Commands**: a `Cmd` enum implementing [`Command`]. The ribbon, the
//!   menus and the keyboard shortcuts already run them
//!   (`Cmd::Toggle.run(app, window, cx)`); a command that is not written yet
//!   ends in `app.not_implemented(self.label(), cx)`.
//!
//! State a feature keeps between events lives in `app.ext_mut::<MyState>()`
//! (any `Default` type, created on first use, dropped when another workbook
//! is opened). The workbook itself is `app.wb`; change it the usual way
//! (`app.wb.snapshot()`, mutate, `app.commit(label, cx)`).
//!
//! # What the hooks cover
//!
//! | Need | Hook / API |
//! |---|---|
//! | Ribbon button, menu entry, shortcut | [`Command`]; menus are `Vec<MenuEntry>` (`MenuEntry::command`, `MenuEntry::submenu`) opened with `app.open_command_menu` |
//! | Contextual ribbon tab ("Table Design") | [`Feature::context_tabs`] + [`Feature::render_tab`], built with the `pub` helpers of `ribbon.rs` (`group`, `rows`, `row`, `app.cmd_large`, `app.cmd_labeled`, `app.cmd_check`, …) |
//! | Modal dialog | `app.open_modal(window, cx, \|modal, window, cx\| MyDialog::new(modal, …))`; the view is drawn with `widgets::dialog(&self.modal, title)`; alerts with `app.alert(Alert::new(..), cx)` |
//! | Side pane right of the grid | `app.open_pane(id, title, view, cx)`, `app.close_pane(id, cx)` |
//! | Drop-down hanging from a cell | `app.open_popup_view(app.cell_anchor(rect), view, cx)` |
//! | Cell appearance | the grid paints `Workbook::display`; [`condfmt::paint_adornment`] draws data bars and icons |
//! | Drawing under / over the cells | [`Feature::paint`] with a [`Painter`] (once per pane and [`PaintLayer`]) |
//! | Clickable things drawn on the grid | [`Painter::zone`], then [`Feature::zone_mouse_down`], [`Feature::zone_drag`], [`Feature::zone_mouse_up`], [`Feature::zone_hover`]; cursor and tooltip are fields of the [`Zone`] |
//! | Real elements over the grid (a note being typed) | [`Feature::overlay_elements`] |
//! | Refusing a change (protection, pivot cells) | [`Feature::guard`] with an [`Intent`] |
//! | Checking an entry (validation) | [`Feature::may_commit`] → [`Verdict`] |
//! | Reacting to edits and to the selection | [`Feature::cells_changed`], [`Feature::selection_changed`] |
//! | Context menus | [`Feature::context_menu`] with a [`MenuTarget`] |
//! | Status bar | [`Feature::status_items`] |
//! | Charts-like objects (pictures, shapes) | [`Feature::objects`] and the `*_object` hooks; selection, moving, resizing, nudging, deleting, copy / paste and z-order come from `objects.rs` |
//! | Page Layout view / Page Break Preview | `app.view_mode()`, [`Feature::adjust_layout`] (`Axis::set_gaps`), [`Feature::view_mode_changed`] |
//!
//! # A worked example
//!
//! A feature that marks cells holding a note with a red corner, shows the
//! note in a tooltip and edits it in a dialog on double-click:
//!
//! ```ignore
//! pub struct Notes;
//!
//! #[derive(Clone, Copy)]
//! pub enum Cmd { Edit }
//!
//! impl Command for Cmd {
//!     fn label(self) -> &'static str { t("Edit Note", "Modifier la note") }
//!     fn icon(self) -> &'static str { "comment" }
//!     fn enabled(self, app: &ExcelApp) -> bool {
//!         let (r, c) = app.active();
//!         app.wb.sheets[app.sheet].comments.contains_key(&(r, c))
//!     }
//!     fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
//!         let (row, col) = app.active();
//!         if !app.may(Intent::EditObjects, cx) { return; }
//!         let text = app.wb.sheets[app.sheet].comments.get(&(row, col)).map(|c| c.text.clone());
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
//!         for ((row, col), note) in sheet.comments.range((p.visible.r0, 0)..=(p.visible.r1, u32::MAX)) {
//!             if !p.visible.contains(*row, *col) { continue; }
//!             let (_, y0, x1, _) = p.cell_box(Rect::cell(*row, *col));
//!             p.polygon(&[(x1 - 7.0, y0), (x1 - 1.0, y0), (x1 - 1.0, y0 + 6.0)], c(0xC00000));
//!             p.zone((x1 - 8.0, y0, x1, y0 + 8.0), Corner(*row, *col)).tooltip(note.text.clone());
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
//! }
//!
//! struct NoteDialog { modal: Modal, row: u32, col: u32, text: Entity<TextInput> }
//!
//! impl NoteDialog {
//!     fn new(modal: Modal, row: u32, col: u32, text: Option<String>, cx: &mut Context<Self>) -> Self {
//!         let text = widgets::input(cx, text.unwrap_or_default());
//!         NoteDialog { modal, row, col, text }
//!     }
//!     fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
//!         let (row, col, text) = (self.row, self.col, self.text.read(cx).text().to_string());
//!         self.modal.update_app(cx, |app, cx| {
//!             app.wb.snapshot();
//!             let sheet = app.sheet;
//!             app.wb.sheets[sheet].comments.entry((row, col)).or_default().text = text;
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
//!             .child(widgets::TextField::new(&self.text))
//!             .footer(widgets::ok_cancel(&self.modal, cx.listener(Self::ok)))
//!     }
//! }
//! ```
//!
//! `format_cells.rs` (a full dialog) and `charts.rs` (floating objects, a
//! contextual tab, a context menu) are the two modules already written
//! this way.

use crate::app::{ExcelApp, MenuEntry};
use crate::grid::{Draw, GridLayout};
use crate::theme::c;
use gpui::{
    AnyElement, BorderStyle, Bounds, Context, CursorStyle, Font, FontStyle, FontWeight, Hsla, Modifiers, MouseButton,
    PathBuilder, Pixels, Point, ShapedLine, SharedString, TextRun, Window, WindowTextSystem, fill, px, quad, size,
};
use opensheet_engine::Rect;
use opensheet_engine::ops::Direction;
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
pub mod drawings;
pub mod filters;
pub mod format_cells;
pub mod misc;
pub mod names;
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
    static ALL: [&dyn Feature; 14] = [
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
        &misc::Misc,
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

/// A change about to be made, submitted to [`Feature::guard`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Intent {
    /// Typing into, pasting over, filling or clearing these cells of the
    /// current sheet.
    EditCells(Rect),
    /// Changing the formatting of these cells.
    FormatCells(Rect),
    /// Changing the width or the visibility of columns / rows.
    FormatColumns,
    FormatRows,
    InsertColumns,
    InsertRows,
    DeleteColumns,
    DeleteRows,
    Sort,
    /// Adding, moving, resizing or deleting charts, pictures, shapes and
    /// comments.
    EditObjects,
    /// Adding, deleting, renaming, moving, hiding or colouring sheets.
    EditStructure,
}

/// An entry waiting to be stored, submitted to [`Feature::may_commit`].
#[derive(Clone, Debug)]
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

/// Why cells changed, for [`Feature::cells_changed`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeCause {
    Typing,
    Paste,
    Fill,
    Clear,
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
    /// Device pixels per logical pixel.
    pub scale: f32,
    pub(crate) owner: usize,
    pub(crate) items: Vec<Draw>,
    pub(crate) zones: Vec<Zone>,
}

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

    /// Cells of `sheet` were changed by the user and the change was
    /// committed (its own undo step). What the feature does in response is
    /// a step of its own (`snapshot`, mutate, `app.commit`).
    fn cells_changed(&self, _app: &mut ExcelApp, _sheet: usize, _rect: Rect, _cause: ChangeCause, _cx: &mut Cx) {}

    /// The selection or the current sheet changed.
    fn selection_changed(&self, _app: &mut ExcelApp, _cx: &mut Cx) {}

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

pub(crate) fn cells_changed(app: &mut ExcelApp, sheet: usize, rect: Rect, cause: ChangeCause, cx: &mut Cx) {
    for f in all() {
        f.cells_changed(app, sheet, rect, cause, cx);
    }
}

pub(crate) fn selection_changed(app: &mut ExcelApp, cx: &mut Cx) {
    for f in all() {
        f.selection_changed(app, cx);
    }
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

/// Every floating object of a sheet, bottom-most first.
pub(crate) fn objects(app: &ExcelApp, sheet: usize) -> Vec<(ObjectId, ObjectInfo)> {
    let mut out = Vec::new();
    for (i, f) in all().iter().enumerate() {
        out.extend(f.objects(app, sheet).into_iter().map(|o| (ObjectId { owner: i, id: o.id }, o)));
    }
    out
}
