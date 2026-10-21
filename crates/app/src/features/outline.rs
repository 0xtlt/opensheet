//! Outlines: Data > Group, Ungroup, Show Detail, Hide Detail and Subtotal,
//! and the margin of the grid where the outline shows (level buttons,
//! bars, the − / + buttons and the dots of deeper rows).
//!
//! The margin is room set aside in the grid's layout (`GridLayout::margin_w`
//! and `margin_h`, see `adjust_layout`); what goes into it is worked out by
//! `symbols` from the sheet and the layout alone, so that it can be tested
//! without a window, then painted on the top layer. The dialogs are in
//! `outline/dialogs.rs` and `outline/subtotal.rs`.

mod dialogs;
mod subtotal;
#[cfg(test)]
mod ui_tests;

use super::{
    Command, Cx, Feature, Intent, PaintLayer, Painter, ViewMode, Zone, ZoneEvent, ZoneResponse,
};
use crate::app::{ExcelApp, MenuEntry};
use crate::grid::{Draw, GridLayout};
use crate::i18n::{t, t_ctx};
use crate::ribbon::{launcher_group, rows};
use crate::shell::Alert;
use crate::theme::{self, c};
use gpui::{
    AnyElement, App, Context, Entity, KeyBinding, MouseButton, SharedString, Stateful, Window,
    actions, div, prelude::*, px,
};
use opensheet_engine::Rect;
use opensheet_engine::outline::{Axis, Group, OutlineError};
use opensheet_engine::protect::Action;
use opensheet_engine::workbook::Sheet;
use std::ops::Range;

pub struct Outline;

commands! {
    /// What the user can ask of outlines.
    pub enum Cmd {
        GroupMenu = "outline-group", t_ctx("outline", "Group", "Grouper");
        Group = "outline-group", t("Group…", "Grouper…");
        AutoOutline = "", t("Auto Outline", "Plan automatique");
        UngroupMenu = "outline-ungroup", t_ctx("outline", "Ungroup", "Dissocier");
        Ungroup = "outline-ungroup", t("Ungroup…", "Dissocier…");
        ClearOutline = "", t("Clear Outline", "Effacer le plan");
        ShowDetail = "outline-show", t("Show Detail", "Afficher les détails");
        HideDetail = "outline-hide", t("Hide Detail", "Masquer les détails");
        Settings = "", t_ctx("outline", "Settings", "Paramètres");
        ToggleSymbols = "", t("Outline symbols", "Symboles du plan");
        Subtotal = "outline-subtotal", t("Subtotal", "Sous-total");
    }
}

impl Command for Cmd {
    fn label(self) -> &'static str {
        self.face().1
    }

    fn icon(self) -> &'static str {
        self.face().0
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        // Menus only open: their entries come back here.
        let menu = matches!(self, Cmd::GroupMenu | Cmd::UngroupMenu);
        if !menu && app.is_editing() && !app.commit_edit(None, window, cx) {
            return;
        }
        match self {
            Cmd::GroupMenu => {
                let entries = vec![
                    MenuEntry::command(Cmd::Group, app),
                    MenuEntry::command(Cmd::AutoOutline, app),
                ];
                app.open_command_menu(entries, 200.0, window, cx);
            }
            Cmd::UngroupMenu => {
                let entries = vec![
                    MenuEntry::command(Cmd::Ungroup, app),
                    MenuEntry::command(Cmd::ClearOutline, app),
                ];
                app.open_command_menu(entries, 200.0, window, cx);
            }
            Cmd::Group => {
                group_selection(app, false, window, cx);
            }
            Cmd::Ungroup => {
                group_selection(app, true, window, cx);
            }
            Cmd::AutoOutline => auto_outline(app, cx),
            Cmd::ClearOutline => {
                let sheet = app.sheet;
                if app.may(Intent::ChangeSheet, cx)
                    && app.wb.clear_outline(sheet, selected_area(app)).is_ok()
                {
                    app.commit(Cmd::ClearOutline.label(), cx);
                }
            }
            Cmd::ShowDetail => detail(app, true, cx),
            Cmd::HideDetail => detail(app, false, cx),
            Cmd::Settings => {
                dialogs::open_settings(app, window, cx);
            }
            Cmd::ToggleSymbols => toggle_symbols(app, cx),
            Cmd::Subtotal => {
                subtotal::open(app, window, cx);
            }
        }
    }

    fn enabled(self, app: &ExcelApp) -> bool {
        // Excel greys the outline commands out on a protected sheet, and
        // Subtotal inside a table as well.
        let free = app
            .wb
            .permits(&Action::ChangeSheet { sheet: app.sheet })
            .is_ok();
        match self {
            Cmd::Subtotal => {
                let (row, col) = app.active();
                free && app.wb.table_at(app.sheet, row, col).is_none()
            }
            _ => free,
        }
    }

    fn checked(self, app: &ExcelApp) -> bool {
        self == Cmd::ToggleSymbols && app.wb.sheets[app.sheet].outline.show_symbols
    }
}

