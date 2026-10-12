//! Defined names: the Name Manager, defining and applying names, creating
//! them from the selection, using them in a formula, and the Name Box.
//!
//! The dialogs are in `names/`; this file holds the commands and what the
//! dialogs and the Name Box work out from the workbook, written as plain
//! functions so that it can be tested without a window.

mod dialogs;
mod manager;

use super::{ChangeCause, Command, ContextMenu, Cx, Feature, Intent, MenuSection, MenuTarget};
use crate::app::{EditMode, EditSource, ExcelApp, MenuEntry};
use crate::grid::Selection;
use crate::i18n::t;
use crate::shell::{Alert, RefStyle};
use gpui::Window;
use opensheet_engine::ast::{RefExpr, same_name};
use opensheet_engine::eval::Calc;
use opensheet_engine::format::format_value;
use opensheet_engine::names::{NameError, NameScope, is_valid_name};
use opensheet_engine::ops::PasteMode;
use opensheet_engine::protect::Action;
use opensheet_engine::refs::cell_name;
use opensheet_engine::{Content, ErrorKind, Expr, Locale, Rect, Reference, Value, Workbook};
use std::cmp::Ordering;
use std::sync::Arc;

pub struct Names;

commands! {
    /// What the user can ask of defined names.
    pub enum Cmd {
        Manager = "tag", t("Name Manager", "Gestionnaire de noms");
        DefineMenu = "tag", t("Define Name", "Définir un nom");
        Define = "tag", t("Define Name…", "Définir un nom…");
        Apply = "", t("Apply Names…", "Appliquer les noms…");
        UseInFormula = "function", t("Use in Formula", "Dans une formule");
        PasteNames = "", t("Paste Names…", "Coller des noms…");
        CreateFromSelection = "grid", t("Create from Selection", "Depuis sélection");
        NameBoxList = "chevron-down", t("Name Box", "Zone Nom");
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
        // Names go into the formula being typed; everything else works on
        // the sheet, once the entry in progress is stored.
        let on_formula = matches!(self, Cmd::UseInFormula | Cmd::PasteNames);
        if !on_formula && app.is_editing() && !app.commit_edit(None, window, cx) {
            return;
        }
        match self {
            Cmd::Manager => manager::open(app, window, cx),
            Cmd::DefineMenu => {
                let entries = vec![MenuEntry::command(Cmd::Define, app), MenuEntry::command(Cmd::Apply, app)];
                app.open_command_menu(entries, 200.0, window, cx);
            }
            Cmd::Define => dialogs::open_new_name(app, window, cx),
            Cmd::Apply => dialogs::open_apply_names(app, window, cx),
            Cmd::UseInFormula => {
                let mut entries: Vec<MenuEntry> = formula_names(&app.wb, app.sheet)
                    .into_iter()
                    .map(|name| {
                        MenuEntry::new(name.clone(), move |a, window, cx| insert_in_formula(a, &name, window, cx))
                    })
                    .collect();
                if !entries.is_empty() {
                    entries.push(MenuEntry::separator());
                }
                entries.push(MenuEntry::command(Cmd::PasteNames, app));
                app.open_command_menu(entries, 220.0, window, cx);
            }
            Cmd::PasteNames => dialogs::open_paste_names(app, window, cx),
            Cmd::CreateFromSelection => dialogs::open_create_names(app, window, cx),
            Cmd::NameBoxList => {
                let entries: Vec<MenuEntry> = visible_names(&app.wb, app.sheet)
                    .into_iter()
                    .map(|name| {
                        MenuEntry::new(name.clone(), move |a, window, cx| {
                            name_box_entered(a, &name, cx);
                            window.focus(&a.grid_focus);
                        })
                    })
                    .collect();
                if !entries.is_empty() {
                    app.open_menu("name-box", entries, 200.0, window, cx);
                }
            }
        }
    }

    fn enabled(self, app: &ExcelApp) -> bool {
        match self {
            // Excel greys these out on a protected sheet.
            Cmd::DefineMenu | Cmd::Define | Cmd::Apply | Cmd::CreateFromSelection => locked(app).is_none(),
            Cmd::UseInFormula | Cmd::PasteNames => has_formula_names(&app.wb, app.sheet),
            Cmd::Manager | Cmd::NameBoxList => true,
        }
    }
}

impl Feature for Names {
    fn name(&self) -> &'static str {
        "names"
    }

    fn context_menu(&self, app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
        if matches!(target, MenuTarget::Cell(..)) {
            menu.add(MenuSection::Extra, MenuEntry::command(Cmd::Define, app));
        }
    }
}

// ------------------------------------------------------------- protection

/// Why names cannot be defined, changed or deleted right now: Excel
/// refuses it while the current sheet is protected (the commands are greyed
/// out, the Name Box answers with the message of protected sheets). The
/// protection of the workbook structure alone leaves names free, as in
/// Excel.
pub(crate) fn locked(app: &ExcelApp) -> Option<String> {
    app.wb
        .permits(&Action::ChangeSheet { sheet: app.sheet })
        .err()
        .map(|denied| denied.message(app.wb.locale))
}

// ----------------------------------------------------------------- messages

