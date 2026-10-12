//! Pivot tables: the creation dialog, the PivotTable Fields pane, the
//! buttons on pivot cells, and the "PivotTable Analyze" and "Design" tabs.
//!
//! * `create.rs`: Create PivotTable, Move PivotTable, Change Data Source.
//! * `pane.rs`: the PivotTable Fields pane.
//! * `sheet.rs`: what is drawn and clicked on the grid (drop-down and
//!   expand / collapse buttons, the empty report, double clicks).
//! * `filter.rs`: the drop-down of a field and the Sort, Label Filter,
//!   Value Filter and Top 10 dialogs.
//! * `dialogs.rs`: Value Field Settings, Field Settings, PivotTable
//!   Options, Grouping, Show Report Filter Pages.
//! * `menus.rs`: the drop-downs of the two tabs and the context menu.
//! * `styles.rs`: the gallery of PivotTable styles.
//! * `actions.rs`: what the commands do to the workbook.
//! * `logic.rs`: what is decided without a window.
//!
//! Every change goes through [`perform`]: it asks the protection, runs
//! the engine command, asks Excel's question when the report would cover
//! data, fits the columns, and commits one undo step.

mod actions;
mod create;
mod dialogs;
mod filter;
mod logic;
mod menus;
mod pane;
mod sheet;
mod styles;

use super::{CellEdit, Command, ContextTab, Cx, Feature, Intent, Painter, Zone, ZoneEvent, ZoneResponse, tables};
use crate::app::{ExcelApp, MenuEntry};
use crate::grid::{GridLayout, cell_font, pt_to_px};
use crate::i18n::{t, t_ctx};
use crate::ribbon::{group, row, rows};
use crate::shell::{Alert, Modal};
use crate::text_input::{InputEvent, TextInput};
use crate::theme::c;
use crate::widgets::TextField;
use gpui::{
    AnyElement, Entity, Focusable, Pixels, Point, SharedString, Subscription, TextRun, Window, WindowTextSystem, div,
    prelude::*, px,
};
use logic::Target;
use opensheet_engine::pivot::{PivotButton, PivotCell, PivotError, PivotTable, Report};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

pub struct Pivots;

const ANALYZE_TAB: &str = "pivot-analyze";
const DESIGN_TAB: &str = "pivot-design";
/// Id of the PivotTable Fields pane.
const PANE: &str = "pivot-fields";

