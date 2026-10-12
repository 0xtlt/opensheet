//! AutoFilter, sorting and Remove Duplicates.
//!
//! * The filter buttons: those of the sheet's filter (Data > Filter) and
//!   those of table headers are drawn and served here; their drop-down is
//!   in `dropdown.rs`, the Custom AutoFilter and Top 10 dialogs in
//!   `custom.rs`.
//! * Data > Filter, Clear and Reapply, the "n of m records found" message
//!   of the status bar and the blue row numbers of a filtered list.
//! * Sorting: the A to Z / Z to A buttons here, the Sort dialog in
//!   `sort.rs`.
//! * Remove Duplicates (`duplicates.rs`).
//! * The Filter and Sort submenus of the context menu of cells.

mod custom;
mod dropdown;
mod duplicates;
mod sort;

use super::{
    Command, ContextMenu, Cx, Feature, Intent, MenuSection, MenuTarget, PaintLayer, Painter, StatusItem, Zone, ZoneEvent,
    ZoneResponse,
};
use crate::app::{ExcelApp, MenuEntry};
use crate::grid::Draw;
use crate::i18n::{self, t};
use crate::shell::Alert;
use crate::theme::{self, c};
use gpui::{CursorStyle, MouseButton, Pixels, Point, Window, px};
use opensheet_engine::filter::{
    AutoFilter, ColorFilter, ColumnKind, Comparison, Condition, Criteria, DynamicFilter, FilterButton, FilterScope,
    ValueFilter,
};
use opensheet_engine::format::{self, FormatKind};
use opensheet_engine::ops::{SortError, SortLevel, SortOn, SortSpec};
use opensheet_engine::refs::col_to_letters;
use opensheet_engine::{Color, Locale, Rect, Value, Workbook};
use std::cell::Cell;
use std::collections::HashMap;
use std::ops::RangeInclusive;
use std::rc::Rc;

pub struct Filters;

commands! {
    /// What the user can ask of sorting and filtering.
    pub enum Cmd {
        Toggle = "filter", t("Filter", "Filtrer");
        Clear = "clear", t("Clear", "Effacer");
        Reapply = "refresh", t("Reapply", "Réappliquer");
        Advanced = "filter", t("Advanced", "Avancé");
        Sort = "sort-filter", t("Sort", "Trier");
        CustomSort = "sort-filter", t("Custom Sort…", "Tri personnalisé…");
        SortAscending = "sort-asc", t("Sort A to Z", "Trier de A à Z");
        SortDescending = "sort-desc", t("Sort Z to A", "Trier de Z à A");
        RemoveDuplicates = "delete-cells", t("Remove Duplicates", "Supprimer les doublons");
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
            // Something to clear or to apply again: a criterion, or the
            // sort remembered with the filter.
            Cmd::Clear | Cmd::Reapply => active_scope(app)
                .and_then(|scope| app.wb.auto_filter(app.sheet, scope))
                .is_some_and(|filter| filter.is_filtered() || filter.sort.is_some()),
            _ => true,
        }
    }

    fn checked(self, app: &ExcelApp) -> bool {
        self == Cmd::Toggle && buttons_shown(app)
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        if app.is_editing() && !app.commit_edit(None, window, cx) {
            return;
        }
        match self {
            Cmd::Toggle => toggle(app, cx),
            Cmd::Clear => clear(app, cx),
            Cmd::Reapply => reapply(app, cx),
            Cmd::Advanced => app.not_implemented(self.label(), cx),
            Cmd::Sort | Cmd::CustomSort => sort::open(app, window, cx),
            Cmd::SortAscending => sort_selection(app, SortOn::Values, false, window, cx),
            Cmd::SortDescending => sort_selection(app, SortOn::Values, true, window, cx),
            Cmd::RemoveDuplicates => duplicates::open(app, window, cx),
        }
    }
}

// -------------------------------------------------------------------- state

/// What this module remembers between events.
#[derive(Default)]
struct State {
    /// The drop-down last opened.
    menu: Option<OpenMenu>,
    /// The filter whose outcome the status bar reports.
    found: Option<Found>,
    /// The last sort made with the Sort dialog on plain cells, by sheet:
    /// the dialog opens on it again.
    last_sort: HashMap<usize, sort::Setup>,
}

struct OpenMenu {
    /// The button it hangs from.
    key: (FilterScope, u32),
    /// Where the press that closed it landed: a press on its own button
    /// must not open it again.
    dismissed: Rc<Cell<Option<Point<Pixels>>>>,
}

/// "n of m records found" stays on the status bar until the workbook is
/// recalculated for another reason, as in Excel.
#[derive(Clone, Copy, PartialEq)]
struct Found {
    sheet: usize,
    scope: FilterScope,
    generation: u64,
}

// ------------------------------------------------------------------ wording

/// Title of the message boxes.
pub(super) const APP_NAME: &str = "OpenSheet";

pub(super) fn records_found(found: usize, total: usize) -> String {
    t("{0} of {1} records found", "{0} enregistrement(s) trouvé(s) sur {1}")
        .replace("{0}", &found.to_string())
        .replace("{1}", &total.to_string())
}

/// The two sort entries, worded for what the column holds.
pub(super) fn sort_labels(kind: ColumnKind) -> (&'static str, &'static str) {
    match kind {
        ColumnKind::Text => (Cmd::SortAscending.label(), Cmd::SortDescending.label()),
        ColumnKind::Number => (
            t("Sort Smallest to Largest", "Trier du plus petit au plus grand"),
            t("Sort Largest to Smallest", "Trier du plus grand au plus petit"),
        ),
        ColumnKind::Date => (
            t("Sort Oldest to Newest", "Trier du plus ancien au plus récent"),
            t("Sort Newest to Oldest", "Trier du plus récent au plus ancien"),
        ),
    }
}