/// What Excel answers to a name it refuses.
pub(crate) fn name_error_message(error: NameError) -> &'static str {
    match error {
        NameError::Duplicate => t(
            "This name already exists. Names should be unique.",
            "Ce nom existe déjà. Les noms doivent être uniques.",
        ),
        NameError::Invalid | NameError::NotFound => t(
            "This name isn't valid. A name starts with a letter or an underscore (_), holds no space or other character that isn't allowed, and is neither a cell reference nor a name the workbook already uses.",
            "Ce nom n'est pas valide. Un nom commence par une lettre ou un trait de soulignement (_), ne contient ni espace ni caractère non autorisé, et n'est ni une référence de cellule ni un nom déjà utilisé dans le classeur.",
        ),
    }
}

pub(crate) fn invalid_reference() -> &'static str {
    t("Reference isn't valid.", "La référence n’est pas valide.")
}

// ---------------------------------------------------------------- the list

/// What a line of the Name Manager stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RowKind {
    Name(NameScope),
    /// A table: its sheet and its id.
    Table(usize, u32),
}

/// A line of the Name Manager.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NameRow {
    pub kind: RowKind,
    pub name: String,
    pub value: String,
    pub refers_to: String,
    pub scope: String,
    pub comment: String,
    /// The name stands for an error (a deleted range, a division by zero).
    pub error: bool,
}

impl NameRow {
    pub fn is_table(&self) -> bool {
        matches!(self.kind, RowKind::Table(..))
    }

    fn column(&self, column: Column) -> &str {
        match column {
            Column::Name => &self.name,
            Column::Value => &self.value,
            Column::RefersTo => &self.refers_to,
            Column::Scope => &self.scope,
            Column::Comment => &self.comment,
        }
    }
}

/// The columns of the Name Manager.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Column {
    Name,
    Value,
    RefersTo,
    Scope,
    Comment,
}

impl Column {
    pub const ALL: [Column; 5] = [Column::Name, Column::Value, Column::RefersTo, Column::Scope, Column::Comment];

    pub fn title(self) -> &'static str {
        match self {
            Column::Name => t("Name", "Nom"),
            Column::Value => t("Value", "Valeur"),
            Column::RefersTo => t("Refers To", "Fait référence à"),
            Column::Scope => t("Scope", "Étendue"),
            Column::Comment => t("Comment", "Commentaire"),
        }
    }
}

/// The choices of the Filter menu of the Name Manager: within each pair
/// the two are exclusive, `None` shows both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) struct NameFilter {
    /// `Some(true)`: names scoped to a worksheet; `Some(false)`: to the
    /// workbook.
    pub sheet_scoped: Option<bool>,
    /// `Some(true)`: names with errors.
    pub with_errors: Option<bool>,
    /// `Some(true)`: table names; `Some(false)`: defined names.
    pub tables: Option<bool>,
}

impl NameFilter {
    pub fn is_empty(&self) -> bool {
        *self == NameFilter::default()
    }

    fn keeps(&self, row: &NameRow) -> bool {
        let sheet_scoped = matches!(row.kind, RowKind::Name(NameScope::Sheet(_)));
        self.sheet_scoped.is_none_or(|wanted| wanted == sheet_scoped)
            && self.with_errors.is_none_or(|wanted| wanted == row.error)
            && self.tables.is_none_or(|wanted| wanted == row.is_table())
    }
}

/// The text of a list of values the way Excel writes an array constant,
/// cut after a few of them.
fn array_text(values: &mut dyn Iterator<Item = (bool, String)>, locale: &Locale) -> String {
    const SHOWN: usize = 24;
    let mut out = String::from("{");
    let mut count = 0;
    for (new_row, text) in values {
        if count > 0 {
            out.push(if new_row { locale.array_row_sep } else { locale.array_col_sep });
        }
        if count == SHOWN {
            out.push('…');
            break;
        }
        out.push_str(&text);
        count += 1;
    }
    out.push('}');
    out
}

/// A value as the Name Manager shows it: what the cell would display, and
/// every item between quotes when there are several.
pub(crate) fn listed_value(value: &Value, locale: &Locale) -> String {
    let shown = |v: &Value| format_value(v, "General", locale).text;
    match value {
        Value::Array(array) => {
            let mut items = array
                .data
                .iter()
                .enumerate()
                .map(|(i, v)| (i > 0 && i % array.cols.max(1) == 0, format!("\"{}\"", shown(v))));
            array_text(&mut items, locale)
        }
        scalar => shown(scalar),
    }
}

/// A value as a formula would write it (`"text"`, `{1;2;3}`): what the
/// Function Arguments dialog shows beside each argument.
pub(crate) fn literal_value(value: &Value, locale: &Locale) -> String {
    let one = |v: &Value| match v {
        Value::Text(text) => format!("\"{}\"", text.replace('"', "\"\"")),
        Value::Empty => "0".to_string(),
        other => format_value(other, "General", locale).text,
    };
    match value {
        Value::Array(array) => {
            let mut items = array
                .data
                .iter()
                .enumerate()
                .map(|(i, v)| (i > 0 && i % array.cols.max(1) == 0, one(v)));
            array_text(&mut items, locale)
        }
        Value::Empty => String::new(),
        scalar => one(scalar),
    }
}