commands! {
    /// What the user can ask of pivot tables.
    pub enum Cmd {
        Insert = "pivot", t("PivotTable", "Tableau croisé dynamique");
        Recommended = "pivot", t("Recommended PivotTables", "Tableaux croisés dynamiques recommandés");
        InsertTimeline = "clock", t("Timeline", "Chronologie");
        // PivotTable Analyze
        Options = "pivot", t("Options", "Options");
        FieldSettings = "format-cells", t("Field Settings", "Paramètres de champ");
        ExpandField = "plus", t("Expand Field", "Développer le champ");
        CollapseField = "minus", t("Collapse Field", "Réduire le champ");
        GroupSelection = "group-objects", t("Group Selection", "Grouper la sélection");
        Ungroup = "group-objects", t("Ungroup", "Dissocier");
        GroupField = "group-objects", t("Group Field", "Grouper les champs");
        FilterConnections = "link", t("Filter Connections", "Connexions de filtre");
        Refresh = "refresh", t("Refresh", "Actualiser");
        ChangeSource = "data", t("Change Data Source", "Changer la source de données");
        Clear = "clear", t("Clear", "Effacer");
        Select = "grid", t_ctx("pivot", "Select", "Sélectionner");
        Move = "pivot", t("Move PivotTable", "Déplacer le tableau croisé dynamique");
        CalculatedFields = "function", t("Fields, Items, & Sets", "Champs, éléments et jeux");
        PivotChart = "chart-bar", t("PivotChart", "Graphique croisé dynamique");
        FieldList = "columns", t("Field List", "Liste des champs");
        Buttons = "plus", t("+/- Buttons", "Boutons +/-");
        FieldHeaders = "table", t("Field Headers", "En-têtes de champ");
        // Design
        Subtotals = "calculator", t("Subtotals", "Sous-totaux");
        GrandTotals = "calculator", t("Grand Totals", "Totaux généraux");
        ReportLayout = "table", t("Report Layout", "Disposition du rapport");
        BlankRows = "insert-cells", t("Blank Rows", "Lignes vides");
        RowHeaders = "", t("Row Headers", "En-têtes de lignes");
        ColumnHeaders = "", t("Column Headers", "En-têtes de colonnes");
        BandedRows = "", t("Banded Rows", "Lignes à bandes");
        BandedColumns = "", t("Banded Columns", "Colonnes à bandes");
        Styles = "cell-styles", t("Quick Styles", "Styles rapides");
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
            Cmd::Insert | Cmd::Recommended | Cmd::InsertTimeline => true,
            Cmd::FieldSettings => active_target(app).is_some(),
            Cmd::ExpandField | Cmd::CollapseField => actions::drill_field(app).is_some(),
            Cmd::GroupSelection => actions::can_group(app),
            Cmd::GroupField => actions::group_field(app).is_some(),
            Cmd::Ungroup => actions::can_ungroup(app),
            _ => active_pivot(app).is_some(),
        }
    }

    fn checked(self, app: &ExcelApp) -> bool {
        let Some(pivot) = active_pivot(app) else {
            return false;
        };
        match self {
            Cmd::FieldList => app.pane_open(PANE),
            Cmd::Buttons => pivot.options.show_drill,
            Cmd::FieldHeaders => pivot.options.show_headers,
            Cmd::RowHeaders => pivot.style.row_headers,
            Cmd::ColumnHeaders => pivot.style.col_headers,
            Cmd::BandedRows => pivot.style.row_stripes,
            Cmd::BandedColumns => pivot.style.col_stripes,
            _ => false,
        }
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        if app.is_editing() && !app.commit_edit(None, window, cx) {
            return;
        }
        match self {
            // On the Insert tab the button opens Excel's list of sources;
            // from anywhere else (Table Design > Summarize with PivotTable)
            // the dialog opens at once.
            Cmd::Insert if app.command_anchor == Some("in-pivot") => menus::insert_menu(app, window, cx),
            Cmd::Insert | Cmd::Recommended => create::open_create(app, window, cx),
            // The engine has no timelines, slicer connections, calculated
            // fields nor pivot charts.
            Cmd::InsertTimeline | Cmd::FilterConnections | Cmd::CalculatedFields | Cmd::PivotChart => {
                app.not_implemented(self.label(), cx)
            }
            Cmd::Options => menus::options_menu(app, window, cx),
            Cmd::FieldSettings => dialogs::open_settings(app, window, cx),
            Cmd::ExpandField => actions::collapse_active_field(app, false, window, cx),
            Cmd::CollapseField => actions::collapse_active_field(app, true, window, cx),
            Cmd::GroupSelection => actions::group_selection(app, window, cx),
            Cmd::GroupField => actions::open_group_field(app, window, cx),
            Cmd::Ungroup => actions::ungroup(app, window, cx),
            Cmd::Refresh => menus::refresh_menu(app, window, cx),
            Cmd::ChangeSource => menus::source_menu(app, window, cx),
            Cmd::Clear => menus::clear_menu(app, window, cx),
            Cmd::Select => menus::select_menu(app, window, cx),
            Cmd::Move => create::open_move(app, window, cx),
            Cmd::FieldList => toggle_field_list(app, cx),
            Cmd::Buttons => {
                update_active(app, t("+/- Buttons", "Boutons +/-"), window, cx, |p| {
                    p.options.show_drill = !p.options.show_drill
                });
            }
            Cmd::FieldHeaders => {
                update_active(app, t("Field Headers", "En-têtes de champ"), window, cx, |p| {
                    p.options.show_headers = !p.options.show_headers
                });
            }
            Cmd::Subtotals => menus::subtotals_menu(app, window, cx),
            Cmd::GrandTotals => menus::grand_totals_menu(app, window, cx),
            Cmd::ReportLayout => menus::layout_menu(app, window, cx),
            Cmd::BlankRows => menus::blank_rows_menu(app, window, cx),
            Cmd::RowHeaders => style_option(app, self, window, cx, |s| s.row_headers = !s.row_headers),
            Cmd::ColumnHeaders => style_option(app, self, window, cx, |s| s.col_headers = !s.col_headers),
            Cmd::BandedRows => style_option(app, self, window, cx, |s| s.row_stripes = !s.row_stripes),
            Cmd::BandedColumns => style_option(app, self, window, cx, |s| s.col_stripes = !s.col_stripes),
            Cmd::Styles => styles::open_gallery(app, window, cx),
        }
    }
}