/// The name a column goes by in menus and dialogs: its header, or
/// "(Column B)" when the header cell is empty.
pub(super) fn column_title(header: &str, col: u32) -> String {
    if header.trim().is_empty() {
        t("(Column {})", "(Colonne {})").replace("{}", &col_to_letters(col))
    } else {
        header.to_string()
    }
}

pub(super) fn clear_filter_label(title: &str) -> String {
    t("Clear Filter From \"{}\"", "Effacer le filtre de «\u{a0}{}\u{a0}»").replace("{}", title)
}

/// The name of a month with a capital, as menus list them.
pub(super) fn month_name(locale: &Locale, month: u32) -> String {
    let name = locale.months[(month.clamp(1, 12) - 1) as usize];
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

pub(super) fn dynamic_label(filter: DynamicFilter, locale: &Locale) -> String {
    let label = match filter {
        DynamicFilter::AboveAverage => t("Above Average", "Au-dessus de la moyenne"),
        DynamicFilter::BelowAverage => t("Below Average", "En dessous de la moyenne"),
        DynamicFilter::Today => t("Today", "Aujourd'hui"),
        DynamicFilter::Yesterday => t("Yesterday", "Hier"),
        DynamicFilter::Tomorrow => t("Tomorrow", "Demain"),
        DynamicFilter::ThisWeek => t("This Week", "Cette semaine"),
        DynamicFilter::LastWeek => t("Last Week", "La semaine dernière"),
        DynamicFilter::NextWeek => t("Next Week", "La semaine prochaine"),
        DynamicFilter::ThisMonth => t("This Month", "Ce mois"),
        DynamicFilter::LastMonth => t("Last Month", "Le mois dernier"),
        DynamicFilter::NextMonth => t("Next Month", "Le mois prochain"),
        DynamicFilter::ThisQuarter => t("This Quarter", "Ce trimestre"),
        DynamicFilter::LastQuarter => t("Last Quarter", "Le trimestre dernier"),
        DynamicFilter::NextQuarter => t("Next Quarter", "Le trimestre prochain"),
        DynamicFilter::ThisYear => t("This Year", "Cette année"),
        DynamicFilter::LastYear => t("Last Year", "L'année dernière"),
        DynamicFilter::NextYear => t("Next Year", "L'année prochaine"),
        DynamicFilter::YearToDate => t("Year to Date", "Année à ce jour"),
        DynamicFilter::Quarter(q) => return t("Quarter {}", "Trimestre {}").replace("{}", &q.to_string()),
        DynamicFilter::Month(m) => return month_name(locale, m as u32),
    };
    label.to_string()
}

/// One condition of a custom filter in words, for the tip of a button:
/// `Greater than "5"`.
fn condition_summary(condition: &Condition, as_date: bool, locale: &Locale) -> String {
    let (comparison, _) = condition.comparison();
    let pattern = match (comparison, as_date) {
        (Comparison::Equals, _) => t("Equals \"{}\"", "Est égal à «\u{a0}{}\u{a0}»"),
        (Comparison::NotEquals, _) => t("Does not equal \"{}\"", "N'est pas égal à «\u{a0}{}\u{a0}»"),
        (Comparison::Greater, false) => t("Greater than \"{}\"", "Supérieur à «\u{a0}{}\u{a0}»"),
        (Comparison::GreaterOrEqual, false) => {
            t("Greater than or equal to \"{}\"", "Supérieur ou égal à «\u{a0}{}\u{a0}»")
        }
        (Comparison::Less, false) => t("Less than \"{}\"", "Inférieur à «\u{a0}{}\u{a0}»"),
        (Comparison::LessOrEqual, false) => t("Less than or equal to \"{}\"", "Inférieur ou égal à «\u{a0}{}\u{a0}»"),
        (Comparison::Greater, true) => t("After \"{}\"", "Postérieur au «\u{a0}{}\u{a0}»"),
        (Comparison::GreaterOrEqual, true) => t("After or equal to \"{}\"", "Postérieur ou égal au «\u{a0}{}\u{a0}»"),
        (Comparison::Less, true) => t("Before \"{}\"", "Antérieur au «\u{a0}{}\u{a0}»"),
        (Comparison::LessOrEqual, true) => t("Before or equal to \"{}\"", "Antérieur ou égal au «\u{a0}{}\u{a0}»"),
        (Comparison::BeginsWith, _) => t("Begins with \"{}\"", "Commence par «\u{a0}{}\u{a0}»"),
        (Comparison::NotBeginsWith, _) => t("Does not begin with \"{}\"", "Ne commence pas par «\u{a0}{}\u{a0}»"),
        (Comparison::EndsWith, _) => t("Ends with \"{}\"", "Se termine par «\u{a0}{}\u{a0}»"),
        (Comparison::NotEndsWith, _) => t("Does not end with \"{}\"", "Ne se termine pas par «\u{a0}{}\u{a0}»"),
        (Comparison::Contains, _) => t("Contains \"{}\"", "Contient «\u{a0}{}\u{a0}»"),
        (Comparison::NotContains, _) => t("Does not contain \"{}\"", "Ne contient pas «\u{a0}{}\u{a0}»"),
    };
    pattern.replace("{}", &condition.operand_text(locale, as_date))
}

/// What a column is filtered on, in words: the second line of the tip of
/// its button.
pub(super) fn criteria_summary(criteria: Option<&Criteria>, as_date: bool, locale: &Locale) -> String {
    let Some(criteria) = criteria else {
        return t("(Showing All)", "(Afficher tout)").to_string();
    };
    match criteria {
        Criteria::Values(filter) => {
            let mut names: Vec<String> = filter.values.iter().take(3).map(|v| format!("\"{v}\"")).collect();
            if filter.blanks && names.len() < 3 {
                names.push(blanks_label().to_string());
            }
            let listed = filter.values.len() + filter.blanks as usize + filter.dates.len();
            if names.is_empty() || listed > names.len() {
                // Too many to name, or only dates: say how many are kept.
                return t("{} items selected", "{} élément(s) sélectionné(s)").replace("{}", &listed.to_string());
            }
            t("Equals {}", "Est égal à {}").replace("{}", &names.join(t(" or ", " ou ")))
        }
        Criteria::Custom(custom) => {
            let first = condition_summary(&custom.first, as_date, locale);
            match &custom.second {
                None => first,
                Some(second) => {
                    let joint = if custom.and { t(" and ", " et ") } else { t(" or ", " ou ") };
                    format!("{first}{joint}{}", condition_summary(second, as_date, locale))
                }
            }
        }
        Criteria::Top10(top) => {
            let pattern = match (top.top, top.percent) {
                (true, false) => t("Top {} items", "{} premiers éléments"),
                (false, false) => t("Bottom {} items", "{} derniers éléments"),
                (true, true) => t("Top {} percent", "{}\u{a0}% les plus élevés"),
                (false, true) => t("Bottom {} percent", "{}\u{a0}% les moins élevés"),
            };
            pattern.replace("{}", &locale.number_to_text(top.value))
        }
        Criteria::Dynamic(dynamic) => dynamic_label(*dynamic, locale),
        Criteria::Color(ColorFilter { color: Some(_), font: false }) => {
            t("Equals a cell color", "Correspond à une couleur de cellule").to_string()
        }
        Criteria::Color(ColorFilter { color: None, font: false }) => t(
            "Equals a cell color (No Fill)",
            "Correspond à la couleur de cellule (Aucun remplissage)",
        )
        .to_string(),
        Criteria::Color(ColorFilter { color: Some(_), font: true }) => {
            t("Equals a font color", "Correspond à une couleur de police").to_string()
        }
        Criteria::Color(ColorFilter { color: None, font: true }) => t(
            "Equals a font color (Automatic)",
            "Correspond à la couleur de police (Automatique)",
        )
        .to_string(),
    }
}

pub(super) fn blanks_label() -> &'static str {
    t("(Blanks)", "(Vides)")
}