/// The name of a scope as the Scope column and the Scope list show it.
pub(crate) fn scope_label(wb: &Workbook, scope: NameScope) -> String {
    match scope {
        NameScope::Workbook => t("Workbook", "Classeur").to_string(),
        NameScope::Sheet(index) => wb.sheets.get(index).map(|s| s.name.clone()).unwrap_or_default(),
    }
}

/// Every name and every table of the workbook, as the Name Manager lists
/// them while the cell (`sheet`, `row`, `col`) is active: by name.
pub(crate) fn name_rows(wb: &Workbook, sheet: usize, row: u32, col: u32) -> Vec<NameRow> {
    let ref_error = wb.locale.error_name(ErrorKind::Ref);
    let mut rows: Vec<NameRow> = wb
        .all_names()
        .into_iter()
        .map(|(scope, def)| {
            // A sheet's own name is read from that sheet.
            let from = match scope {
                NameScope::Sheet(index) => index,
                NameScope::Workbook => sheet,
            };
            let value = Calc::eval_once(wb, from, row, col, &def.expr_at(row, col));
            let refers_to = def.refers_to(wb.locale, row, col);
            NameRow {
                kind: RowKind::Name(scope),
                name: def.name.clone(),
                error: value.is_error() || refers_to.contains(ref_error),
                value: listed_value(&value, wb.locale),
                refers_to,
                scope: scope_label(wb, scope),
                comment: def.comment.clone(),
            }
        })
        .collect();
    for (index, sh) in wb.sheets.iter().enumerate() {
        for table in &sh.tables {
            let data = table.data_range().unwrap_or(table.range);
            let value = Calc::eval_once(wb, index, row, col, &absolute_ref(&sh.name, data));
            rows.push(NameRow {
                kind: RowKind::Table(index, table.id),
                name: table.name.clone(),
                value: listed_value(&value, wb.locale),
                refers_to: RefStyle::formula().format(data, &sh.name),
                scope: scope_label(wb, NameScope::Workbook),
                comment: String::new(),
                error: false,
            });
        }
    }
    sort_rows(&mut rows, Column::Name, true);
    rows
}

/// Keep the rows a filter shows.
pub(crate) fn filter_rows(rows: Vec<NameRow>, filter: &NameFilter) -> Vec<NameRow> {
    rows.into_iter().filter(|row| filter.keeps(row)).collect()
}

/// Order the rows by a column, without regard to case; names break ties.
pub(crate) fn sort_rows(rows: &mut [NameRow], column: Column, ascending: bool) {
    let key = |text: &str| text.to_lowercase();
    rows.sort_by(|a, b| {
        let order = key(a.column(column)).cmp(&key(b.column(column)));
        let order = if ascending { order } else { order.reverse() };
        order.then_with(|| key(&a.name).cmp(&key(&b.name)))
    });
}

// ---------------------------------------------------- names seen from a sheet

fn by_name(a: &String, b: &String) -> Ordering {
    a.to_lowercase().cmp(&b.to_lowercase())
}

/// The defined names a formula of `sheet` can use as they are: the
/// sheet's own, then the workbook's that they do not hide.
pub(crate) fn formula_names(wb: &Workbook, sheet: usize) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    if let Some(sh) = wb.sheets.get(sheet) {
        names.extend(sh.names.values().map(|def| def.name.clone()));
    }
    let own = names.len();
    for def in wb.names.values() {
        if !names[..own].iter().any(|name| same_name(name, &def.name)) {
            names.push(def.name.clone());
        }
    }
    names.sort_by(by_name);
    names
}

fn has_formula_names(wb: &Workbook, sheet: usize) -> bool {
    !wb.names.is_empty() || wb.sheets.get(sheet).is_some_and(|sh| !sh.names.is_empty())
}

/// What the drop-down of the Name Box and the Go To dialog list: the names
/// of `formula_names` that stand for cells, and the tables.
pub(crate) fn visible_names(wb: &Workbook, sheet: usize) -> Vec<String> {
    let mut names: Vec<String> = formula_names(wb, sheet)
        .into_iter()
        .filter(|name| wb.reference_areas(sheet, 0, 0, name).is_some())
        .collect();
    names.extend(wb.sheets.iter().flat_map(|sh| sh.tables.iter().map(|table| table.name.clone())));
    names.sort_by(by_name);
    names
}

/// The name of the selected cells, when they are exactly what a name (or
/// the body of a table) stands for: what the Name Box shows instead of
/// the address of the active cell.
pub(crate) fn selection_name(wb: &Workbook, sheet: usize, selection: &Selection) -> Option<String> {
    let [range] = selection.ranges.as_slice() else {
        return None;
    };
    let wanted = Reference::new(sheet, *range);
    let (row, col) = selection.active;
    let sh = wb.sheets.get(sheet)?;
    // Without names there is nothing to look for (the common case, at
    // every move of the selection).
    if wb.names.is_empty() && sh.names.is_empty() && wb.sheets.iter().all(|s| s.tables.is_empty()) {
        return None;
    }
    let named = sh.names.values().chain(wb.names.values()).find(|def| {
        wb.lookup_name(sheet, &def.name)
            .is_some_and(|(_, seen)| std::ptr::eq(seen, *def))
            && wb.name_reference(sheet, row, col, &def.name) == Some(wanted)
    });
    if let Some(def) = named {
        return Some(def.name.clone());
    }
    sh.tables
        .iter()
        .find(|table| table.data_range() == Some(*range))
        .map(|table| table.name.clone())
}