/// Tell the user why what was asked cannot be done.
fn warn(app: &mut ExcelApp, message: impl Into<SharedString>, cx: &mut Cx) {
    app.alert(Alert::new("OpenSheet", message).warning(), cx);
}

/// What Auto Outline and Clear Outline work on: the whole sheet with a
/// single cell selected, else the selection.
fn selected_area(app: &ExcelApp) -> Option<Rect> {
    let sel = app.sel();
    (!sel.is_single_cell()).then(|| sel.current())
}

/// The rows or the columns the selection stands for when it is made of
/// whole rows or of whole columns; otherwise Excel asks.
fn selected_axis(rect: Rect) -> Option<Axis> {
    match (rect.is_whole_rows(), rect.is_whole_cols()) {
        (true, false) => Some(Axis::Rows),
        (false, true) => Some(Axis::Cols),
        _ => None,
    }
}

fn span(rect: Rect, axis: Axis) -> (u32, u32) {
    match axis {
        Axis::Rows => (rect.r0, rect.r1),
        Axis::Cols => (rect.c0, rect.c1),
    }
}

/// Group or Ungroup: at once on whole rows or columns, after the Rows /
/// Columns question (the dialog returned) on cells.
fn group_selection(
    app: &mut ExcelApp,
    ungroup: bool,
    window: &mut Window,
    cx: &mut Cx,
) -> Option<Entity<dialogs::GroupDialog>> {
    match selected_axis(app.sel().current()) {
        Some(axis) => {
            apply_group(app, axis, ungroup, cx);
            None
        }
        None => Some(dialogs::open_group(app, ungroup, window, cx)),
    }
}

/// Group or ungroup the rows (columns) of the selection.
pub(crate) fn apply_group(app: &mut ExcelApp, axis: Axis, ungroup: bool, cx: &mut Cx) {
    if !app.may(Intent::ChangeSheet, cx) {
        return;
    }
    let sheet = app.sheet;
    let (lo, hi) = span(app.sel().current(), axis);
    let done = if ungroup {
        app.wb.outline_ungroup(sheet, axis, lo, hi)
    } else {
        app.wb.outline_group(sheet, axis, lo, hi)
    };
    match done {
        Ok(()) => {
            let label = if ungroup {
                Cmd::UngroupMenu.label()
            } else {
                Cmd::GroupMenu.label()
            };
            app.commit(label, cx);
        }
        Err(OutlineError::TooDeep) => warn(
            app,
            t(
                "Cannot group that selection.",
                "Impossible de grouper cette sélection.",
            ),
            cx,
        ),
        Err(_) if app.wb.sheets[sheet].outline.is_empty() => warn(
            app,
            t(
                "Cannot ungroup.  To start an outline, select the detail rows or columns and group them.",
                "Impossible de dissocier. Pour construire un plan, sélectionnez les lignes ou les colonnes de détail et groupez-les.",
            ),
            cx,
        ),
        Err(_) => warn(
            app,
            t(
                "Cannot ungroup that selection.",
                "Impossible de dissocier le groupe de cette sélection.",
            ),
            cx,
        ),
    }
}

pub(crate) fn auto_outline(app: &mut ExcelApp, cx: &mut Cx) {
    if !app.may(Intent::ChangeSheet, cx) {
        return;
    }
    let sheet = app.sheet;
    match app.wb.auto_outline(sheet, selected_area(app)) {
        Ok(()) => app.commit(Cmd::AutoOutline.label(), cx),
        Err(_) => warn(
            app,
            t("Cannot create an outline.", "Impossible de créer un plan."),
            cx,
        ),
    }
}

fn detail(app: &mut ExcelApp, show: bool, cx: &mut Cx) {
    let sheet = app.sheet;
    let selection = app.sel().current();
    // Folding hides rows or columns: a protected sheet must allow that.
    if !app.may(Intent::FormatRows, cx) || !app.may(Intent::FormatColumns, cx) {
        return;
    }
    match app.wb.outline_detail(sheet, selection, show) {
        Ok(()) => app.commit(
            if show {
                Cmd::ShowDetail.label()
            } else {
                Cmd::HideDetail.label()
            },
            cx,
        ),
        Err(_) => warn(
            app,
            t(
                "Cannot show or hide detail for this selection.",
                "Impossible d'afficher ou de masquer le détail pour cette sélection.",
            ),
            cx,
        ),
    }
}