// ------------------------------------------------------------- what is where

/// Every filter of a sheet with its range: its own, then those of its
/// tables.
fn filters_of(wb: &Workbook, sheet: usize) -> impl Iterator<Item = (FilterScope, &AutoFilter)> {
    let sh = &wb.sheets[sheet];
    let own = sh.auto_filter.iter().map(|filter| (FilterScope::Sheet, filter));
    own.chain(
        sh.tables
            .iter()
            .filter_map(|table| Some((FilterScope::Table(table.id), table.auto_filter.as_ref()?))),
    )
}

/// The filter the buttons of the ribbon act on: the one whose range holds
/// the active cell, else the sheet's own.
fn active_scope(app: &ExcelApp) -> Option<FilterScope> {
    let (row, col) = app.active();
    app.wb.filter_scope_at(app.sheet, row, col).or_else(|| {
        app.wb.sheets[app.sheet]
            .auto_filter
            .as_ref()
            .map(|_| FilterScope::Sheet)
    })
}

/// Whether "Filter" shows as pressed: the table under the active cell has
/// its buttons, or the sheet has a filter.
fn buttons_shown(app: &ExcelApp) -> bool {
    let (row, col) = app.active();
    match app.wb.table_at(app.sheet, row, col) {
        Some(table) => table.auto_filter.is_some(),
        None => app.wb.sheets[app.sheet].auto_filter.is_some(),
    }
}

/// The fill and the font colour a cell shows, conditional formats
/// included: what "Filter by Color" and "Sort by Color" compare.
pub(super) fn shown_colors(wb: &Workbook, sheet: usize, row: u32, col: u32) -> (Option<Color>, Option<Color>) {
    let style = wb.style_at(sheet, row, col);
    let (mut fill, mut font) = (style.fill, style.font.color);
    if let Some(result) = opensheet_engine::condfmt::result_at(wb, sheet, row, col) {
        fill = result.patch.fill.unwrap_or(fill);
        font = result.patch.font_color.unwrap_or(font);
    }
    (fill, font)
}

/// How many cells of a column are looked at to tell what it holds.
const KIND_SAMPLE: usize = 5_000;

/// What some cells mostly hold: text, numbers or dates (the engine's rule
/// for the columns of a filter).
pub(super) fn cells_kind(wb: &Workbook, sheet: usize, cells: impl Iterator<Item = (u32, u32)>) -> ColumnKind {
    let sh = &wb.sheets[sheet];
    let (mut texts, mut numbers, mut dates) = (0usize, 0usize, 0usize);
    for (row, col) in cells.take(KIND_SAMPLE) {
        match sh.value_at(row, col) {
            Value::Empty => {}
            Value::Number(_) => {
                let code = &wb.style_at(sheet, row, col).num_fmt;
                if matches!(format::format_kind(code), FormatKind::Date | FormatKind::DateTime) {
                    dates += 1;
                } else {
                    numbers += 1;
                }
            }
            _ => texts += 1,
        }
    }
    if dates > 0 && dates >= numbers && dates >= texts {
        ColumnKind::Date
    } else if numbers > texts {
        ColumnKind::Number
    } else {
        ColumnKind::Text
    }
}