/// Flip one of the "PivotTable Style Options".
fn style_option(
    app: &mut ExcelApp,
    cmd: Cmd,
    window: &mut Window,
    cx: &mut Cx,
    flip: impl Fn(&mut opensheet_engine::pivot::PivotStyle) + 'static,
) {
    update_active(app, cmd.label(), window, cx, move |p| flip(&mut p.style));
}

// -------------------------------------------------------------------- state

/// The name box of the ribbon ("PivotTable Name", "Active Field").
struct NameBox {
    input: Entity<TextInput>,
    _subscription: Subscription,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BoxKind {
    PivotName,
    ActiveField,
}

/// What the pivot interface remembers between events.
#[derive(Default)]
struct PivotUi {
    /// "Field List" is switched off.
    list_hidden: bool,
    /// The pane was open when last looked at.
    pane_shown: bool,
    /// "Defer Layout Update" is ticked.
    defer: bool,
    /// The definition the pane edits while layout updates are deferred.
    pending: Option<PivotTable>,
    pivot_name: Option<NameBox>,
    active_field: Option<NameBox>,
    /// The cell whose drop-down was last opened.
    open_filter: Option<(u32, u32)>,
    /// Where the press that closed that drop-down landed, and the one that
    /// closed the styles gallery: a press on their own button must not
    /// open them again.
    filter_closed: Dismissed,
    gallery_closed: Dismissed,
}

/// Where the press that closed a drop-down landed (set by the drop-down
/// itself, read by the button it hangs from).
type Dismissed = Rc<Cell<Option<Point<Pixels>>>>;

/// True when the press that closed a drop-down is the one now handled at
/// `at`: it landed on the button the drop-down hangs from.
fn just_closed(closed: &Dismissed, at: Point<Pixels>) -> bool {
    closed
        .take()
        .is_some_and(|press| (press.x - at.x).abs() < px(4.) && (press.y - at.y).abs() < px(4.))
}

// ------------------------------------------------------------------ helpers

/// The pivot table the active cell is in.
fn active_pivot(app: &ExcelApp) -> Option<&PivotTable> {
    let (row, col) = app.active();
    app.wb.pivot_at(app.sheet, row, col)
}

/// What the active cell is in its pivot table.
fn active_cell(app: &ExcelApp) -> Option<PivotCell> {
    let (row, col) = app.active();
    app.wb.pivot_cell(app.sheet, row, col)
}

/// The field or value field the active cell stands for.
fn active_target(app: &ExcelApp) -> Option<Target> {
    active_cell(app).as_ref().and_then(logic::target_of)
}

/// How a command ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Done,
    /// The report would cover data: the user is being asked, and the
    /// command runs again if the answer is yes.
    Asked,
    /// Refused, and the user was told why.
    Refused,
}

/// A change to the workbook; the flag allows the report to cover cells
/// holding data.
type Op = Rc<dyn Fn(&mut ExcelApp, bool) -> Result<(), PivotError>>;

