//! Sparklines: Insert > Sparklines, their drawing in the cells, and the
//! Sparkline tab shown while a cell with a sparkline is active.
//!
//! The picture of a sparkline is worked out by the engine
//! (`sparkline::shapes`), the same for the screen and for paper; this
//! file draws it between the fill of the cell and its text
//! (`paint_cells`, called by the grid) and holds the commands. The
//! dialogs are in `sparklines/dialogs.rs`, the drop-downs of the tab
//! (styles, colours) in `sparklines/gallery.rs`.

mod dialogs;
mod gallery;
#[cfg(test)]
mod ui_tests;

use super::{CellEdit, ChangeCause, Command, ContextTab, Cx, Feature, Intent, Painter};
use crate::app::{ExcelApp, MenuEntry};
use crate::grid::Draw;
use crate::i18n::{t, t_ctx};
use crate::ribbon::{group, row, rows};
use crate::theme;
use gpui::{AnyElement, Pixels, Point, Window, div, point, prelude::*, px};
use opensheet_engine::ops::PasteMode;
use opensheet_engine::protect::Action;
use opensheet_engine::sparkline::{AxisBound, SparkShape, SparklineGroup, SparklineType};
use opensheet_engine::workbook::ClearMode;
use opensheet_engine::Rect;

pub struct Sparklines;

/// The contextual tab.
const TAB: &str = "sparkline";

commands! {
    /// What the user can ask of sparklines.
    pub enum Cmd {
        // Insert
        Menu = "chart-line", t("Sparklines", "Graphiques sparkline");
        InsertLine = "sparkline-line", t_ctx("sparkline", "Line", "Courbe");
        InsertColumn = "sparkline-column", t_ctx("sparkline", "Column", "Histogramme");
        InsertWinLoss = "sparkline-winloss", t("Win/Loss", "Positif/Négatif");
        // The Sparkline tab
        EditDataMenu = "sparkline-edit", t("Edit Data", "Modifier les données");
        EditGroup = "", t("Edit Group Location & Data…", "Modifier l'emplacement et les données du groupe…");
        EditSingle = "", t("Edit Single Sparkline's Data…", "Modifier les données d'un seul graphique sparkline…");
        HiddenEmpty = "", t("Hidden & Empty Cells…", "Cellules masquées et vides…");
        TypeLine = "sparkline-line", t_ctx("sparkline", "Line", "Courbe");
        TypeColumn = "sparkline-column", t_ctx("sparkline", "Column", "Histogramme");
        TypeWinLoss = "sparkline-winloss", t("Win/Loss", "Positif/Négatif");
        HighPoint = "", t("High Point", "Point haut");
        LowPoint = "", t("Low Point", "Point bas");
        NegativePoints = "", t("Negative Points", "Points négatifs");
        FirstPoint = "", t("First Point", "Premier point");
        LastPoint = "", t("Last Point", "Dernier point");
        Markers = "", t("Markers", "Marqueurs");
        Styles = "cell-styles", t_ctx("sparkline", "Style", "Style");
        SparklineColor = "sparkline-color", t("Sparkline Color", "Couleur de sparkline");
        MarkerColor = "sparkline-marker", t("Marker Color", "Couleur de marqueur");
        Axis = "sparkline-axis", t_ctx("sparkline", "Axis", "Axe");
        Group = "outline-group", t_ctx("sparkline", "Group", "Grouper");
        Ungroup = "outline-ungroup", t_ctx("sparkline", "Ungroup", "Dissocier");
        ClearMenu = "clear", t_ctx("sparkline", "Clear", "Effacer");
        ClearSelected = "", t("Clear Selected Sparklines", "Effacer les graphiques sparkline sélectionnés");
        ClearGroups = "", t("Clear Selected Sparkline Groups", "Effacer les groupes de graphiques sparkline sélectionnés");
    }
}

/// The labels of the undo steps, in Excel's words.
pub(crate) mod labels {
    use crate::i18n::t;