pub(super) fn column_kind(wb: &Workbook, sheet: usize, col: u32, rows: RangeInclusive<u32>) -> ColumnKind {
    cells_kind(wb, sheet, rows.map(|row| (row, col)))
}

/// Select a range without moving the active cell when it is inside: what
/// Excel shows while it asks how to sort a block of data.
pub(super) fn select_keeping_active(app: &mut ExcelApp, rect: Rect, cx: &mut Cx) {
    let active = app.active();
    if app.sel().ranges == [rect] {
        return;
    }
    app.select_rect(rect, cx);
    if rect.contains(active.0, active.1) {
        let selection = &mut app.views[app.sheet].sel;
        selection.active = active;
        selection.anchor = active;
        app.refresh(cx);
    }
}

// ------------------------------------------------------- filter on, off, clear

fn warn(app: &mut ExcelApp, message: &str, cx: &mut Cx) {
    app.alert(Alert::new(APP_NAME, message.to_string()).warning(), cx);
}

/// The selection when it is a single range; several ranges are refused
/// with Excel's message.
pub(super) fn single_selection(app: &mut ExcelApp, cx: &mut Cx) -> Option<Rect> {
    if app.sel().ranges.len() > 1 {
        warn(
            app,
            t(
                "This action won't work on multiple selections.",
                "Cette action ne peut pas être effectuée sur des sélections multiples.",
            ),
            cx,
        );
        return None;
    }
    Some(app.sel().current())
}

pub(super) fn cannot_apply(app: &mut ExcelApp, cx: &mut Cx) {
    warn(
        app,
        t(
            "This can't be applied to the selected range. Select a single cell in a range and try again.",
            "Nous ne pouvons pas appliquer cette action à la plage sélectionnée. Sélectionnez une seule cellule dans une plage, puis réessayez.",
        ),
        cx,
    );
}

/// Data > Filter.
fn toggle(app: &mut ExcelApp, cx: &mut Cx) {
    let Some(selection) = single_selection(app, cx) else {
        return;
    };
    if !app.may(Intent::ToggleAutoFilter, cx) {
        return;
    }
    match app.wb.toggle_auto_filter(app.sheet, selection) {
        Ok(_) => app.commit(Cmd::Toggle.label(), cx),
        Err(_) => {
            app.wb.rollback();
            cannot_apply(app, cx);
        }
    }
}

/// Data > Clear: every criterion of the filter and its sort arrow.
fn clear(app: &mut ExcelApp, cx: &mut Cx) {
    let Some(scope) = active_scope(app) else {
        return;
    };
    if app.may(Intent::UseAutoFilter, cx) && app.wb.clear_filters(app.sheet, scope) {
        app.commit(t("Clear Filter", "Effacer le filtre"), cx);
    }
}

/// Data > Reapply: sort and filter again with the cells as they are now.
fn reapply(app: &mut ExcelApp, cx: &mut Cx) {
    let Some(scope) = active_scope(app) else {
        return;
    };
    if !app.may(Intent::UseAutoFilter, cx) {
        return;
    }
    let sheet = app.sheet;
    let sorted = app
        .wb
        .auto_filter(sheet, scope)
        .and_then(|filter| Some((filter.range, filter.sort.clone()?)));
    if let Some((range, sort)) = sorted {
        if !app.may(Intent::Sort(range), cx) {
            return;
        }
        if let Err(err) = app.wb.sort_filter(sheet, scope, sort) {
            app.wb.rollback();
            sort_failed(app, err, cx);
            return;
        }
    }
    app.wb.reapply_filters(sheet);
    app.commit(Cmd::Reapply.label(), cx);
    note_found(app, sheet, scope);
}

/// Have the status bar report what the filter kept.
fn note_found(app: &mut ExcelApp, sheet: usize, scope: FilterScope) {
    let generation = app.wb.generation;
    app.ext_mut::<State>().found = Some(Found { sheet, scope, generation });
}

/// Give a column of a filter its criterion (`None` shows everything
/// again): what the drop-down and its dialogs end with.
pub(super) fn apply_criteria(
    app: &mut ExcelApp,
    sheet: usize,
    scope: FilterScope,
    col_id: u32,
    criteria: Option<Criteria>,
    cx: &mut Cx,
) {
    let current = app.wb.auto_filter(sheet, scope).and_then(|filter| filter.columns.get(&col_id));
    if current == criteria.as_ref() || !app.may(Intent::UseAutoFilter, cx) {
        return;
    }
    let label = match criteria {
        Some(_) => Cmd::Toggle.label(),
        None => t("Clear Filter", "Effacer le filtre"),
    };
    if app.wb.set_filter_criteria(sheet, scope, col_id, criteria) {
        app.commit(label, cx);
        note_found(app, sheet, scope);
    }
}

// ------------------------------------------------------------------ sorting

pub(super) fn sort_failed(app: &mut ExcelApp, error: SortError, cx: &mut Cx) {
    match error {
        SortError::MergedCells => warn(
            app,
            t(
                "To do this, all the merged cells need to be the same size.",
                "Pour ce faire, la taille des cellules fusionnées doit être identique.",
            ),
            cx,
        ),
    }
}