// ------------------------------------------------------------ the Name Box

/// What the Name Box shows for the current selection.
pub fn name_box_label(app: &ExcelApp) -> String {
    let selection = app.sel();
    selection_name(&app.wb, app.sheet, selection)
        .unwrap_or_else(|| cell_name(selection.active.0, selection.active.1))
}

/// Select what a reference, a name or a table stands for, on its sheet.
/// Returns false when the areas are not all on one sheet.
pub(crate) fn select_areas(app: &mut ExcelApp, areas: &[Reference], cx: &mut Cx) -> bool {
    let Some(first) = areas.first() else {
        return false;
    };
    let sheet = first.sheet;
    if sheet >= app.wb.sheets.len() || areas.iter().any(|area| area.sheet != sheet) || app.is_editing() {
        return false;
    }
    app.deselect_object(cx);
    app.popup = None;
    app.sheet = sheet;
    while app.views.len() <= sheet {
        app.views.push(Default::default());
    }
    // The area holding the active cell comes last (see `Selection`).
    let mut ranges: Vec<Rect> = areas[1..].iter().map(|area| area.rect).collect();
    ranges.push(app.expand_merges(first.rect));
    let active = (first.rect.r0, first.rect.c0);
    app.views[sheet].sel = Selection {
        active,
        anchor: active,
        ranges,
    };
    app.ensure_visible(active.0, active.1);
    app.refresh(cx);
    true
}

/// The "refers to" text of the current selection: `=Sheet1!$A$1:$B$4`,
/// every area of a multiple selection.
pub(crate) fn selection_formula(app: &ExcelApp) -> String {
    let sheet = &app.wb.sheets[app.sheet].name;
    let style = RefStyle {
        absolute: true,
        sheet: true,
        equals: false,
    };
    let areas: Vec<String> = app.sel().ranges.iter().map(|range| style.format(*range, sheet)).collect();
    format!("={}", areas.join(&app.wb.locale.arg_sep.to_string()))
}

/// Return was pressed in the Name Box (or one of its names was picked): go
/// to the reference, the name or the table typed there; a new name is
/// given to the selection. Returns whether something was done.
pub fn name_box_entered(app: &mut ExcelApp, text: &str, cx: &mut Cx) -> bool {
    let text = text.trim();
    if text.is_empty() || app.is_editing() {
        return false;
    }
    let (row, col) = app.active();
    if let Some(areas) = app.wb.reference_areas(app.sheet, row, col, text) {
        return select_areas(app, &areas, cx);
    }
    let unknown = app.wb.lookup_name(app.sheet, text).is_none();
    if !(unknown && app.wb.check_name(NameScope::Workbook, text, None).is_ok()) {
        let message = t(
            "You must enter a valid reference you want to go to, or type a valid name for the selection.",
            "Vous devez taper une référence valide, ou taper un nom pour la sélection.",
        );
        app.alert(Alert::new("OpenSheet", message).warning(), cx);
        return false;
    }
    if let Some(message) = locked(app) {
        app.alert(Alert::new("OpenSheet", message).warning(), cx);
        return false;
    }
    let Ok(expr) = app.wb.parse_name_formula(&selection_formula(app), app.sheet, row, col) else {
        return false;
    };
    if app.wb.add_name(NameScope::Workbook, text, expr, "").is_err() {
        return false;
    }
    app.commit(t("Define Name", "Définir un nom"), cx);
    true
}

/// Put a name into the formula being typed, at the caret; a cell that is
/// not being edited starts a formula with it.
pub(crate) fn insert_in_formula(app: &mut ExcelApp, name: &str, window: &mut Window, cx: &mut Cx) {
    if !app.is_editing() {
        app.start_edit(Some(&format!("={name}")), EditMode::Edit, EditSource::Cell, window, cx);
        return;
    }
    let editor = app.editor().clone();
    let text = if editor.read(cx).text().is_empty() { format!("={name}") } else { name.to_string() };
    editor.update(cx, |input, cx| input.insert(&text, window, cx));
    app.focus_editor(window, cx);
}

// ------------------------------------------------- names from the selection

/// A reference to `rect` on the sheet named `sheet`, absolute: what a name
/// made for cells refers to.
pub(crate) fn absolute_ref(sheet: &str, rect: Rect) -> Expr {
    let mut reference = RefExpr::range(rect);
    reference.sheet = Some(Arc::from(sheet));
    reference.start.abs_row = true;
    reference.start.abs_col = true;
    if let Some(end) = &mut reference.end {
        end.abs_row = true;
        end.abs_col = true;
    }
    Expr::Ref(reference)
}

/// Where the labels are in a range that names are created from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) struct LabelSides {
    pub top: bool,
    pub left: bool,
    pub bottom: bool,
    pub right: bool,
}

impl LabelSides {
    pub fn any(&self) -> bool {
        self.top || self.left || self.bottom || self.right
    }
}