/// Ctrl+8: show or hide the outline symbols. Without an outline Excel
/// offers to make one.
fn toggle_symbols(app: &mut ExcelApp, cx: &mut Cx) {
    let sheet = app.sheet;
    let outline = app.wb.sheets[sheet].outline.clone();
    if outline.is_empty() {
        let question = t(
            "Cannot show outline symbols because no outline exists on the active worksheet. Create an outline?",
            "Impossible d'afficher les symboles du plan car aucun plan n'est présent dans la feuille active. Voulez-vous créer un plan?",
        );
        let alert = Alert::new("OpenSheet", question)
            .warning()
            .button(t("OK", "OK"), |app, _, cx| auto_outline(app, cx))
            .close_button(t("Cancel", "Annuler"));
        app.alert(alert, cx);
        return;
    }
    app.wb.set_outline_settings(
        sheet,
        outline.summary_below,
        outline.summary_right,
        !outline.show_symbols,
    );
    app.commit(Cmd::ToggleSymbols.label(), cx);
}

/// Fold or unfold a group, as a click on its button does.
fn fold(app: &mut ExcelApp, axis: Axis, group: Group, collapse: bool, cx: &mut Cx) {
    let intent = if axis == Axis::Rows {
        Intent::FormatRows
    } else {
        Intent::FormatColumns
    };
    if !app.may(intent, cx) {
        return;
    }
    let sheet = app.sheet;
    app.wb.outline_set_collapsed(sheet, axis, &group, collapse);
    app.commit(
        if collapse {
            Cmd::HideDetail.label()
        } else {
            Cmd::ShowDetail.label()
        },
        cx,
    );
}

/// A click on a level button.
fn show_level(app: &mut ExcelApp, axis: Axis, level: u8, cx: &mut Cx) {
    let intent = if axis == Axis::Rows {
        Intent::FormatRows
    } else {
        Intent::FormatColumns
    };
    if !app.may(intent, cx) {
        return;
    }
    let sheet = app.sheet;
    app.wb.outline_show_level(sheet, axis, level);
    app.commit(Cmd::ShowDetail.label(), cx);
}

// ------------------------------------------------------------------- margin

/// Width of the room one level takes in the margin. The symbols follow
/// the zoom within limits, so that they stay readable and clickable.
fn unit(zoom: f32) -> f32 {
    (16.0 * zoom.clamp(0.75, 1.5)).round()
}

/// Room around the levels.
const PAD: f32 = 3.0;

/// The colour of the bars, dots and button frames.
const INK: u32 = 0x6E6E6E;

/// How much room the outline symbols of a sheet take left of the row
/// headings and above the column headings, given the size of the
/// headings themselves. The level buttons stand beside the headings;
/// where headings are hidden they get a strip of their own.
fn margins(sheet: &Sheet, zoom: f32, headings: (f32, f32)) -> (f32, f32) {
    let outline = &sheet.outline;
    if !outline.show_symbols {
        return (0.0, 0.0);
    }
    let u = unit(zoom);
    let room = |deepest: u8| {
        if deepest > 0 {
            (deepest as f32 + 1.0) * u + PAD
        } else {
            0.0
        }
    };
    let (mut w, mut h) = (
        room(outline.rows.max_level()),
        room(outline.cols.max_level()),
    );
    if w > 0.0 && headings.1 < u {
        h += u;
    }
    if h > 0.0 && headings.0 < u && outline.cols.max_level() > 0 {
        w += u;
    }
    (w, h)
}

pub(crate) fn adjust_layout(app: &ExcelApp, layout: &mut GridLayout) {
    // The Page Layout view has its rulers there.
    if app.view_mode() == ViewMode::PageLayout {
        return;
    }
    let (w, h) = margins(
        &app.wb.sheets[app.sheet],
        layout.zoom,
        (layout.header_w, layout.header_h),
    );
    layout.margin_w = w;
    layout.margin_h = h;
    layout.header_w += w;
    layout.header_h += h;
}

/// What a click in the margin lands on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Hit {
    /// The margin itself: nothing happens (and no cell is selected).
    Inert,
    /// A level button.
    Level(Axis, u8),
    /// The button or the bar of a group: fold it (`true`) or unfold it.
    Fold(Axis, Group, bool),
}

/// A shape of the margin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Shape {
    /// A line of a bar (x0, y0, x1, y1).
    Line,
    /// A dot marking a row of deeper detail.
    Dot,
    /// A − button (group shown) or a + button (group folded).
    Button { collapsed: bool },
    /// A level button with its number.
    Level(u8),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Symbol {
    pub shape: Shape,
    /// x0, y0, x1, y1 in grid-element coordinates.
    pub rect: (f32, f32, f32, f32),
    /// What cuts it off: the strip of the frozen or of the scrolled rows
    /// (columns).
    pub clip: (f32, f32, f32, f32),
    pub hit: Option<Hit>,
}

/// The geometry of one axis: where its rows (columns) are along the
/// screen, and where its levels are across the margin.
struct Strip<'a> {
    axis: Axis,
    layout: &'a GridLayout,
    sheet: &'a Sheet,
    unit: f32,
}