    pub fn insert() -> &'static str {
        t("Insert Sparkline", "Insérer un graphique sparkline")
    }
    pub fn data() -> &'static str {
        t("Sparkline Data Change", "Modification des données du graphique sparkline")
    }
    pub fn kind() -> &'static str {
        t("Sparkline Type Change", "Modification du type de graphique sparkline")
    }
    pub fn style() -> &'static str {
        t("Sparkline Style Change", "Modification du style de graphique sparkline")
    }
    pub fn color() -> &'static str {
        t("Sparkline Color Change", "Modification de la couleur du graphique sparkline")
    }
    pub fn axis() -> &'static str {
        t("Sparkline Axis Change", "Modification de l’axe du graphique sparkline")
    }
    pub fn group() -> &'static str {
        t("Group Sparklines", "Grouper des graphiques sparkline")
    }
    pub fn ungroup() -> &'static str {
        t("Ungroup Sparklines", "Dissocier des graphiques sparkline")
    }
    pub fn clear() -> &'static str {
        t("Clear Sparklines", "Effacer les graphiques sparkline")
    }
}

/// The group of the sparkline in the active cell.
pub(crate) fn active_group(app: &ExcelApp) -> Option<&SparklineGroup> {
    let (row, col) = app.active();
    let sheet = &app.wb.sheets[app.sheet];
    sheet.sparkline_at(row, col).map(|(group, _)| &sheet.sparklines[group])
}

/// Where a drop-down of the command being run opens: under its button.
pub(crate) fn drop_position(app: &ExcelApp, window: &Window) -> Point<Pixels> {
    let anchor = app.command_anchor.and_then(|id| app.anchors.borrow().get(id).copied());
    match anchor {
        Some(bounds) => point(bounds.origin.x, bounds.origin.y + bounds.size.height + px(2.)),
        None => window.mouse_position(),
    }
}

/// Change the groups of the selected sparklines, in one undo step.
pub(crate) fn change(app: &mut ExcelApp, label: &str, cx: &mut Cx, f: impl Fn(&mut SparklineGroup)) {
    if !app.may(Intent::ChangeSheet, cx) {
        return;
    }
    let (sheet, ranges) = (app.sheet, app.sel().ranges.clone());
    if app.wb.update_sparkline_groups(sheet, &ranges, f) {
        app.commit(label, cx);
    }
}

fn set_kind(app: &mut ExcelApp, kind: SparklineType, cx: &mut Cx) {
    change(app, labels::kind(), cx, |group| group.kind = kind);
}

impl Cmd {
    /// The check boxes of the Show group: the option of a group each one
    /// stands for.
    fn show_flag(self, group: &mut SparklineGroup) -> Option<&mut bool> {
        match self {
            Cmd::HighPoint => Some(&mut group.high),
            Cmd::LowPoint => Some(&mut group.low),
            Cmd::NegativePoints => Some(&mut group.negative),
            Cmd::FirstPoint => Some(&mut group.first),
            Cmd::LastPoint => Some(&mut group.last),
            Cmd::Markers => Some(&mut group.markers),
            _ => None,
        }
    }