/// Which boxes of Create Names from Selection are ticked when it opens:
/// the sides of the range that hold text while the rest holds something
/// else.
pub(crate) fn guess_label_sides(wb: &Workbook, sheet: usize, rect: Rect) -> LabelSides {
    let rect = wb.clamp_to_used(sheet, rect);
    let Some(sh) = wb.sheets.get(sheet) else {
        return LabelSides::default();
    };
    // Counts of (text cells, other filled cells) in a part of the range.
    let census = |part: Rect| {
        let (mut text, mut other) = (0u32, 0u32);
        for ((_, col), cell) in sh.cells.range((part.r0, 0)..=(part.r1, u32::MAX)) {
            if *col < part.c0 || *col > part.c1 {
                continue;
            }
            match cell.value {
                Value::Text(_) => text += 1,
                Value::Empty => {}
                _ => other += 1,
            }
        }
        (text, other)
    };
    let labels = |line: Rect| {
        let (text, other) = census(line);
        text > 0 && other == 0
    };
    let (rows, cols) = (rect.rows(), rect.cols());
    let top_line = Rect::new(rect.r0, rect.c0, rect.r0, rect.c1);
    let left_line = Rect::new(rect.r0, rect.c0, rect.r1, rect.c0);
    let bottom_line = Rect::new(rect.r1, rect.c0, rect.r1, rect.c1);
    let right_line = Rect::new(rect.r0, rect.c1, rect.r1, rect.c1);
    let (_, values) = census(rect);
    let mut sides = LabelSides {
        top: rows > 1 && labels(top_line),
        left: cols > 1 && labels(left_line),
        ..LabelSides::default()
    };
    if values > 0 {
        // Labels on the far sides only count when the near ones are not.
        sides.bottom = rows > 1 && !sides.top && labels(bottom_line);
        sides.right = cols > 1 && !sides.left && labels(right_line);
    } else if sides.top && sides.left {
        // Text everywhere: a list of labels beside their texts reads by
        // row when it is two columns wide, by column otherwise.
        sides.top = cols != 2;
        sides.left = cols == 2;
    }
    sides
}

/// The name Excel makes of a label: characters a name cannot hold become
/// underscores, and so does the start of what would not read as a name.
pub(crate) fn label_to_name(label: &str, locale: &Locale) -> Option<String> {
    let mut name: String = label
        .trim()
        .chars()
        .take(255)
        .map(|ch| if ch.is_alphanumeric() || matches!(ch, '_' | '.' | '?' | '\\') { ch } else { '_' })
        .collect();
    if name.is_empty() {
        return None;
    }
    let starts_well = name.chars().next().is_some_and(|ch| ch.is_alphabetic() || ch == '_' || ch == '\\');
    if !starts_well {
        name.insert(0, '_');
    }
    // "A1", "R1C1" and "TRUE" read as something else: Excel adds an
    // underscore.
    if !is_valid_name(&name, locale) {
        name.push('_');
    }
    is_valid_name(&name, locale).then_some(name)
}

/// The names Create Names from Selection makes of `rect`: for each label
/// of the ticked sides, the cells of its row or column (and for a label in
/// the corner of two ticked sides, all the cells between the labels).
pub(crate) fn names_from_labels(wb: &Workbook, sheet: usize, rect: Rect, sides: LabelSides) -> Vec<(String, Rect)> {
    let rect = wb.clamp_to_used(sheet, rect);
    let body_rows = (rect.r0 + sides.top as u32, rect.r1.saturating_sub(sides.bottom as u32));
    let body_cols = (rect.c0 + sides.left as u32, rect.c1.saturating_sub(sides.right as u32));
    if body_rows.0 > body_rows.1 || body_cols.0 > body_cols.1 || body_rows.1 > rect.r1 || body_cols.1 > rect.c1 {
        return Vec::new();
    }
    let label = |row: u32, col: u32| -> Option<String> {
        let text = wb.display_text(sheet, row, col).text;
        label_to_name(&text, wb.locale)
    };
    let mut out: Vec<(String, Rect)> = Vec::new();
    let mut add = |name: Option<String>, cells: Rect| {
        if let Some(name) = name {
            // The first label wins when two read the same.
            if !out.iter().any(|(taken, _)| same_name(taken, &name)) {
                out.push((name, cells));
            }
        }
    };
    let body = Rect::new(body_rows.0, body_cols.0, body_rows.1, body_cols.1);
    for (row_side, row) in [(sides.top, rect.r0), (sides.bottom, rect.r1)] {
        if !row_side {
            continue;
        }
        for col in body_cols.0..=body_cols.1 {
            add(label(row, col), Rect::new(body.r0, col, body.r1, col));
        }
    }
    for (col_side, col) in [(sides.left, rect.c0), (sides.right, rect.c1)] {
        if !col_side {
            continue;
        }
        for row in body_rows.0..=body_rows.1 {
            add(label(row, col), Rect::new(row, body.c0, row, body.c1));
        }
    }
    for (row_side, row) in [(sides.top, rect.r0), (sides.bottom, rect.r1)] {
        for (col_side, col) in [(sides.left, rect.c0), (sides.right, rect.c1)] {
            if row_side && col_side {
                add(label(row, col), body);
            }
        }
    }
    out
}

// ------------------------------------------------------------- apply names