impl Strip<'_> {
    fn start(&self, index: u32) -> f32 {
        match self.axis {
            Axis::Rows => self.layout.row_y(index),
            Axis::Cols => self.layout.col_x(index),
        }
    }

    fn size(&self, index: u32) -> f32 {
        match self.axis {
            Axis::Rows => self.layout.row_h(index),
            Axis::Cols => self.layout.col_w(index),
        }
    }

    /// Middle of a level across the margin.
    fn across(&self, level: u8) -> f32 {
        (PAD / 2.0).floor() + (level as f32 - 0.5) * self.unit
    }

    /// The extent of the margin across.
    fn depth(&self) -> f32 {
        match self.axis {
            Axis::Rows => self.layout.margin_w,
            Axis::Cols => self.layout.margin_h,
        }
    }

    /// The frozen and the scrolled rows (columns) in view, each with the
    /// part of the screen it has.
    fn bands(&self) -> [(Range<u32>, f32, f32); 2] {
        let l = self.layout;
        match self.axis {
            Axis::Rows => {
                let edge = l.header_h + l.frozen_h;
                [
                    (0..l.frozen_rows, l.header_h, edge),
                    (l.scrolled_rows(), edge, l.height()),
                ]
            }
            Axis::Cols => {
                let edge = l.header_w + l.frozen_w;
                [
                    (0..l.frozen_cols, l.header_w, edge),
                    (l.scrolled_cols(), edge, l.width()),
                ]
            }
        }
    }

    /// A box given along the axis and across the margin.
    fn boxed(&self, along: (f32, f32), across: (f32, f32)) -> (f32, f32, f32, f32) {
        match self.axis {
            Axis::Rows => (across.0, along.0, across.1, along.1),
            Axis::Cols => (along.0, across.0, along.1, across.1),
        }
    }

    /// Half the side of a button.
    fn half(&self) -> f32 {
        ((self.unit - 5.0) / 2.0).floor()
    }

    fn symbols(&self, out: &mut Vec<Symbol>) {
        let lines = self.sheet.outline.axis(self.axis);
        let after = self.sheet.outline.summary_after(self.axis);
        let half = self.half();
        for (range, lo, hi) in self.bands() {
            if range.is_empty() || hi <= lo {
                continue;
            }
            let clip = self.boxed((lo, hi), (0.0, self.depth()));
            let mut push =
                |shape: Shape, along: (f32, f32), across: (f32, f32), hit: Option<Hit>| {
                    out.push(Symbol {
                        shape,
                        rect: self.boxed(along, across),
                        clip,
                        hit,
                    });
                };
            let (first_shown, last_shown) = (range.start, range.end - 1);
            for group in lines.groups(first_shown, last_shown) {
                let centre = self.across(group.level);
                let collapsed = self.sheet.outline_collapsed(self.axis, &group);
                // The button, on the summary when it is in this strip and shown.
                let summary = self
                    .sheet
                    .outline_summary(self.axis, &group)
                    .filter(|s| range.contains(s) && self.size(*s) > 0.0);
                let button = summary.map(|s| self.start(s) + (self.size(s) / 2.0).floor());
                if let Some(middle) = button {
                    push(
                        Shape::Button { collapsed },
                        (middle - half, middle + half + 1.0),
                        (centre - half, centre + half + 1.0),
                        Some(Hit::Fold(self.axis, group, !collapsed)),
                    );
                }
                if collapsed || group.last < first_shown || group.first > last_shown {
                    continue;
                }
                // The bar runs from the far end of the group, where it has
                // a tick, to the button.
                let begins = group.first >= first_shown;
                let ends = group.last <= last_shown;
                let top = if begins {
                    self.start(group.first) + 2.0
                } else {
                    lo - 2.0
                };
                let bottom = if ends {
                    self.start(group.last) + self.size(group.last) - 2.0
                } else {
                    hi + 2.0
                };
                let (from, to) = match (after, button) {
                    (true, Some(middle)) => (top, middle - half),
                    (false, Some(middle)) => (middle + half + 1.0, bottom),
                    _ => (top, bottom),
                };
                if to > from {
                    let hit = Some(Hit::Fold(self.axis, group, true));
                    push(Shape::Line, (from, to), (centre, centre + 1.0), hit);
                    let tick = (centre, centre + (self.unit / 3.0).round());
                    if after && begins {
                        push(Shape::Line, (from, from + 1.0), tick, hit);
                    } else if !after && ends {
                        push(Shape::Line, (to - 1.0, to), tick, hit);
                    }
                }
            }
            // A dot beside each row of detail, one level past its own.
            for (&index, &level) in lines.levels.range(first_shown..=last_shown) {
                if self.size(index) <= 0.0 || !lines.summarized_by(index, after).is_empty() {
                    continue;
                }
                let (middle, centre) = (
                    self.start(index) + (self.size(index) / 2.0).floor(),
                    self.across(level + 1),
                );
                push(
                    Shape::Dot,
                    (middle - 1.0, middle + 1.0),
                    (centre - 1.0, centre + 1.0),
                    None,
                );
            }
        }
    }

    /// The level buttons, in the corner beside the headings.
    fn level_buttons(&self, out: &mut Vec<Symbol>) {
        let l = self.layout;
        // Along the other axis: the band of the headings, or the strip
        // kept for the buttons when there are no headings.
        let (margin, header) = match self.axis {
            Axis::Rows => (l.margin_h, l.header_h),
            Axis::Cols => (l.margin_w, l.header_w),
        };
        let band = if header - margin >= self.unit {
            (margin, header)
        } else {
            (header - self.unit, header)
        };
        let middle = ((band.0 + band.1) / 2.0).floor();
        let half = self.half();
        let clip = match self.axis {
            Axis::Rows => (0.0, 0.0, l.margin_w, l.header_h),
            Axis::Cols => (0.0, 0.0, l.header_w, l.margin_h),
        };
        for level in 1..=self.sheet.outline.axis(self.axis).max_level() + 1 {
            let centre = self.across(level);
            // Across the margin as the bars are; beside the headings the other way.
            let rect = match self.axis {
                Axis::Rows => (
                    centre - half,
                    middle - half,
                    centre + half + 1.0,
                    middle + half + 1.0,
                ),
                Axis::Cols => (
                    middle - half,
                    centre - half,
                    middle + half + 1.0,
                    centre + half + 1.0,
                ),
            };
            out.push(Symbol {
                shape: Shape::Level(level),
                rect,
                clip,
                hit: Some(Hit::Level(self.axis, level)),
            });
        }
    }
}