    fn kind(self) -> Option<SparklineType> {
        match self {
            Cmd::InsertLine | Cmd::TypeLine => Some(SparklineType::Line),
            Cmd::InsertColumn | Cmd::TypeColumn => Some(SparklineType::Column),
            Cmd::InsertWinLoss | Cmd::TypeWinLoss => Some(SparklineType::WinLoss),
            _ => None,
        }
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
        let menu = matches!(self, Cmd::Menu | Cmd::EditDataMenu | Cmd::ClearMenu | Cmd::Axis | Cmd::MarkerColor);
        if !menu && app.is_editing() && !app.commit_edit(None, window, cx) {
            return;
        }
        match self {
            Cmd::Menu => {
                let entries = vec![
                    MenuEntry::command(Cmd::InsertLine, app).icon(Cmd::InsertLine.icon()),
                    MenuEntry::command(Cmd::InsertColumn, app).icon(Cmd::InsertColumn.icon()),
                    MenuEntry::command(Cmd::InsertWinLoss, app).icon(Cmd::InsertWinLoss.icon()),
                ];
                app.open_command_menu(entries, 190.0, window, cx);
            }
            Cmd::InsertLine | Cmd::InsertColumn | Cmd::InsertWinLoss => {
                if let Some(kind) = self.kind() {
                    dialogs::open_create(app, kind, window, cx);
                }
            }
            Cmd::EditDataMenu => {
                let entries = vec![
                    MenuEntry::command(Cmd::EditGroup, app),
                    MenuEntry::command(Cmd::EditSingle, app),
                    MenuEntry::separator(),
                    MenuEntry::command(Cmd::HiddenEmpty, app),
                ];
                // Wide enough for the French labels.
                app.open_command_menu(entries, 400.0, window, cx);
            }
            Cmd::EditGroup => {
                dialogs::open_edit_group(app, window, cx);
            }
            Cmd::EditSingle => {
                dialogs::open_edit_single(app, window, cx);
            }
            Cmd::HiddenEmpty => {
                dialogs::open_hidden_empty(app, window, cx);
            }
            Cmd::TypeLine | Cmd::TypeColumn | Cmd::TypeWinLoss => {
                if let Some(kind) = self.kind() {
                    set_kind(app, kind, cx);
                }
            }
            Cmd::HighPoint | Cmd::LowPoint | Cmd::NegativePoints | Cmd::FirstPoint | Cmd::LastPoint | Cmd::Markers => {
                let on = !self.checked(app);
                change(app, labels::style(), cx, |group| {
                    if let Some(flag) = self.show_flag(group) {
                        *flag = on;
                    }
                });
            }
            Cmd::Styles => gallery::open_styles(app, window, cx),
            Cmd::SparklineColor => gallery::open_color(app, gallery::Slot::Series, window, cx),
            Cmd::MarkerColor => {
                let entries = gallery::Slot::POINTS
                    .into_iter()
                    .map(|slot| MenuEntry::new(slot.label(), move |app, window, cx| gallery::open_color(app, slot, window, cx)))
                    .collect();
                app.open_command_menu(entries, 200.0, window, cx);
            }
            Cmd::Axis => {
                let entries = axis_menu(app);
                app.open_command_menu(entries, 330.0, window, cx);
            }
            Cmd::Group => {
                let (sheet, ranges, lead) = (app.sheet, app.sel().ranges.clone(), app.active());
                if app.may(Intent::ChangeSheet, cx) && app.wb.group_sparklines(sheet, &ranges, lead) {
                    app.commit(labels::group(), cx);
                }
            }
            Cmd::Ungroup => {
                let (sheet, ranges) = (app.sheet, app.sel().ranges.clone());
                if app.may(Intent::ChangeSheet, cx) && app.wb.ungroup_sparklines(sheet, &ranges) {
                    app.commit(labels::ungroup(), cx);
                }
            }
            Cmd::ClearMenu => {
                let entries = vec![MenuEntry::command(Cmd::ClearSelected, app), MenuEntry::command(Cmd::ClearGroups, app)];
                app.open_command_menu(entries, 400.0, window, cx);
            }
            Cmd::ClearSelected | Cmd::ClearGroups => {
                let (sheet, ranges) = (app.sheet, app.sel().ranges.clone());
                if app.may(Intent::ChangeSheet, cx) && app.wb.clear_sparklines(sheet, &ranges, self == Cmd::ClearGroups) {
                    app.commit(labels::clear(), cx);
                }
            }
        }
    }

    fn enabled(self, app: &ExcelApp) -> bool {
        // Excel greys sparklines out on a protected sheet.
        if app.wb.permits(&Action::ChangeSheet { sheet: app.sheet }).is_err() {
            return false;
        }
        match self {
            Cmd::Menu | Cmd::InsertLine | Cmd::InsertColumn | Cmd::InsertWinLoss => true,
            // Markers belong to lines.
            Cmd::Markers => active_group(app).is_some_and(|group| group.kind == SparklineType::Line),
            _ => active_group(app).is_some(),
        }
    }

    fn checked(self, app: &ExcelApp) -> bool {
        let Some(group) = active_group(app) else {
            return false;
        };
        match self {
            Cmd::TypeLine | Cmd::TypeColumn | Cmd::TypeWinLoss => self.kind() == Some(group.kind),
            _ => {
                let mut copy = group.clone();
                self.show_flag(&mut copy).is_some_and(|flag| *flag)
            }
        }
    }
}