/// Run a pivot command as one undo step named `label`.
fn perform(
    app: &mut ExcelApp,
    label: &'static str,
    window: &mut Window,
    cx: &mut Cx,
    op: impl Fn(&mut ExcelApp, bool) -> Result<(), PivotError> + 'static,
) -> Outcome {
    if !app.may(Intent::UsePivotTable, cx) {
        return Outcome::Refused;
    }
    attempt(app, label, Rc::new(op), false, window, cx)
}

/// `perform` for the OK button of a dialog. As in Excel, the dialog stays
/// open when the command is refused, and closes before the question about
/// replacing data is asked.
fn perform_in_dialog(
    app: &mut ExcelApp,
    modal: &Modal,
    label: &'static str,
    window: &mut Window,
    cx: &mut Cx,
    op: impl Fn(&mut ExcelApp, bool) -> Result<(), PivotError> + 'static,
) -> Outcome {
    if !app.may(Intent::UsePivotTable, cx) {
        return Outcome::Refused;
    }
    let op: Op = Rc::new(op);
    match try_op(app, label, &op, false, window, cx) {
        Ok(()) => {
            app.close_modal_of(&modal.focus, window, cx);
            Outcome::Done
        }
        Err(PivotError::WouldOverwrite(_)) => {
            app.close_modal_of(&modal.focus, window, cx);
            ask_replace(app, label, op, cx);
            Outcome::Asked
        }
        Err(err) => {
            app.alert(Alert::new("OpenSheet", logic::error_text(&err)).warning(), cx);
            Outcome::Refused
        }
    }
}

/// Run a command once. A refused command leaves nothing behind.
fn try_op(
    app: &mut ExcelApp,
    label: &'static str,
    op: &Op,
    overwrite: bool,
    window: &mut Window,
    cx: &mut Cx,
) -> Result<(), PivotError> {
    let before: Vec<(u32, Arc<Report>)> = app.wb.pivots().map(|(_, p)| (p.id, p.report.clone())).collect();
    let home = active_pivot(app).map(|p| p.id);
    match op(app, overwrite) {
        Ok(()) => {
            settle(app, label, &before, home, window, cx);
            Ok(())
        }
        Err(err) => {
            // A command made of several steps may have begun before one
            // of them was refused.
            app.wb.rollback();
            Err(err)
        }
    }
}

/// Excel's question when a report would cover cells holding data; the
/// command runs again, allowed to, on "OK".
fn ask_replace(app: &mut ExcelApp, label: &'static str, op: Op, cx: &mut Cx) {
    let sheet = app.wb.sheets[app.sheet].name.clone();
    app.alert(
        Alert::new("OpenSheet", logic::replace_question(&sheet))
            .warning()
            .button(t("OK", "OK"), move |app, window, cx| {
                attempt(app, label, op.clone(), true, window, cx);
            })
            .close_button(t("Cancel", "Annuler")),
        cx,
    );
}

fn attempt(app: &mut ExcelApp, label: &'static str, op: Op, overwrite: bool, window: &mut Window, cx: &mut Cx) -> Outcome {
    match try_op(app, label, &op, overwrite, window, cx) {
        Ok(()) => Outcome::Done,
        Err(PivotError::WouldOverwrite(_)) if !overwrite => {
            ask_replace(app, label, op, cx);
            Outcome::Asked
        }
        Err(err) => {
            app.alert(Alert::new("OpenSheet", logic::error_text(&err)).warning(), cx);
            Outcome::Refused
        }
    }
}