/// Apply Names: in the formulas of `ranges`, the references that are what
/// one of `names` stands for are replaced by the name. With
/// `ignore_kind`, `A1` matches a name defined as `$A$1`; without it only
/// references written like the name's are replaced. Returns how many
/// formulas changed (the caller commits).
pub(crate) fn apply_names(wb: &mut Workbook, sheet: usize, ranges: &[Rect], names: &[String], ignore_kind: bool) -> usize {
    let Some(sh) = wb.sheets.get(sheet) else {
        return 0;
    };
    let mut changes: Vec<(u32, u32, Expr)> = Vec::new();
    for ((row, col), cell) in sh.cells.iter() {
        let Content::Formula(expr) = &cell.content else {
            continue;
        };
        if !ranges.iter().any(|range| range.contains(*row, *col)) {
            continue;
        }
        // What each name stands for, read from this cell.
        let targets: Vec<(&String, RefExpr, usize)> = names
            .iter()
            .filter_map(|name| {
                let (_, def) = wb.lookup_name(sheet, name)?;
                let Expr::Ref(reference) = def.expr_at(*row, *col) else {
                    return None;
                };
                let on = match &reference.sheet {
                    Some(sheet_name) => wb.sheet_index(sheet_name)?,
                    None => sheet,
                };
                Some((name, reference, on))
            })
            .collect();
        let replaced = expr.map_refs(&mut |reference| {
            let on = match &reference.sheet {
                Some(sheet_name) => wb.sheet_index(sheet_name),
                None => Some(sheet),
            };
            let found = targets.iter().find(|(_, target, target_sheet)| {
                on == Some(*target_sheet)
                    && target.rect() == reference.rect()
                    && (ignore_kind || (target.start, target.end, target.kind) == (reference.start, reference.end, reference.kind))
            });
            match found {
                Some((name, _, _)) => Expr::Name(Arc::from(name.as_str())),
                None => Expr::Ref(reference.clone()),
            }
        });
        if replaced != **expr {
            changes.push((*row, *col, replaced));
        }
    }
    let count = changes.len();
    for (row, col, expr) in changes {
        wb.set_formula(sheet, row, col, expr);
    }
    count
}

// --------------------------------------------------------------- paste list