/// What a sort acts on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum SortTarget {
    /// A filtered list, or a table showing its buttons: it is sorted
    /// through its filter, which remembers the sort (the arrow on the
    /// button). `range` is the header row and the data rows.
    Filter { scope: FilterScope, range: Rect },
    /// Plain cells. `has_header`: the first row holds labels and stays in
    /// place.
    Range { rect: Rect, has_header: bool },
}

impl SortTarget {
    pub(super) fn rect(&self) -> Rect {
        match self {
            SortTarget::Filter { range, .. } => *range,
            SortTarget::Range { rect, .. } => *rect,
        }
    }
}

/// What a sort command does with the selection it finds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum SortPlan {
    /// A lone cell, or nothing but blanks around it.
    Nothing,
    Sort(SortTarget),
    /// One column of a larger block is selected: ask whether the rest of
    /// the block follows ("Sort Warning").
    Ask { selection: Rect, expanded: Rect },
    /// The selection is partly in a filtered range and partly outside.
    StraddlesFilter,
}

/// Decide what to sort for a selection and its active cell, as Excel does:
/// a single cell stands for the block of data around it, cells inside a
/// filtered list or a table for the whole list.
pub(super) fn sort_plan(wb: &Workbook, sheet: usize, selection: Rect, active: (u32, u32)) -> SortPlan {
    let sh = &wb.sheets[sheet];
    let selection = if selection.is_single() {
        selection
    } else {
        wb.clamp_to_used(sheet, selection)
    };
    let inside = |range: &Rect| range.intersection(&selection) == Some(selection);
    for (scope, filter) in filters_of(wb, sheet) {
        if filter.range.intersects(&selection) {
            return match inside(&filter.range) {
                true => SortPlan::Sort(SortTarget::Filter {
                    scope,
                    range: filter.range,
                }),
                false => SortPlan::StraddlesFilter,
            };
        }
    }
    // A table whose buttons are off is still sorted as a whole, without
    // its totals row.
    if let Some(table) = sh.tables.iter().find(|table| table.contains(active.0, active.1)) {
        let range = table.range;
        let rect = Rect::new(range.r0, range.c0, range.r1 - table.totals_row as u32, range.c1);
        if inside(&rect) {
            return SortPlan::Sort(SortTarget::Range {
                rect,
                has_header: table.header_row,
            });
        }
    }
    if selection.is_single() {
        let region = wb.current_region(sheet, selection.r0, selection.c0);
        if region.rows() < 2 {
            return SortPlan::Nothing;
        }
        return SortPlan::Sort(SortTarget::Range {
            rect: region,
            has_header: wb.guess_header_row(sheet, region),
        });
    }
    if selection.cols() == 1 {
        let expanded = block_around(wb, sheet, selection);
        if expanded != selection {
            return SortPlan::Ask { selection, expanded };
        }
    }
    SortPlan::Sort(SortTarget::Range {
        rect: selection,
        has_header: wb.guess_header_row(sheet, selection),
    })
}

/// The selection grown to the block of data it is part of ("Expand the
/// selection").
pub(super) fn block_around(wb: &Workbook, sheet: usize, selection: Rect) -> Rect {
    let sh = &wb.sheets[sheet];
    let filled = (selection.r0..=selection.r1)
        .flat_map(|row| (selection.c0..=selection.c1).map(move |col| (row, col)))
        .find(|(row, col)| !sh.is_blank(*row, *col));
    match filled {
        Some((row, col)) => wb.current_region(sheet, row, col).union(&selection),
        None => selection,
    }
}

/// The message for a sort that would cut through a filtered list.
pub(super) fn straddles_filter(app: &mut ExcelApp, cx: &mut Cx) {
    warn(
        app,
        t(
            "Excel can't sort because the sort range includes an Autofilter. Please select cells that are all outside or all within an Autofilter and sort again.",
            "Désolé... Excel ne peut pas effectuer le tri, car la plage de tri comprend un filtre automatique. Veuillez sélectionner des cellules qui sont toutes situées en dehors d'un filtre automatique ou dans un filtre automatique, puis réessayez.",
        ),
        cx,
    );
}

/// Sort what the selection stands for on the column of the active cell:
/// the A to Z / Z to A buttons, and "Put Selected Cell Color On Top".
pub(super) fn sort_selection(app: &mut ExcelApp, on: SortOn, descending: bool, window: &mut Window, cx: &mut Cx) {
    let Some(selection) = single_selection(app, cx) else {
        return;
    };
    let (_, col) = app.active();
    match sort_plan(&app.wb, app.sheet, selection, app.active()) {
        SortPlan::Nothing => {}
        SortPlan::StraddlesFilter => straddles_filter(app, cx),
        SortPlan::Sort(target) => sort_target(app, target, col, on, descending, cx),
        SortPlan::Ask { selection, expanded } => sort::ask_to_expand(
            app,
            sort::Asking::Sort,
            selection,
            expanded,
            move |app, rect, _, cx| {
                let has_header = app.wb.guess_header_row(app.sheet, rect);
                sort_target(app, SortTarget::Range { rect, has_header }, col, on, descending, cx);
            },
            window,
            cx,
        ),
    }
}

/// Sort a target on one of its columns (`col` is a sheet column).
pub(super) fn sort_target(app: &mut ExcelApp, target: SortTarget, col: u32, on: SortOn, descending: bool, cx: &mut Cx) {
    let rect = target.rect();
    if !app.may(Intent::Sort(rect), cx) {
        return;
    }
    let sheet = app.sheet;
    let key = col.clamp(rect.c0, rect.c1) - rect.c0;
    let outcome = match target {
        SortTarget::Filter { scope, .. } => app.wb.sort_by_filter(sheet, scope, key, on, descending),
        SortTarget::Range { rect, has_header } => {
            let level = SortLevel {
                key,
                on,
                descending,
                custom_list: None,
            };
            let spec = SortSpec {
                levels: vec![level],
                has_header,
                ..Default::default()
            };
            app.wb.sort_range(sheet, rect, &spec)
        }
    };
    match outcome {
        Ok(()) => app.commit(Cmd::Sort.label(), cx),
        Err(err) => {
            app.wb.rollback();
            sort_failed(app, err, cx);
        }
    }
}