/// The Axis menu, as Excel lays it out: the horizontal axis, then the
/// two ends of the vertical one.
fn axis_menu(app: &ExcelApp) -> Vec<MenuEntry> {
    let Some(group) = active_group(app) else {
        return Vec::new();
    };
    let heading = |text: &'static str| MenuEntry::new(text, |_, _, _| {}).disabled(true);
    let flag = |text: &'static str, on: bool, set: fn(&mut SparklineGroup, bool)| {
        MenuEntry::new(text, move |app, _, cx| change(app, labels::axis(), cx, |group| set(group, !on))).checked(on)
    };
    let bound = |text: &'static str, current: AxisBound, wanted: AxisBound, max: bool| {
        MenuEntry::new(text, move |app, _, cx| {
            change(app, labels::axis(), cx, |group| if max { group.max = wanted } else { group.min = wanted })
        })
        .checked(current == wanted)
    };
    let end = |current: AxisBound, max: bool| {
        vec![
            bound(
                t("Automatic for Each Sparkline", "Automatique pour chaque graphique sparkline"),
                current,
                AxisBound::Individual,
                max,
            ),
            bound(t("Same for All Sparklines", "Identique pour tous les graphiques sparkline"), current, AxisBound::Group, max),
            MenuEntry::new(t("Custom Value…", "Valeur personnalisée…"), move |app, window, cx| {
                dialogs::open_axis_value(app, max, window, cx);
            })
            .checked(matches!(current, AxisBound::Custom(_))),
        ]
    };
    let dated = group.date_axis.is_some();
    let mut entries = vec![
        heading(t("Horizontal Axis Options", "Options de l'axe horizontal")),
        MenuEntry::new(t("General Axis Type", "Type de l'axe général"), |app, _, cx| {
            dialogs::set_date_axis(app, None, cx);
        })
        .checked(!dated),
        MenuEntry::new(t("Date Axis Type...", "Type de l'axe de date..."), |app, window, cx| {
            dialogs::open_date_range(app, window, cx);
        })
        .checked(dated),
        flag(t("Show Axis", "Afficher l'axe"), group.show_axis, |group, on| group.show_axis = on),
        flag(
            t("Plot Data Right-to-Left", "Tracer les données de droite à gauche"),
            group.right_to_left,
            |group, on| group.right_to_left = on,
        ),
        MenuEntry::separator(),
        heading(t("Vertical Axis Minimum Value Options", "Options - Valeur minimale de l'axe vertical")),
    ];
    entries.extend(end(group.min, false));
    entries.push(MenuEntry::separator());
    entries.push(heading(t("Vertical Axis Maximum Value Options", "Options - Valeur maximale de l'axe vertical")));
    entries.extend(end(group.max, true));
    entries
}

// ------------------------------------------------------------------ drawing

/// Draw the sparklines of the cells in view: over the fills of the pane,
/// under its text. Called by the grid for each pane.
pub(crate) fn paint_cells(p: &mut Painter) {
    let app = p.app;
    let sheet = &app.wb.sheets[app.sheet];
    if sheet.sparklines.is_empty() {
        return;
    }
    let (layout, zoom) = (p.layout, p.layout.zoom);
    for group in &sheet.sparklines {
        let in_view: Vec<_> = group.sparklines.iter().filter(|s| p.visible.contains(s.row, s.col)).collect();
        if in_view.is_empty() {
            continue;
        }
        let shared = app.wb.sparkline_scale(group);
        let dates = app.wb.sparkline_dates(group);
        for sparkline in in_view {
            // The sparkline of a merged cell fills the merged area.
            let cells = sheet.merge_at(sparkline.row, sparkline.col).unwrap_or(Rect::cell(sparkline.row, sparkline.col));
            if (cells.r0, cells.c0) != (sparkline.row, sparkline.col) {
                continue;
            }
            let (x0, y0, x1, y1) = layout.rect_px(cells);
            let (width, height) = (x1 - x0 - 1.0, y1 - y0 - 1.0);
            if width <= 2.0 || height <= 2.0 {
                continue;
            }
            let values = app.wb.sparkline_values(group, sparkline);
            let outer = std::mem::take(&mut p.items);
            for shape in opensheet_engine::sparkline::shapes_at(group, &values, dates.as_deref(), shared, width, height, zoom) {
                match shape {
                    SparkShape::Rect { x0: a, y0: b, x1: c, y1: d, color } => {
                        p.rect(x0 + a, y0 + b, x0 + c, y0 + d, theme::engine_color(color));
                    }
                    SparkShape::Line { points, color, width } => {
                        let points: Vec<(f32, f32)> = points.iter().map(|(x, y)| (x0 + x, y0 + y)).collect();
                        p.polyline(&points, width, theme::engine_color(color));
                    }
                }
            }
            let items = std::mem::replace(&mut p.items, outer);
            if !items.is_empty() {
                p.push(Draw::Clipped {
                    clip: layout.bounds_px(x0.max(p.clip.0), y0.max(p.clip.1), (x1 - 1.0).min(p.clip.2), (y1 - 1.0).min(p.clip.3)),
                    items,
                });
            }
        }
    }
}