/// After a command wrote its reports: fit the columns ("Autofit column
/// widths on update"), commit, and keep the active cell in its PivotTable.
fn settle(
    app: &mut ExcelApp,
    label: &'static str,
    before: &[(u32, Arc<Report>)],
    home: Option<u32>,
    window: &mut Window,
    cx: &mut Cx,
) {
    let rewritten: Vec<(usize, u32)> = app
        .wb
        .pivots()
        .filter(|(_, p)| !before.iter().any(|(id, report)| *id == p.id && Arc::ptr_eq(report, &p.report)))
        .map(|(sheet, p)| (sheet, p.id))
        .collect();
    let ts = window.text_system().clone();
    for (sheet, id) in rewritten {
        autofit(app, sheet, id, &ts);
    }
    // What the pane had put aside described the report as it was.
    app.ext_mut::<PivotUi>().pending = None;
    app.commit(label, cx);
    // A report that shrank may have left the active cell outside: Excel
    // keeps the selection in the PivotTable, and with it the field list.
    if let Some(id) = home.filter(|id| active_pivot(app).map(|p| p.id) != Some(*id)) {
        let corner = app.wb.pivot(id).filter(|(sheet, _)| *sheet == app.sheet).and_then(|(_, p)| p.location);
        if let Some(body) = corner {
            app.select_cell(body.r0, body.c0, cx);
        }
    }
}