/// Everything the margin shows for a sheet laid out as `layout`.
pub(crate) fn symbols(sheet: &Sheet, layout: &GridLayout) -> Vec<Symbol> {
    let mut out = Vec::new();
    let unit = unit(layout.zoom);
    for (axis, depth) in [(Axis::Rows, layout.margin_w), (Axis::Cols, layout.margin_h)] {
        if depth <= 0.0 || sheet.outline.axis(axis).is_empty() {
            continue;
        }
        let strip = Strip {
            axis,
            layout,
            sheet,
            unit,
        };
        strip.symbols(&mut out);
        strip.level_buttons(&mut out);
    }
    out
}

fn paint_margin(p: &mut Painter) {
    let layout = p.layout;
    let (mw, mh) = (layout.margin_w, layout.margin_h);
    if mw <= 0.0 && mh <= 0.0 {
        return;
    }
    let (w, h) = (layout.width(), layout.height());
    let (ground, edge, ink) = (c(theme::HEADER_BG), c(theme::HEADER_BORDER), c(INK));
    // The margin itself, which swallows the clicks that hit no symbol.
    if mw > 0.0 {
        p.rect(0.0, 0.0, mw, h, ground);
        p.rect(mw - 1.0, mh, mw, h, edge);
        p.zone((0.0, 0.0, mw, h), Hit::Inert);
    }
    if mh > 0.0 {
        p.rect(0.0, 0.0, w, mh, ground);
        p.rect(mw, mh - 1.0, w, mh, edge);
        p.zone((0.0, 0.0, w, mh), Hit::Inert);
    }
    let size = (unit(layout.zoom) * 0.6).round();
    for symbol in symbols(&p.app.wb.sheets[p.app.sheet], layout) {
        let (x0, y0, x1, y1) = symbol.rect;
        let clip = symbol.clip;
        // Each symbol is cut off at its strip: bars run under the frozen
        // rows as the cells do.
        let outer = std::mem::take(&mut p.items);
        match symbol.shape {
            Shape::Line | Shape::Dot => p.rect(x0, y0, x1, y1, ink),
            Shape::Button { collapsed } => {
                p.rect(x0, y0, x1, y1, c(0xFFFFFF));
                p.frame(x0, y0, x1, y1, 1.0, ink);
                let (cx, cy) = (((x0 + x1) / 2.0).floor(), ((y0 + y1) / 2.0).floor());
                let arm = ((x1 - x0) / 2.0 - 3.0).max(1.0).floor();
                p.rect(cx - arm, cy, cx + arm + 1.0, cy + 1.0, c(0x000000));
                if collapsed {
                    p.rect(cx, cy - arm, cx + 1.0, cy + arm + 1.0, c(0x000000));
                }
            }
            Shape::Level(level) => {
                p.rect(x0, y0, x1, y1, c(0xFFFFFF));
                p.frame(x0, y0, x1, y1, 1.0, ink);
                let line = p.shape(&level.to_string(), size, c(0x000000), false);
                let width = f32::from(line.width);
                let top = y0 + ((y1 - y0 - size * 1.25) / 2.0).round();
                p.put(line, x0 + ((x1 - x0 - width) / 2.0).round(), top, size);
            }
        }
        let items = std::mem::replace(&mut p.items, outer);
        p.push(Draw::Clipped {
            clip: layout.bounds_px(clip.0, clip.1, clip.2, clip.3),
            items,
        });
        if let Some(hit) = symbol.hit {
            // A little larger than the drawing, and no further than the strip.
            let bounds = (
                (x0 - 2.0).max(clip.0),
                (y0 - 2.0).max(clip.1),
                (x1 + 2.0).min(clip.2),
                (y1 + 2.0).min(clip.3),
            );
            if bounds.2 > bounds.0 && bounds.3 > bounds.1 {
                p.zone(bounds, hit);
            }
        }
    }
}