impl Feature for Sparklines {
    fn name(&self) -> &'static str {
        "sparklines"
    }

    fn context_tabs(&self, app: &ExcelApp) -> Vec<ContextTab> {
        match active_group(app) {
            Some(_) => vec![ContextTab {
                id: TAB,
                title: t_ctx("tab", "Sparkline", "Graphique sparkline"),
            }],
            None => Vec::new(),
        }
    }

    fn render_tab(&self, app: &mut ExcelApp, _tab: &'static str, _window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
        tab_groups(app, cx)
    }

    // Sparklines travel with their cells: pasted and filled cells get
    // copies, cut ones take theirs along, and clearing everything clears
    // them too. All of it in the undo step of the edit.
    fn cells_written(&self, app: &mut ExcelApp, edit: &CellEdit, _cx: &mut Cx) {
        match edit.cause {
            ChangeCause::Paste {
                from: Some((sheet, rect)),
                cut,
                mode: PasteMode::All | PasteMode::AllExceptBorders,
            } => {
                // A copy laid several times over a larger destination
                // brings its sparklines each time.
                let (rows, cols) = (rect.rows().max(1), rect.cols().max(1));
                let (down, across) = if cut { (1, 1) } else { ((edit.rect.rows() / rows).max(1), (edit.rect.cols() / cols).max(1)) };
                for i in 0..down {
                    for j in 0..across {
                        let dest = (edit.rect.r0 + i * rows, edit.rect.c0 + j * cols);
                        app.wb.copy_sparklines(sheet, rect, edit.sheet, dest, cut);
                    }
                }
            }
            ChangeCause::Fill => app.wb.fill_sparklines(edit.sheet, edit.rect),
            ChangeCause::Clear(ClearMode::All) => {
                app.wb.clear_sparklines(edit.sheet, &[edit.rect], false);
            }
            _ => {}
        }
    }
}

// ------------------------------------------------------------------- ribbon

/// The groups of the Sparkline tab.
fn tab_groups(app: &ExcelApp, cx: &mut Cx) -> Vec<AnyElement> {
    let check = |id: &'static str, cmd: Cmd, cx: &mut Cx| -> AnyElement {
        // A greyed-out box keeps its place (Markers, for columns).
        div().when(!cmd.enabled(app), |d| d.opacity(0.45)).child(app.cmd_check(id, cmd, cx)).into_any_element()
    };
    vec![
        group(t_ctx("group", "Sparkline", "Graphique sparkline"), row(vec![app.cmd_large("sp-edit", Cmd::EditDataMenu, true, cx)]))
            .into_any_element(),
        group(
            t_ctx("group", "Type", "Type"),
            row(vec![
                app.cmd_large("sp-line", Cmd::TypeLine, false, cx),
                app.cmd_large("sp-column", Cmd::TypeColumn, false, cx),
                app.cmd_large("sp-winloss", Cmd::TypeWinLoss, false, cx),
            ]),
        )
        .into_any_element(),
        group(
            t_ctx("group", "Show", "Afficher"),
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(rows(vec![
                    check("sp-high", Cmd::HighPoint, cx),
                    check("sp-low", Cmd::LowPoint, cx),
                    check("sp-negative", Cmd::NegativePoints, cx),
                ]))
                .child(rows(vec![
                    check("sp-first", Cmd::FirstPoint, cx),
                    check("sp-last", Cmd::LastPoint, cx),
                    check("sp-markers", Cmd::Markers, cx),
                ])),
        )
        .into_any_element(),
        group(
            t_ctx("group", "Style", "Style"),
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(app.cmd_large("sp-styles", Cmd::Styles, true, cx))
                .child(rows(vec![
                    app.cmd_labeled("sp-color", Cmd::SparklineColor, true, cx),
                    app.cmd_labeled("sp-marker-color", Cmd::MarkerColor, true, cx),
                ])),
        )
        .into_any_element(),
        group(
            t_ctx("group", "Group", "Groupe"),
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(app.cmd_large("sp-axis", Cmd::Axis, true, cx))
                .child(rows(vec![
                    app.cmd_labeled("sp-group", Cmd::Group, false, cx),
                    app.cmd_labeled("sp-ungroup", Cmd::Ungroup, false, cx),
                    app.cmd_labeled("sp-clear", Cmd::ClearMenu, true, cx),
                ])),
        )
        .border_r_0()
        .into_any_element(),
    ]
}