// --------------------------------------------------- context menu of a cell

/// What "Filter by Selected Cell's ..." filters on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum By {
    Value,
    CellColor,
    FontColor,
}

/// The criterion that keeps the rows like a given cell.
pub(super) fn criterion_like(wb: &Workbook, sheet: usize, row: u32, col: u32, by: By) -> Criteria {
    let (fill, font) = shown_colors(wb, sheet, row, col);
    match by {
        By::Value => {
            let text = wb.display_text(sheet, row, col).text;
            Criteria::Values(ValueFilter {
                blanks: text.is_empty(),
                values: if text.is_empty() { Vec::new() } else { vec![text] },
                dates: Vec::new(),
            })
        }
        By::CellColor => Criteria::Color(ColorFilter { color: fill, font: false }),
        By::FontColor => Criteria::Color(ColorFilter { color: font, font: true }),
    }
}

/// "Filter by Selected Cell's Value / Color / Font Color": the list around
/// the cell gets its filter buttons if it has none yet.
fn filter_by_cell(app: &mut ExcelApp, by: By, cx: &mut Cx) {
    let (sheet, (row, col)) = (app.sheet, app.active());
    if !app.may(Intent::UseAutoFilter, cx) {
        return;
    }
    if app.wb.filter_scope_at(sheet, row, col).is_none() {
        if !app.may(Intent::ToggleAutoFilter, cx) {
            return;
        }
        if app.wb.toggle_auto_filter(sheet, Rect::cell(row, col)).is_err() {
            app.wb.rollback();
            cannot_apply(app, cx);
            return;
        }
    }
    let located = app
        .wb
        .filter_scope_at(sheet, row, col)
        .and_then(|scope| Some((scope, app.wb.auto_filter(sheet, scope)?.range.c0)));
    let Some((scope, first_col)) = located else {
        // The buttons could not be put around this cell after all.
        app.wb.rollback();
        cannot_apply(app, cx);
        return;
    };
    let criteria = criterion_like(&app.wb, sheet, row, col, by);
    app.wb.set_filter_criteria(sheet, scope, col - first_col, Some(criteria));
    app.commit(Cmd::Toggle.label(), cx);
    note_found(app, sheet, scope);
}

/// The filtered column a cell belongs to: its filter, its offset in it,
/// its title, and whether it has a criterion.
pub(super) fn filter_column_at(wb: &Workbook, sheet: usize, row: u32, col: u32) -> Option<(FilterScope, u32, String, bool)> {
    let scope = wb.filter_scope_at(sheet, row, col)?;
    let filter = wb.auto_filter(sheet, scope)?;
    let col_id = col - filter.range.c0;
    let header = wb.display_text(sheet, filter.range.r0, col).text;
    Some((scope, col_id, column_title(&header, col), filter.columns.contains_key(&col_id)))
}

/// The Filter and Sort submenus of the context menu of a cell. `reapply`
/// and `custom_sort` are the entries of those two commands.
fn cell_menu(wb: &Workbook, sheet: usize, row: u32, col: u32, reapply: MenuEntry, custom_sort: MenuEntry) -> [MenuEntry; 2] {
    let column = filter_column_at(wb, sheet, row, col);
    let clear = match &column {
        Some((scope, col_id, title, filtered)) => {
            let (scope, col_id) = (*scope, *col_id);
            MenuEntry::new(clear_filter_label(title), move |app, _, cx| {
                let sheet = app.sheet;
                apply_criteria(app, sheet, scope, col_id, None, cx);
            })
            .disabled(!filtered)
        }
        None => MenuEntry::new(t("Clear Filter", "Effacer le filtre"), |_, _, _| {}).disabled(true),
    };
    let like = |label: &'static str, by: By| MenuEntry::new(label, move |app, _, cx| filter_by_cell(app, by, cx));
    let filter = vec![
        clear.icon("filters-clear"),
        reapply,
        like(
            t("Filter by Selected Cell's Value", "Filtrer par la valeur de la cellule sélectionnée"),
            By::Value,
        ),
        like(
            t("Filter by Selected Cell's Color", "Filtrer par la couleur de la cellule sélectionnée"),
            By::CellColor,
        ),
        like(
            t(
                "Filter by Selected Cell's Font Color",
                "Filtrer par la couleur de police de la cellule sélectionnée",
            ),
            By::FontColor,
        ),
    ];
    // The wording of the sort entries follows what the cell holds.
    let (asc, desc) = sort_labels(column_kind(wb, sheet, col, row..=row));
    let on_top = |label: &'static str, font: bool| {
        MenuEntry::new(label, move |app, window, cx| {
            let (row, col) = app.active();
            let (fill, color) = shown_colors(&app.wb, app.sheet, row, col);
            let on = if font { SortOn::FontColor(color) } else { SortOn::CellColor(fill) };
            sort_selection(app, on, false, window, cx);
        })
    };
    let sort = vec![
        MenuEntry::new(asc, |app, window, cx| sort_selection(app, SortOn::Values, false, window, cx)).icon("sort-asc"),
        MenuEntry::new(desc, |app, window, cx| sort_selection(app, SortOn::Values, true, window, cx)).icon("sort-desc"),
        on_top(
            t(
                "Put Selected Cell Color On Top",
                "Placer la couleur de cellule sélectionnée sur le dessus",
            ),
            false,
        ),
        on_top(
            t(
                "Put Selected Font Color On Top",
                "Placer la couleur de police sélectionnée sur le dessus",
            ),
            true,
        ),
        custom_sort,
    ];
    [
        MenuEntry::submenu(Cmd::Toggle.label(), filter).icon("filter"),
        MenuEntry::submenu(Cmd::Sort.label(), sort).icon("sort-asc"),
    ]
}