/// Give the columns of a PivotTable the width of what they show.
fn autofit(app: &mut ExcelApp, sheet: usize, id: u32, ts: &WindowTextSystem) {
    let Some((_, pivot)) = app.wb.pivot(id) else {
        return;
    };
    let fonts = app.fonts.clone();
    let widths = logic::fit_widths(&app.wb, sheet, pivot, |text, style| {
        let text: SharedString = text.replace(['\n', '\r'], " ").into();
        let run = TextRun {
            len: text.len(),
            font: cell_font(style, &fonts),
            color: c(0),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        f32::from(ts.shape_line(text, px(pt_to_px(style.font.size_pt())), &[run], None).width)
    });
    for (col, width) in widths {
        app.wb.set_col_width(sheet, col, Some(width));
    }
}

/// Change the definition of a PivotTable and write its report again.
fn update(
    app: &mut ExcelApp,
    id: u32,
    label: &'static str,
    window: &mut Window,
    cx: &mut Cx,
    edit: impl Fn(&mut PivotTable) + 'static,
) -> Outcome {
    perform(app, label, window, cx, move |app, overwrite| app.wb.pivot_update(id, overwrite, &edit))
}

/// `update` for the OK button of a dialog (see `perform_in_dialog`).
fn update_in_dialog(
    app: &mut ExcelApp,
    modal: &Modal,
    id: u32,
    label: &'static str,
    window: &mut Window,
    cx: &mut Cx,
    edit: impl Fn(&mut PivotTable) + 'static,
) -> Outcome {
    perform_in_dialog(app, modal, label, window, cx, move |app, overwrite| {
        app.wb.pivot_update(id, overwrite, &edit)
    })
}

/// `update` for the PivotTable of the active cell.
fn update_active(
    app: &mut ExcelApp,
    label: &'static str,
    window: &mut Window,
    cx: &mut Cx,
    edit: impl Fn(&mut PivotTable) + 'static,
) -> Outcome {
    match active_pivot(app).map(|p| p.id) {
        Some(id) => update(app, id, label, window, cx, edit),
        None => Outcome::Refused,
    }
}

/// A command inserted a sheet at `index`: the views follow the sheets.
fn sheet_inserted(app: &mut ExcelApp, index: usize) {
    let index = index.min(app.views.len());
    app.views.insert(index, crate::app::SheetView::default());
}

// ---------------------------------------------------------- the field list

fn toggle_field_list(app: &mut ExcelApp, cx: &mut Cx) {
    let open = app.pane_open(PANE);
    let ui = app.ext_mut::<PivotUi>();
    ui.list_hidden = open;
    // Closing it here is not the user closing the pane with its own box.
    ui.pane_shown = false;
    if open {
        app.close_pane(PANE, cx);
    }
    sync_pane(app, cx);
}

/// Show the PivotTable Fields pane while the active cell is in a
/// PivotTable (unless "Field List" is off), hide it elsewhere.
fn sync_pane(app: &mut ExcelApp, cx: &mut Cx) {
    let active = active_pivot(app).map(|p| p.id);
    let open = app.pane_open(PANE);
    let ui = app.ext_mut::<PivotUi>();
    if ui.pane_shown && !open {
        // Closed with its close box: as if "Field List" were switched off.
        ui.list_hidden = true;
    }
    ui.pane_shown = open;
    if ui.pending.as_ref().is_some_and(|p| Some(p.id) != active) {
        ui.pending = None;
    }
    let wanted = active.is_some() && !ui.list_hidden;
    if wanted && !open {
        let weak = cx.weak_entity();
        let view = cx.new(|cx| pane::FieldsPane::new(weak, cx));
        app.open_pane(PANE, t("PivotTable Fields", "Champs de tableau croisé dynamique"), view, cx);
        app.ext_mut::<PivotUi>().pane_shown = true;
    } else if !wanted && open {
        app.close_pane(PANE, cx);
        app.ext_mut::<PivotUi>().pane_shown = false;
    }
}

// ------------------------------------------------------------------- ribbon

/// The text field of a name box of the ribbon, created on first use.
fn name_box(app: &mut ExcelApp, kind: BoxKind, window: &mut Window, cx: &mut Cx) -> Entity<TextInput> {
    let ui = app.ext_mut::<PivotUi>();
    let slot = match kind {
        BoxKind::PivotName => &mut ui.pivot_name,
        BoxKind::ActiveField => &mut ui.active_field,
    };
    if let Some(existing) = slot {
        return existing.input.clone();
    }
    let input = cx.new(|cx| {
        let mut input = TextInput::new(cx);
        input.select_on_focus = true;
        input
    });
    let subscription = cx.subscribe_in(&input, window, move |app, _, ev: &InputEvent, window, cx| {
        name_box_event(app, kind, ev, window, cx)
    });
    let ui = app.ext_mut::<PivotUi>();
    let slot = match kind {
        BoxKind::PivotName => &mut ui.pivot_name,
        BoxKind::ActiveField => &mut ui.active_field,
    };
    *slot = Some(NameBox {
        input: input.clone(),
        _subscription: subscription,
    });
    input
}

/// What a name box shows while nothing is typed in it.
fn name_box_text(app: &ExcelApp, kind: BoxKind) -> String {
    let Some(pivot) = active_pivot(app) else {
        return String::new();
    };
    match kind {
        BoxKind::PivotName => pivot.name.clone(),
        BoxKind::ActiveField => active_target(app).map(|target| logic::target_name(pivot, target)).unwrap_or_default(),
    }
}

fn name_box_event(app: &mut ExcelApp, kind: BoxKind, ev: &InputEvent, window: &mut Window, cx: &mut Cx) {
    match ev {
        InputEvent::Enter { .. } | InputEvent::Tab { .. } => {
            let ui = app.ext_mut::<PivotUi>();
            let slot = match kind {
                BoxKind::PivotName => &ui.pivot_name,
                BoxKind::ActiveField => &ui.active_field,
            };
            let Some(text) = slot.as_ref().map(|b| b.input.read(cx).text().trim().to_string()) else {
                return;
            };
            window.focus(&app.grid_focus);
            if text != name_box_text(app, kind) {
                match kind {
                    BoxKind::PivotName => actions::rename_pivot(app, text, window, cx),
                    BoxKind::ActiveField => actions::rename_active_field(app, text, window, cx),
                }
            }
            cx.notify();
        }
        InputEvent::Escape => {
            window.focus(&app.grid_focus);
            cx.notify();
        }
        InputEvent::Blurred => cx.notify(),
        _ => {}
    }
}

/// A name box as the ribbon shows it: the name of what is active, unless
/// the user is typing another one.
fn name_box_element(app: &mut ExcelApp, kind: BoxKind, window: &mut Window, cx: &mut Cx) -> AnyElement {
    let input = name_box(app, kind, window, cx);
    let current = name_box_text(app, kind);
    if !input.read(cx).focus_handle(cx).is_focused(window) && input.read(cx).text() != current {
        input.update(cx, |input, cx| {
            input.set_text(current, cx);
            // A name longer than the box shows its beginning.
            input.set_selection(0..0, cx);
        });
    }
    div().px(px(2.)).child(TextField::new(&input).width(150.0)).into_any_element()
}

/// The groups of the "PivotTable Analyze" tab.
fn analyze_tab(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
    let caption = |text: &'static str| div().px(px(4.)).whitespace_nowrap().child(text).into_any_element();
    let pivot_name = name_box_element(app, BoxKind::PivotName, window, cx);
    let active_field = name_box_element(app, BoxKind::ActiveField, window, cx);
    vec![
        group(
            t("PivotTable", "Tableau croisé dynamique"),
            rows(vec![
                caption(t("PivotTable Name:", "Nom du tableau croisé dynamique :")),
                pivot_name,
                app.cmd_labeled("pa-options", Cmd::Options, true, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Active Field", "Champ actif"),
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(rows(vec![
                    caption(t("Active Field:", "Champ actif :")),
                    active_field,
                    app.cmd_labeled("pa-field-settings", Cmd::FieldSettings, false, cx),
                ]))
                .child(rows(vec![
                    app.cmd_labeled("pa-expand", Cmd::ExpandField, false, cx),
                    app.cmd_labeled("pa-collapse", Cmd::CollapseField, false, cx),
                ])),
        )
        .into_any_element(),
        group(
            t_ctx("pivot", "Group", "Groupe"),
            rows(vec![
                app.cmd_labeled("pa-group", Cmd::GroupSelection, false, cx),
                app.cmd_labeled("pa-ungroup", Cmd::Ungroup, false, cx),
                app.cmd_labeled("pa-group-field", Cmd::GroupField, false, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Filter", "Filtrer"),
            row(vec![
                app.cmd_large("pa-slicer", tables::Cmd::InsertSlicer, false, cx),
                app.cmd_large("pa-timeline", Cmd::InsertTimeline, false, cx),
                app.cmd_large("pa-connections", Cmd::FilterConnections, false, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Data", "Données"),
            row(vec![
                app.cmd_large("pa-refresh", Cmd::Refresh, true, cx),
                app.cmd_large("pa-source", Cmd::ChangeSource, true, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Actions", "Actions"),
            rows(vec![
                app.cmd_labeled("pa-clear", Cmd::Clear, true, cx),
                app.cmd_labeled("pa-select", Cmd::Select, true, cx),
                app.cmd_labeled("pa-move", Cmd::Move, false, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Calculations", "Calculs"),
            row(vec![app.cmd_large("pa-calculated", Cmd::CalculatedFields, true, cx)]),
        )
        .into_any_element(),
        group(
            t("Tools", "Outils"),
            row(vec![
                app.cmd_large("pa-chart", Cmd::PivotChart, false, cx),
                app.cmd_large("pa-recommended", Cmd::Recommended, false, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("Show", "Afficher"),
            rows(vec![
                app.cmd_labeled("pa-field-list", Cmd::FieldList, false, cx),
                app.cmd_labeled("pa-buttons", Cmd::Buttons, false, cx),
                app.cmd_labeled("pa-headers", Cmd::FieldHeaders, false, cx),
            ]),
        )
        .border_r_0()
        .into_any_element(),
    ]
}

/// The groups of the "Design" tab of pivot tables.
fn design_tab(app: &mut ExcelApp, cx: &mut Cx) -> Vec<AnyElement> {
    vec![
        group(
            t("Layout", "Disposition"),
            row(vec![
                app.cmd_large("pd-subtotals", Cmd::Subtotals, true, cx),
                app.cmd_large("pd-totals", Cmd::GrandTotals, true, cx),
                app.cmd_large("pd-layout", Cmd::ReportLayout, true, cx),
                app.cmd_large("pd-blank", Cmd::BlankRows, true, cx),
            ]),
        )
        .into_any_element(),
        group(
            t("PivotTable Style Options", "Options de style de tableau croisé dynamique"),
            div()
                .flex()
                .gap(px(8.))
                .child(rows(vec![
                    app.cmd_check("pd-row-headers", Cmd::RowHeaders, cx),
                    app.cmd_check("pd-col-headers", Cmd::ColumnHeaders, cx),
                ]))
                .child(rows(vec![
                    app.cmd_check("pd-banded-rows", Cmd::BandedRows, cx),
                    app.cmd_check("pd-banded-cols", Cmd::BandedColumns, cx),
                ])),
        )
        .into_any_element(),
        group(
            t("PivotTable Styles", "Styles de tableau croisé dynamique"),
            styles::ribbon_gallery(app, cx),
        )
        .border_r_0()
        .into_any_element(),
    ]
}

impl Feature for Pivots {
    fn name(&self) -> &'static str {
        "pivot"
    }

    fn context_tabs(&self, app: &ExcelApp) -> Vec<ContextTab> {
        match active_pivot(app) {
            Some(_) => vec![
                ContextTab {
                    id: ANALYZE_TAB,
                    title: t("PivotTable Analyze", "Analyse du tableau croisé dynamique"),
                },
                ContextTab {
                    id: DESIGN_TAB,
                    title: t_ctx("pivot", "Design", "Création"),
                },
            ],
            None => Vec::new(),
        }
    }

    fn render_tab(&self, app: &mut ExcelApp, tab: &'static str, window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
        if tab == DESIGN_TAB {
            design_tab(app, cx)
        } else {
            analyze_tab(app, window, cx)
        }
    }

    fn paint(&self, p: &mut Painter) {
        sheet::paint(p);
    }

    /// Nothing is laid over the grid; this hook runs at every repaint with
    /// the application in hand, which is when the field list follows the
    /// active cell (after a click, an undo, a change of sheet).
    fn overlay_elements(&self, app: &mut ExcelApp, _layout: &GridLayout, _window: &mut Window, cx: &mut Cx) -> Vec<AnyElement> {
        sync_pane(app, cx);
        Vec::new()
    }

    fn zone_mouse_down(&self, app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) -> ZoneResponse {
        sheet::mouse_down(app, zone, ev, window, cx)
    }

    /// Alt+Down on a header or a filter cell opens its drop-down.
    fn cell_drop_down(&self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> bool {
        let (row, col) = app.active();
        match app.wb.pivot_cell(app.sheet, row, col) {
            Some(cell) if matches!(cell.button, Some(PivotButton::Filter { .. })) => {
                filter::toggle_drop_down(app, cell.pivot, row, col, None, window, cx);
                true
            }
            _ => false,
        }
    }

    fn guard(&self, app: &ExcelApp, intent: &Intent) -> Option<SharedString> {
        logic::refusal(&app.wb, app.sheet, intent).map(SharedString::from)
    }

    /// Cells covering a PivotTable whole were cleared or overwritten: its
    /// definition goes in the same undo step.
    fn cells_written(&self, app: &mut ExcelApp, edit: &CellEdit, _cx: &mut Cx) {
        if !logic::replaces_contents(edit.cause) {
            return;
        }
        let gone: Vec<u32> = app
            .wb
            .pivots_in(edit.sheet, edit.rect)
            .into_iter()
            .filter(|(_, whole)| *whole)
            .map(|(id, _)| id)
            .collect();
        if !gone.is_empty() {
            app.wb.snapshot();
            app.wb.sheets[edit.sheet].pivots.retain(|p| !gone.contains(&p.id));
        }
    }
}

/// An entry of a drop-down of the ribbon or of the context menu.
fn entry(
    label: impl Into<SharedString>,
    action: impl Fn(&mut ExcelApp, &mut Window, &mut Cx) + 'static,
) -> MenuEntry {
    MenuEntry::new(label, action)
}