impl Feature for Outline {
    fn name(&self) -> &'static str {
        "outline"
    }

    fn adjust_layout(&self, app: &ExcelApp, layout: &mut GridLayout) {
        adjust_layout(app, layout);
    }

    fn paint(&self, p: &mut Painter) {
        if p.layer == PaintLayer::Top {
            paint_margin(p);
        }
    }

    fn zone_mouse_down(
        &self,
        app: &mut ExcelApp,
        zone: &Zone,
        ev: &ZoneEvent,
        _window: &mut Window,
        cx: &mut Cx,
    ) -> ZoneResponse {
        let Some(hit) = zone.payload::<Hit>() else {
            return ZoneResponse::Ignored;
        };
        if ev.button == MouseButton::Left {
            match *hit {
                Hit::Inert => {}
                Hit::Level(axis, level) => show_level(app, axis, level, cx),
                Hit::Fold(axis, group, collapse) => fold(app, axis, group, collapse, cx),
            }
        }
        ZoneResponse::Handled
    }
}

// ------------------------------------------------------------------- ribbon

/// The Outline group of the Data tab (the last one: the caller takes its
/// right border off).
pub fn ribbon_group(app: &ExcelApp, cx: &mut Cx) -> gpui::Div {
    let compact = crate::ribbon_fold::level() >= 3;
    let small = |id: &'static str, cmd: Cmd, cx: &mut Cx| -> AnyElement {
        if compact {
            app.cmd_icon(id, cmd, cx)
        } else {
            app.cmd_labeled(id, cmd, false, cx)
        }
    };
    launcher_group(
        t_ctx("group", "Outline", "Plan"),
        div()
            .flex()
            .items_center()
            .gap(px(2.))
            .child(app.cmd_large("dt-group", Cmd::GroupMenu, true, cx))
            .child(app.cmd_large("dt-ungroup", Cmd::UngroupMenu, true, cx))
            .child(app.cmd_large("dt-subtotal", Cmd::Subtotal, false, cx))
            .child(rows(vec![
                small("dt-show-detail", Cmd::ShowDetail, cx),
                small("dt-hide-detail", Cmd::HideDetail, cx),
            ])),
        Some(app.launcher(
            "dt-outline-settings",
            Cmd::Settings.label(),
            cx,
            |app, window, cx| app.run_command(Cmd::Settings, window, cx),
        )),
    )
}

// ---------------------------------------------------------------- shortcuts

actions!(
    outline,
    [GroupSelection, UngroupSelection, ToggleOutlineSymbols]
);

/// Excel's shortcuts: ⌥⇧→ groups, ⌥⇧← ungroups (⌘⇧K and ⌘⇧J on the Mac
/// as well), ⌃8 shows or hides the outline symbols.
pub fn bind_keys(cx: &mut App) {
    let w = Some("Workbook");
    cx.bind_keys([
        KeyBinding::new("alt-shift-right", GroupSelection, w),
        KeyBinding::new("cmd-shift-k", GroupSelection, w),
        KeyBinding::new("alt-shift-left", UngroupSelection, w),
        KeyBinding::new("cmd-shift-j", UngroupSelection, w),
        KeyBinding::new("ctrl-8", ToggleOutlineSymbols, w),
    ]);
}