// ------------------------------------------------------------------ painting

/// A filter button on the grid.
struct ButtonZone(FilterButton);

/// The filter buttons among the cells in view.
fn buttons_in(wb: &Workbook, sheet: usize, visible: Rect) -> Vec<FilterButton> {
    let mut out = Vec::new();
    for (_, filter) in filters_of(wb, sheet) {
        let range = filter.range;
        if range.r0 < visible.r0 || range.r0 > visible.r1 {
            continue;
        }
        for col in range.c0.max(visible.c0)..=range.c1.min(visible.c1) {
            out.extend(wb.filter_button_at(sheet, range.r0, col));
        }
    }
    out
}

/// Where the button sits in its cell: a square in the bottom right corner,
/// smaller in a low row or a narrow column, absent from a hidden one.
pub(super) fn button_box(cell: (f32, f32, f32, f32), zoom: f32) -> Option<(f32, f32, f32, f32)> {
    let (x0, y0, x1, y1) = cell;
    let room = (x1 - x0).min(y1 - y0) - 3.0;
    if room < 5.0 {
        return None;
    }
    let side = (16.0 * zoom).round().min(room);
    Some((x1 - 2.0 - side, y1 - 2.0 - side, x1 - 2.0, y1 - 2.0))
}

const BUTTON_BORDER: u32 = 0xA6A6A6;
const BUTTON_GLYPH: u32 = 0x444444;

/// Draw a filter button with one of Excel's looks: a plain arrow, a funnel
/// when the column is filtered, an arrow giving the direction when the
/// list is sorted on the column.
fn draw_button(p: &mut Painter, bounds: (f32, f32, f32, f32), button: &FilterButton) {
    let (x0, y0, x1, y1) = bounds;
    p.rounded(x0, y0, x1, y1, 2.0, c(0xFFFFFF), Some(c(BUTTON_BORDER)));
    // The glyphs are designed on a 16 x 16 square.
    let u = (x1 - x0) / 16.0;
    let at = |x: f32, y: f32| (x0 + x * u, y0 + y * u);
    let glyph = c(BUTTON_GLYPH);
    let arrow = |p: &mut Painter, x: f32, descending: bool| {
        p.polyline(&[at(x, 3.5), at(x, 12.5)], (1.2 * u).max(1.0), glyph);
        let (tip, wings) = if descending { (13.0, 10.0) } else { (3.0, 6.0) };
        p.polygon(&[at(x - 2.6, wings), at(x + 2.6, wings), at(x, tip)], glyph);
    };
    let funnel = |p: &mut Painter| {
        p.polygon(
            &[
                at(2.5, 3.5),
                at(10.5, 3.5),
                at(7.5, 7.5),
                at(7.5, 12.5),
                at(5.5, 11.0),
                at(5.5, 7.5),
            ],
            glyph,
        );
    };
    match (button.filtered, button.sort) {
        (false, None) => p.polygon(&[at(4.5, 6.5), at(11.5, 6.5), at(8.0, 10.5)], glyph),
        (false, Some(descending)) => {
            arrow(p, 4.5, descending);
            p.polygon(&[at(8.5, 7.0), at(14.0, 7.0), at(11.25, 10.5)], glyph);
        }
        (true, None) => {
            funnel(p);
            p.polygon(&[at(9.5, 10.0), at(14.0, 10.0), at(11.75, 12.8)], glyph);
        }
        (true, Some(descending)) => {
            funnel(p);
            arrow(p, 12.0, descending);
        }
    }
}

/// How many cells of a column the tip of its button looks at.
const TIP_SAMPLE: usize = 100;

/// The tip of a button: the column and what it is filtered on.
fn button_tip(wb: &Workbook, sheet: usize, button: &FilterButton) -> String {
    let header = wb.display_text(sheet, button.row, button.col).text;
    let criteria = wb
        .auto_filter(sheet, button.scope)
        .and_then(|filter| filter.columns.get(&button.col_id));
    // Comparisons read "after" rather than "greater than" on dates; a few
    // cells are enough to tell (this runs at every repaint).
    let as_date = criteria.is_some_and(|criteria| matches!(criteria, Criteria::Custom(_)))
        && filter_data_rows(wb, sheet, button.scope).is_some_and(|rows| {
            cells_kind(wb, sheet, rows.take(TIP_SAMPLE).map(|row| (row, button.col))) == ColumnKind::Date
        });
    let title = column_title(&header, button.col);
    format!(
        "{}\n{}",
        i18n::labelled(&title, "").trim_end(),
        criteria_summary(criteria, as_date, wb.locale)
    )
}

fn filter_data_rows(wb: &Workbook, sheet: usize, scope: FilterScope) -> Option<RangeInclusive<u32>> {
    let data = wb.auto_filter(sheet, scope)?.data_range()?;
    Some(data.r0..=data.r1)
}