/// Paste List: the names a formula of the sheet can use and what they
/// refer to, in two columns starting at the active cell.
pub(crate) fn paste_list(app: &mut ExcelApp, cx: &mut Cx) {
    let (row, col) = app.active();
    let sheet = app.sheet;
    let lines: Vec<(String, String)> = formula_names(&app.wb, sheet)
        .into_iter()
        .filter_map(|name| {
            let (_, def) = app.wb.lookup_name(sheet, &name)?;
            Some((def.name.clone(), def.refers_to(app.wb.locale, row, col)))
        })
        .collect();
    if lines.is_empty() {
        return;
    }
    let rect = Rect::new(row, col, row + lines.len() as u32 - 1, col + 1);
    if !app.may(Intent::EditCells(rect), cx) {
        return;
    }
    for (i, (name, refers_to)) in lines.into_iter().enumerate() {
        // Text, not formulas: the list is there to be read.
        app.wb.set_value(sheet, row + i as u32, col, Value::text(name));
        app.wb.set_value(sheet, row + i as u32, col + 1, Value::text(refers_to));
    }
    let cause = ChangeCause::Paste {
        from: None,
        cut: false,
        mode: PasteMode::All,
    };
    app.commit_cells(t("Paste List", "Coller une liste"), &[rect], cause, cx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::parser::parse_formula;

    fn workbook() -> Workbook {
        let mut wb = Workbook::new(Locale::en());
        wb.add_sheet(1);
        let cells = [
            ("A1", "Item"),
            ("B1", "Qty"),
            ("C1", "Unit price"),
            ("A2", "Pens"),
            ("B2", "12"),
            ("C2", "1.5"),
            ("A3", "Ink"),
            ("B3", "3"),
            ("C3", "7"),
            ("E1", "Colour"),
            ("F1", "Blue"),
            ("E2", "Size"),
            ("F2", "Large"),
        ];
        for (cell, text) in cells {
            let (row, col) = opensheet_engine::refs::parse_cell_name(cell).unwrap();
            wb.set_cell_text(0, row, col, text).unwrap();
        }
        wb.commit("cells");
        wb
    }

    fn define(wb: &mut Workbook, scope: NameScope, name: &str, formula: &str, comment: &str) {
        let expr = wb.parse_name_formula(formula, 0, 0, 0).unwrap();
        wb.add_name(scope, name, expr, comment).unwrap();
        wb.commit("name");
    }

    #[test]
    fn rows_list_names_with_values_scopes_and_errors() {
        let mut wb = workbook();
        define(&mut wb, NameScope::Workbook, "Qty", "=Sheet1!$B$2:$B$3", "units");
        define(&mut wb, NameScope::Workbook, "rate", "=0.2", "");
        define(&mut wb, NameScope::Sheet(1), "Local", "=Sheet1!$A$2", "");
        define(&mut wb, NameScope::Workbook, "Broken", "=1/0", "");
        let rows = name_rows(&wb, 0, 0, 0);
        let names: Vec<&str> = rows.iter().map(|row| row.name.as_str()).collect();
        assert_eq!(names, ["Broken", "Local", "Qty", "rate"]);
        let qty = &rows[2];
        assert_eq!(qty.value, "{\"12\";\"3\"}");
        assert_eq!(qty.refers_to, "=Sheet1!$B$2:$B$3");
        assert_eq!((qty.scope.as_str(), qty.comment.as_str()), ("Workbook", "units"));
        assert_eq!(rows[3].value, "0.2");
        assert_eq!((rows[1].value.as_str(), rows[1].scope.as_str()), ("Pens", "Sheet2"));
        assert!(rows[0].error && rows[0].value == "#DIV/0!");
        assert!(!qty.error);

        let only = |filter: NameFilter| -> Vec<String> {
            filter_rows(rows.clone(), &filter).into_iter().map(|row| row.name).collect()
        };
        assert_eq!(only(NameFilter { sheet_scoped: Some(true), ..Default::default() }), ["Local"]);
        assert_eq!(only(NameFilter { sheet_scoped: Some(false), with_errors: Some(false), ..Default::default() }), ["Qty", "rate"]);
        assert_eq!(only(NameFilter { with_errors: Some(true), ..Default::default() }), ["Broken"]);
        assert!(only(NameFilter { tables: Some(true), ..Default::default() }).is_empty());
        assert!(NameFilter::default().is_empty());

        let mut sorted = rows.clone();
        sort_rows(&mut sorted, Column::Scope, false);
        assert_eq!(sorted[0].name, "Broken");
        assert_eq!(sorted.last().unwrap().name, "Local");
    }

    #[test]
    fn tables_are_listed_with_their_data_rows() {
        let mut wb = workbook();
        wb.add_table(0, Rect::new(0, 0, 2, 2), true).unwrap();
        wb.commit("table");
        let rows = name_rows(&wb, 0, 0, 0);
        assert_eq!(rows.len(), 1);
        assert!(rows[0].is_table());
        assert_eq!(rows[0].refers_to, "=Sheet1!$A$2:$C$3");
        assert_eq!(rows[0].value, "{\"Pens\",\"12\",\"1.5\";\"Ink\",\"3\",\"7\"}");
        assert_eq!(visible_names(&wb, 1), [rows[0].name.clone()]);
        let body = Selection {
            active: (1, 0),
            anchor: (1, 0),
            ranges: vec![Rect::new(1, 0, 2, 2)],
        };
        assert_eq!(selection_name(&wb, 0, &body), Some(rows[0].name.clone()));
    }

    #[test]
    fn values_read_as_excel_writes_them() {
        let en = Locale::en();
        let fr = Locale::fr();
        let array = Value::array(opensheet_engine::Array::from_rows(vec![
            vec![Value::Number(1.5), Value::text("a\"b")],
            vec![Value::Bool(true), Value::Empty],
        ]));
        assert_eq!(listed_value(&array, en), "{\"1.5\",\"a\"b\";\"TRUE\",\"\"}");
        assert_eq!(literal_value(&array, en), "{1.5,\"a\"\"b\";TRUE,0}");
        assert_eq!(literal_value(&array, fr), "{1,5.\"a\"\"b\";VRAI.0}");
        assert_eq!(literal_value(&Value::text("x"), en), "\"x\"");
        assert_eq!(literal_value(&Value::Empty, en), "");
        assert_eq!(listed_value(&Value::text("x"), en), "x");
        let long = Value::array(opensheet_engine::Array::column((0..40).map(|n| Value::Number(n as f64)).collect()));
        assert!(literal_value(&long, en).ends_with(";…}"));
    }

    #[test]
    fn names_seen_from_a_sheet() {
        let mut wb = workbook();
        define(&mut wb, NameScope::Workbook, "Qty", "=Sheet1!$B$2:$B$3", "");
        define(&mut wb, NameScope::Workbook, "Rate", "=0.2", "");
        define(&mut wb, NameScope::Sheet(1), "qty", "=Sheet2!$A$1", "");
        define(&mut wb, NameScope::Sheet(1), "Zone", "=Sheet2!$C$3:$D$4", "");
        assert_eq!(formula_names(&wb, 0), ["Qty", "Rate"]);
        // The sheet's own "qty" hides the workbook's.
        assert_eq!(formula_names(&wb, 1), ["qty", "Rate", "Zone"]);
        // Constants are not places to go to.
        assert_eq!(visible_names(&wb, 0), ["Qty"]);
        assert_eq!(visible_names(&wb, 1), ["qty", "Zone"]);

        let select = |ranges: Vec<Rect>| Selection {
            active: (ranges[0].r0, ranges[0].c0),
            anchor: (ranges[0].r0, ranges[0].c0),
            ranges,
        };
        assert_eq!(selection_name(&wb, 0, &select(vec![Rect::new(1, 1, 2, 1)])), Some("Qty".to_string()));
        assert_eq!(selection_name(&wb, 0, &select(vec![Rect::new(1, 1, 1, 1)])), None);
        assert_eq!(selection_name(&wb, 1, &select(vec![Rect::cell(0, 0)])), Some("qty".to_string()));
        assert_eq!(selection_name(&wb, 1, &select(vec![Rect::new(2, 2, 3, 3)])), Some("Zone".to_string()));
        assert_eq!(selection_name(&wb, 0, &select(vec![Rect::new(1, 1, 2, 1), Rect::cell(5, 5)])), None);
    }

    #[test]
    fn labels_become_names() {
        let en = Locale::en();
        let fr = Locale::fr();
        assert_eq!(label_to_name("Unit price", en).as_deref(), Some("Unit_price"));
        assert_eq!(label_to_name("  Total (€) ", en).as_deref(), Some("Total____"));
        assert_eq!(label_to_name("2024", en).as_deref(), Some("_2024"));
        assert_eq!(label_to_name("A1", en).as_deref(), Some("A1_"));
        assert_eq!(label_to_name("TRUE", en).as_deref(), Some("TRUE_"));
        assert_eq!(label_to_name("L1C1", fr).as_deref(), Some("L1C1_"));
        assert_eq!(label_to_name("Été.2024", fr).as_deref(), Some("Été.2024"));
        assert_eq!(label_to_name("   ", en), None);
    }

    #[test]
    fn label_sides_are_guessed_from_where_the_text_is() {
        let wb = workbook();
        let all = Rect::new(0, 0, 2, 2);
        assert_eq!(guess_label_sides(&wb, 0, all), LabelSides { top: true, left: true, ..Default::default() });
        // Numbers under their headers only.
        assert_eq!(guess_label_sides(&wb, 0, Rect::new(0, 1, 2, 2)), LabelSides { top: true, ..Default::default() });
        // Labels beside their numbers.
        assert_eq!(guess_label_sides(&wb, 0, Rect::new(1, 0, 2, 2)), LabelSides { left: true, ..Default::default() });
        // Numbers only: nothing to go by.
        assert!(!guess_label_sides(&wb, 0, Rect::new(1, 1, 2, 2)).any());
        // Text only: a header over a column, labels beside their texts.
        assert_eq!(guess_label_sides(&wb, 0, Rect::new(0, 0, 2, 0)), LabelSides { top: true, ..Default::default() });
        assert_eq!(guess_label_sides(&wb, 0, Rect::new(0, 4, 1, 5)), LabelSides { left: true, ..Default::default() });
        // A whole-column selection is read up to the last used row.
        assert_eq!(guess_label_sides(&wb, 0, Rect::new(0, 1, opensheet_engine::MAX_ROWS - 1, 2)), LabelSides { top: true, ..Default::default() });
    }

    #[test]
    fn names_are_made_from_the_labels_of_the_ticked_sides() {
        let wb = workbook();
        let all = Rect::new(0, 0, 2, 2);
        let named = |sides: LabelSides| -> Vec<(String, String)> {
            names_from_labels(&wb, 0, all, sides).into_iter().map(|(name, rect)| (name, rect.name())).collect()
        };
        let pair = |name: &str, cells: &str| (name.to_string(), cells.to_string());
        assert_eq!(
            named(LabelSides { top: true, ..Default::default() }),
            [pair("Item", "A2:A3"), pair("Qty", "B2:B3"), pair("Unit_price", "C2:C3")]
        );
        assert_eq!(
            named(LabelSides { top: true, left: true, ..Default::default() }),
            [pair("Qty", "B2:B3"), pair("Unit_price", "C2:C3"), pair("Pens", "B2:C2"), pair("Ink", "B3:C3"), pair("Item", "B2:C3")]
        );
        assert_eq!(
            named(LabelSides { left: true, ..Default::default() }),
            [pair("Item", "B1:C1"), pair("Pens", "B2:C2"), pair("Ink", "B3:C3")]
        );
        // Numbers make names too, with an underscore in front.
        assert_eq!(named(LabelSides { bottom: true, ..Default::default() })[1], pair("_3", "B1:B2"));
        // A single row has no cells left to name.
        assert!(names_from_labels(&wb, 0, Rect::new(0, 0, 0, 2), LabelSides { top: true, ..Default::default() }).is_empty());
    }

    #[test]
    fn apply_names_replaces_matching_references() {
        let fixture = || {
            let mut wb = workbook();
            define(&mut wb, NameScope::Workbook, "Qty", "=Sheet1!$B$2:$B$3", "");
            define(&mut wb, NameScope::Workbook, "First", "=Sheet1!$C$2", "");
            wb.set_cell_text(0, 4, 0, "=SUM(B2:B3)+C2").unwrap();
            wb.set_cell_text(0, 5, 0, "=SUM($B$2:$B$3)*$C$2+B2").unwrap();
            wb.commit("formulas");
            wb
        };
        let names = ["Qty".to_string(), "First".to_string()];
        let sheet = [Rect::new(0, 0, 100, 100)];

        // References written like the names' only.
        let mut strict = fixture();
        assert_eq!(apply_names(&mut strict, 0, &sheet, &names, false), 1);
        assert_eq!(strict.formula_text(0, 4, 0).unwrap(), "=SUM(B2:B3)+C2");
        assert_eq!(strict.formula_text(0, 5, 0).unwrap(), "=SUM(Qty)*First+B2");

        let mut wb = fixture();
        assert_eq!(apply_names(&mut wb, 0, &sheet, &names, true), 2);
        assert_eq!(wb.formula_text(0, 4, 0).unwrap(), "=SUM(Qty)+First");
        assert_eq!(wb.formula_text(0, 5, 0).unwrap(), "=SUM(Qty)*First+B2");
        // Nothing left to replace, and nothing outside the ranges.
        assert_eq!(apply_names(&mut wb, 0, &sheet, &names, true), 0);
        let expr = parse_formula("SUM(B2:B3)", Locale::en()).unwrap();
        wb.set_formula(0, 8, 0, expr);
        assert_eq!(apply_names(&mut wb, 0, &[Rect::cell(4, 0)], &names, true), 0);
    }
}