/// Make the window answer the shortcuts of `bind_keys`.
pub fn key_actions(root: Stateful<gpui::Div>, cx: &Context<ExcelApp>) -> Stateful<gpui::Div> {
    root.on_action(
        cx.listener(|app, _: &GroupSelection, window, cx| app.run_command(Cmd::Group, window, cx)),
    )
    .on_action(cx.listener(|app, _: &UngroupSelection, window, cx| {
        app.run_command(Cmd::Ungroup, window, cx)
    }))
    .on_action(cx.listener(|app, _: &ToggleOutlineSymbols, window, cx| {
        app.run_command(Cmd::ToggleSymbols, window, cx)
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Bounds, point, size};
    use opensheet_engine::{Locale, Workbook};

    /// Rows 2-4 and 6-8 grouped under rows 5 and 9, all of them under row 10.
    fn nested() -> Workbook {
        let mut wb = Workbook::new(Locale::en());
        wb.outline_group(0, Axis::Rows, 1, 8).unwrap();
        wb.outline_group(0, Axis::Rows, 1, 3).unwrap();
        wb.outline_group(0, Axis::Rows, 5, 7).unwrap();
        wb.commit("setup");
        wb
    }

    fn layout(wb: &Workbook, zoom: f32, scroll_y: f64) -> GridLayout {
        let bounds = Bounds::new(point(px(0.), px(0.)), size(px(800.), px(400.)));
        let sheet = &wb.sheets[0];
        let mut layout = GridLayout::new(sheet, bounds, zoom, 0.0, scroll_y);
        let (w, h) = margins(sheet, zoom, (layout.header_w, layout.header_h));
        layout.margin_w = w;
        layout.margin_h = h;
        layout.header_w += w;
        layout.header_h += h;
        layout
    }

    fn of(symbols: &[Symbol], wanted: fn(&Shape) -> bool) -> Vec<Symbol> {
        symbols
            .iter()
            .copied()
            .filter(|s| wanted(&s.shape))
            .collect()
    }

    #[test]
    fn the_margin_widens_with_the_levels() {
        let mut wb = nested();
        let sheet = &wb.sheets[0];
        assert_eq!(margins(sheet, 1.0, (30.0, 20.0)), (3.0 * 16.0 + PAD, 0.0));
        // Without headings the level buttons need a strip.
        assert_eq!(margins(sheet, 1.0, (0.0, 0.0)), (3.0 * 16.0 + PAD, 16.0));
        assert_eq!(margins(sheet, 2.0, (60.0, 40.0)).0, 3.0 * 24.0 + PAD);
        wb.outline_group(0, Axis::Cols, 2, 3).unwrap();
        assert_eq!(
            margins(&wb.sheets[0], 1.0, (30.0, 20.0)),
            (3.0 * 16.0 + PAD, 2.0 * 16.0 + PAD)
        );
        let o = wb.sheets[0].outline.clone();
        wb.set_outline_settings(0, o.summary_below, o.summary_right, false);
        assert_eq!(margins(&wb.sheets[0], 1.0, (30.0, 20.0)), (0.0, 0.0));
    }

    #[test]
    fn buttons_bars_and_dots_of_nested_groups() {
        let wb = nested();
        let l = layout(&wb, 1.0, 0.0);
        let all = symbols(&wb.sheets[0], &l);
        // Three level buttons beside the column headings.
        let levels = of(&all, |s| matches!(s, Shape::Level(_)));
        assert_eq!(levels.len(), 3);
        assert_eq!(levels[1].hit, Some(Hit::Level(Axis::Rows, 2)));
        assert!(
            levels.iter().all(|s| s.rect.1 >= l.margin_h
                && s.rect.3 <= l.header_h
                && s.rect.2 <= l.margin_w)
        );
        assert!(levels[0].rect.0 < levels[1].rect.0 && levels[1].rect.0 < levels[2].rect.0);
        // A − button on each summary row, in the column of its level.
        let buttons = of(&all, |s| matches!(s, Shape::Button { .. }));
        assert_eq!(buttons.len(), 3);
        let outer = Group {
            level: 1,
            first: 1,
            last: 8,
        };
        let on_row = |s: &Symbol, row: u32| {
            s.rect.1 >= l.row_y(row) && s.rect.3 <= l.row_y(row) + l.row_h(row)
        };
        let grand = buttons
            .iter()
            .find(|s| s.hit == Some(Hit::Fold(Axis::Rows, outer, true)))
            .unwrap();
        assert!(on_row(grand, 9));
        assert_eq!(grand.shape, Shape::Button { collapsed: false });
        let inner = buttons.iter().find(|s| on_row(s, 4)).unwrap();
        assert!(
            inner.rect.0 > grand.rect.2,
            "deeper levels lie to the right"
        );
        // A bar per group, from the top of its first row, with a tick.
        let lines = of(&all, |s| matches!(s, Shape::Line));
        assert_eq!(lines.len(), 6);
        let bar = lines
            .iter()
            .find(|s| {
                s.hit == Some(Hit::Fold(Axis::Rows, outer, true)) && s.rect.2 - s.rect.0 == 1.0
            })
            .unwrap();
        assert_eq!(bar.rect.1, l.row_y(1) + 2.0);
        assert_eq!(bar.rect.3, grand.rect.1);
        // Dots beside the rows of detail, none on the summaries of groups.
        let dots = of(&all, |s| matches!(s, Shape::Dot));
        assert_eq!(dots.len(), 6);
        assert!(dots.iter().all(|d| d.rect.0 > inner.rect.0));
    }

    #[test]
    fn folded_groups_show_a_plus_and_no_bar() {
        let mut wb = nested();
        wb.outline_show_level(0, Axis::Rows, 2);
        wb.commit("level");
        let l = layout(&wb, 1.0, 0.0);
        let all = symbols(&wb.sheets[0], &l);
        let buttons = of(&all, |s| matches!(s, Shape::Button { .. }));
        let folded: Vec<&Symbol> = buttons
            .iter()
            .filter(|s| s.shape == Shape::Button { collapsed: true })
            .collect();
        assert_eq!(folded.len(), 2);
        assert!(matches!(
            folded[0].hit,
            Some(Hit::Fold(Axis::Rows, _, false))
        ));
        // Only the outer bar is left, and the dots of the two summaries.
        assert_eq!(of(&all, |s| matches!(s, Shape::Line)).len(), 2);
        assert_eq!(of(&all, |s| matches!(s, Shape::Dot)).len(), 0);
    }

    #[test]
    fn frozen_rows_and_scrolling_cut_the_bars() {
        let mut wb = nested();
        wb.set_frozen(0, 3, 0);
        wb.commit("freeze");
        // Scrolled by two rows: rows 4 and 5 are under the frozen ones.
        let l = layout(&wb, 1.0, 40.0);
        let all = symbols(&wb.sheets[0], &l);
        let outer = Group {
            level: 1,
            first: 1,
            last: 8,
        };
        let bars: Vec<Symbol> = all
            .iter()
            .copied()
            .filter(|s| {
                s.shape == Shape::Line
                    && s.hit == Some(Hit::Fold(Axis::Rows, outer, true))
                    && s.rect.2 - s.rect.0 == 1.0
            })
            .collect();
        // One piece in each strip, meeting at the freeze line.
        assert_eq!(bars.len(), 2);
        let edge = l.header_h + l.frozen_h;
        assert_eq!((bars[0].clip.1, bars[0].clip.3), (l.header_h, edge));
        assert_eq!(bars[0].rect.1, l.row_y(1) + 2.0);
        assert!(bars[0].rect.3 >= edge);
        assert_eq!(bars[1].clip.1, edge);
        assert!(bars[1].rect.1 < edge);
        // The summary of the first inner group scrolled out of sight.
        let buttons = of(&all, |s| matches!(s, Shape::Button { .. }));
        assert_eq!(buttons.len(), 2);
    }

    #[test]
    fn summaries_above_and_columns() {
        let mut wb = Workbook::new(Locale::en());
        wb.set_outline_settings(0, false, false, true);
        wb.outline_group(0, Axis::Rows, 2, 4).unwrap();
        wb.outline_group(0, Axis::Cols, 2, 3).unwrap();
        wb.commit("setup");
        let l = layout(&wb, 1.0, 0.0);
        let all = symbols(&wb.sheets[0], &l);
        let rows = Group {
            level: 1,
            first: 2,
            last: 4,
        };
        let button = all
            .iter()
            .find(|s| {
                s.hit == Some(Hit::Fold(Axis::Rows, rows, true))
                    && matches!(s.shape, Shape::Button { .. })
            })
            .unwrap();
        assert!(button.rect.1 >= l.row_y(1) && button.rect.3 <= l.row_y(2));
        let bar = all
            .iter()
            .find(|s| {
                s.shape == Shape::Line
                    && s.hit == Some(Hit::Fold(Axis::Rows, rows, true))
                    && s.rect.2 - s.rect.0 == 1.0
            })
            .unwrap();
        assert_eq!(bar.rect.1, button.rect.3);
        assert_eq!(bar.rect.3, l.row_y(4) + l.row_h(4) - 2.0);
        // The columns' outline lies above the headings, its button on
        // the column before the group.
        let cols = Group {
            level: 1,
            first: 2,
            last: 3,
        };
        let button = all
            .iter()
            .find(|s| {
                s.hit == Some(Hit::Fold(Axis::Cols, cols, true))
                    && matches!(s.shape, Shape::Button { .. })
            })
            .unwrap();
        assert!(button.rect.0 >= l.col_x(1) && button.rect.2 <= l.col_x(2));
        assert!(button.rect.3 <= l.margin_h);
        let levels = of(&all, |s| matches!(s, Shape::Level(_)));
        assert_eq!(levels.len(), 4);
        // The columns' level buttons stand beside the row headings.
        let col_levels: Vec<&Symbol> = levels
            .iter()
            .filter(|s| matches!(s.hit, Some(Hit::Level(Axis::Cols, _))))
            .collect();
        assert!(
            col_levels.iter().all(|s| s.rect.0 >= l.margin_w
                && s.rect.2 <= l.header_w
                && s.rect.3 <= l.margin_h)
        );
    }
}