fn paint_buttons(p: &mut Painter) {
    let app = p.app;
    let sh = &app.wb.sheets[app.sheet];
    if sh.auto_filter.is_none() && sh.tables.iter().all(|table| table.auto_filter.is_none()) {
        return;
    }
    for button in buttons_in(&app.wb, app.sheet, p.visible) {
        let Some(bounds) = button_box(p.cell_box(Rect::cell(button.row, button.col)), p.zoom()) else {
            continue;
        };
        draw_button(p, bounds, &button);
        let tip = button_tip(&app.wb, app.sheet, &button);
        p.zone(bounds, ButtonZone(button)).cursor(CursorStyle::Arrow).tooltip(tip);
    }
}

/// Row numbers of a filtered list, as Excel shows them.
const FILTERED_ROW_NUMBER: u32 = 0x1F5FBF;

/// Draw the numbers of the rows of filtered lists in blue, over the row
/// headers.
fn paint_row_numbers(p: &mut Painter) {
    let app = p.app;
    let layout = p.layout;
    let hw = layout.header_w;
    if hw <= 0.0 {
        return;
    }
    let lists: Vec<Rect> = filters_of(&app.wb, app.sheet)
        .filter(|(_, filter)| filter.is_filtered())
        .filter_map(|(_, filter)| filter.data_range())
        .collect();
    if lists.is_empty() {
        return;
    }
    let ranges = &app.sel().ranges;
    // The header metrics of the grid.
    let size = (12.0 * layout.zoom).clamp(8.0, 20.0);
    let line_h = (size * 1.3).round();
    let frozen_bottom = layout.header_h + layout.frozen_h;
    let panes = [
        (0..layout.frozen_rows, layout.header_h, frozen_bottom),
        (layout.scrolled_rows(), frozen_bottom, layout.height()),
    ];
    for (rows, top, bottom) in panes {
        for row in rows {
            if !lists.iter().any(|list| list.r0 <= row && row <= list.r1) {
                continue;
            }
            let (y, height) = (layout.row_y(row), layout.row_h(row));
            // A row selected as a whole keeps its white number.
            let in_selection = ranges.iter().any(|r| r.r0 <= row && row <= r.r1);
            if height < line_h * 0.7 || ranges.iter().any(|r| r.is_whole_rows() && r.r0 <= row && row <= r.r1) {
                continue;
            }
            let (y0, y1) = (y.max(top), (y + height - 1.0).min(bottom));
            if y1 <= y0 {
                continue;
            }
            let (background, edge) = if in_selection { (theme::HEADER_SEL_BG, 2.0) } else { (theme::HEADER_BG, 1.0) };
            p.rect(0.0, y0, hw - edge, y1, c(background));
            let line = p.shape(&(row + 1).to_string(), size, c(FILTERED_ROW_NUMBER), in_selection);
            let width = f32::from(line.width);
            p.push(Draw::Text {
                line,
                origin: layout.to_window(((hw - width) / 2.0).round(), y + ((height - line_h) / 2.0).round()),
                line_height: px(line_h),
                clip: layout.bounds_px(0.0, y0, hw, y1),
            });
        }
    }
}

// ------------------------------------------------------------------ the hooks

impl Feature for Filters {
    fn name(&self) -> &'static str {
        "filters"
    }

    fn paint(&self, p: &mut Painter) {
        match p.layer {
            PaintLayer::Over => paint_buttons(p),
            PaintLayer::Top => paint_row_numbers(p),
            PaintLayer::Under => {}
        }
    }

    fn zone_mouse_down(&self, app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) -> ZoneResponse {
        let Some(ButtonZone(button)) = zone.payload::<ButtonZone>() else {
            return ZoneResponse::Ignored;
        };
        if ev.button != MouseButton::Left {
            return ZoneResponse::Ignored;
        }
        // The press that just closed the drop-down of this very button
        // leaves it closed.
        let key = (button.scope, button.col_id);
        let closed_it = app
            .ext::<State>()
            .and_then(|state| state.menu.as_ref())
            .is_some_and(|menu| menu.key == key && menu.dismissed.get() == Some(ev.position));
        if closed_it {
            app.ext_mut::<State>().menu = None;
        } else {
            dropdown::open(app, *button, window, cx);
        }
        ZoneResponse::Handled
    }

    fn cell_drop_down(&self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> bool {
        let (row, col) = app.active();
        match app.wb.filter_button_at(app.sheet, row, col) {
            Some(button) => {
                dropdown::open(app, button, window, cx);
                true
            }
            None => false,
        }
    }

    fn context_menu(&self, app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
        if let MenuTarget::Cell(row, col) = *target {
            let (reapply, custom_sort) = (MenuEntry::command(Cmd::Reapply, app), MenuEntry::command(Cmd::CustomSort, app));
            for entry in cell_menu(&app.wb, app.sheet, row, col, reapply, custom_sort) {
                menu.add(MenuSection::Data, entry);
            }
        }
    }

    fn status_items(&self, app: &ExcelApp) -> Vec<StatusItem> {
        if !filters_of(&app.wb, app.sheet).any(|(_, filter)| filter.is_filtered()) {
            return Vec::new();
        }
        let found = app
            .ext::<State>()
            .and_then(|state| state.found)
            .filter(|found| found.sheet == app.sheet && found.generation == app.wb.generation)
            .and_then(|found| app.wb.filter_record_count(app.sheet, found.scope));
        let text = match found {
            Some((shown, total)) => records_found(shown, total),
            None => t("Filter Mode", "Mode Filtre").to_string(),
        };
        vec![StatusItem::new(text)]
    }
}

#[cfg(test)]
mod tests;
