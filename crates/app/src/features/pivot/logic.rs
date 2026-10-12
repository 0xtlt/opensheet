//! What the pivot interface decides without a window: what a drop in the
//! field list means, the lists of the dialogs and what their choices stand
//! for, the widths of the columns after an update, the wording of the
//! messages.

use crate::features::{ChangeCause, Intent};
use crate::i18n::t;
use opensheet_engine::ast::StructuralEdit;
use opensheet_engine::ops::PasteMode;
use opensheet_engine::pivot::{
    Aggregation, Area, AxisField, BaseItem, FieldGroup, FieldLayout, FilterKind, GroupKind, LabelFilter, PivotButton,
    PivotCell, PivotError, PivotPart, PivotSource, PivotTable, ShowAs, SortOrder, Subtotals, Top10By, ValueFilter,
};
use opensheet_engine::workbook::ClearMode;
use opensheet_engine::{Expr, Locale, Rect, Style, Value, Workbook};

// ----------------------------------------------------------------- messages

/// Excel's message for a command the engine refused.
pub fn error_text(err: &PivotError) -> &'static str {
    match err {
        PivotError::InvalidSource => t(
            "Data source reference is not valid.",
            "La référence à la source de données n'est pas valide.",
        ),
        PivotError::EmptyFieldName => t(
            "The PivotTable field name is not valid. To create a PivotTable report, you must use data that is organized as a list with labeled columns. If you are changing the name of a PivotTable field, you must type a new name for the field.",
            "Le nom du champ de tableau croisé dynamique n'est pas valide. Pour créer un rapport de tableau croisé dynamique, vous devez utiliser des données sous forme de liste avec des étiquettes de colonnes. Si vous changez le nom d'un champ de tableau croisé dynamique, vous devez taper un nouveau nom pour le champ.",
        ),
        PivotError::OverlapsPivot(_) => t(
            "A PivotTable report cannot overlap another PivotTable report.",
            "Impossible d'afficher le rapport de tableau croisé dynamique sur un rapport existant.",
        ),
        PivotError::OverlapsTable => t(
            "A PivotTable report cannot overlap a table or XML mapping.",
            "Un rapport de tableau croisé dynamique ne peut pas chevaucher un tableau ou un mappage XML.",
        ),
        PivotError::OverlapsMerged => t(
            "We can't do that to a merged cell.",
            "Cette action ne peut pas être appliquée à une cellule fusionnée.",
        ),
        PivotError::WouldOverwrite(_) => t(
            "A PivotTable report cannot overlap cells with data.",
            "Un rapport de tableau croisé dynamique ne peut pas chevaucher des cellules avec des données.",
        ),
        PivotError::TooLarge => t(
            "The PivotTable report will not fit on the sheet. Please select a different location.",
            "Le rapport de tableau croisé dynamique est plus grand que la feuille. Sélectionnez un autre emplacement.",
        ),
        PivotError::CannotGroup => t("Cannot group that selection.", "Impossible de grouper cette sélection."),
        PivotError::CannotUngroup => t(
            "Cannot ungroup that selection.",
            "Impossible de dissocier le groupe de cette sélection.",
        ),
        PivotError::InvalidName => t("PivotTable name is not valid.", "Nom du tableau croisé dynamique non valide."),
        PivotError::NoSuchPivot | PivotError::NoSuchField | PivotError::NotAValue => cannot_change_part(),
    }
}

/// What typing into a PivotTable answers.
pub fn cannot_change_part() -> &'static str {
    t(
        "We can't change this part of the PivotTable.",
        "Désolé... Nous ne pouvons pas modifier cette partie du tableau croisé dynamique.",
    )
}

/// What an edit cutting through a PivotTable answers.
pub fn would_affect_pivot() -> &'static str {
    t(
        "We can't make this change for the selected cells because it will affect a PivotTable. Use the field list to change the report. If you are trying to insert or delete cells, move the PivotTable and try again.",
        "Nous ne pouvons pas modifier les cellules sélectionnées, car cela affecterait un tableau croisé dynamique. Pour modifier le rapport, veuillez utiliser la liste de champs. Si vous essayez d’insérer ou de supprimer des cellules, nous vous conseillons de déplacer le tableau croisé dynamique, puis de réessayer.",
    )
}

pub fn name_exists() -> &'static str {
    t("PivotTable field name already exists.", "Le nom du champ dynamique existe déjà.")
}

/// The question asked before a report covers cells holding data.
pub fn replace_question(sheet_name: &str) -> String {
    t(
        "There's already data in {}. Do you want to replace it?",
        "{} contient déjà des données. Voulez-vous les remplacer ?",
    )
    .replace("{}", sheet_name)
}

/// Why a change to the sheet is refused, if a PivotTable is in its way.
/// The cells of a PivotTable belong to its report: they are not typed
/// over, cleared, pasted on nor sorted, and no row or column is inserted
/// or deleted through them. A range covering a PivotTable whole may be
/// cleared or overwritten: the PivotTable goes with it.
pub fn refusal(wb: &Workbook, sheet: usize, intent: &Intent) -> Option<&'static str> {
    if wb.sheets.get(sheet)?.pivots.is_empty() {
        return None;
    }
    let cut = |edit: StructuralEdit| wb.pivot_blocks_edit(sheet, edit).then(would_affect_pivot);
    match *intent {
        Intent::EditCells(rect) => {
            if wb.pivots_in(sheet, rect).iter().all(|(_, whole)| *whole) {
                None
            } else if rect.is_single() {
                Some(cannot_change_part())
            } else {
                Some(would_affect_pivot())
            }
        }
        Intent::Sort(rect) => (!wb.pivots_in(sheet, rect).is_empty()).then(would_affect_pivot),
        Intent::InsertRows { at, count } => cut(StructuralEdit::InsertRows { at, count }),
        Intent::InsertColumns { at, count } => cut(StructuralEdit::InsertCols { at, count }),
        Intent::DeleteRows { first, last } => cut(StructuralEdit::DeleteRows {
            at: first,
            count: last - first + 1,
        }),
        Intent::DeleteColumns { first, last } => cut(StructuralEdit::DeleteCols {
            at: first,
            count: last - first + 1,
        }),
        _ => None,
    }
}

/// Whether an edit replaces what its cells hold (rather than only their
/// formats, comments, validation or widths): only then does a PivotTable
/// lying whole under it go away with its report.
pub fn replaces_contents(cause: ChangeCause) -> bool {
    match cause {
        ChangeCause::Typing | ChangeCause::Fill => true,
        ChangeCause::Clear(mode) => matches!(mode, ClearMode::Contents | ClearMode::All),
        ChangeCause::Paste { mode, .. } => !matches!(
            mode,
            PasteMode::Formats | PasteMode::Comments | PasteMode::Validation | PasteMode::ColumnWidths
        ),
    }
}

// ------------------------------------------------------------ the field list

/// Where something dragged in the PivotTable Fields pane comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragSource {
    /// A field of the check-list.
    List(usize),
    /// The entry at this position of an area.
    Area(Area, usize),
}

/// A change made in the PivotTable Fields pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneOp {
    /// Tick a field of the list: it goes where Excel puts it (numbers in
    /// the values, the rest in the rows).
    Check(usize),
    /// Untick it: it leaves every area.
    Uncheck(usize),
    Add {
        field: usize,
        area: Area,
        position: Option<usize>,
    },
    /// The entry at `index` of `from` goes to `position` of `to`, counted
    /// once the entry has left its place.
    Move {
        from: Area,
        index: usize,
        to: Area,
        position: usize,
    },
    Remove {
        area: Area,
        index: usize,
    },
}

/// One line of an area box.
#[derive(Clone, Debug, PartialEq)]
pub struct AreaEntry {
    pub label: String,
    /// `None` for "Σ Values".
    pub field: Option<usize>,
}

/// The entries of an area, in order.
pub fn area_entries(p: &PivotTable, area: Area, locale: &Locale) -> Vec<AreaEntry> {
    let axis = |list: &[AxisField]| {
        list.iter()
            .map(|a| match a {
                AxisField::Field(f) => AreaEntry {
                    label: p.field_caption(*f).to_string(),
                    field: Some(*f),
                },
                AxisField::Values => AreaEntry {
                    label: format!("Σ {}", p.data_caption(locale)),
                    field: None,
                },
            })
            .collect()
    };
    match area {
        Area::Rows => axis(&p.row_fields),
        Area::Columns => axis(&p.col_fields),
        Area::Filters => p
            .page_fields
            .iter()
            .map(|pf| AreaEntry {
                label: p.field_caption(pf.field).to_string(),
                field: Some(pf.field),
            })
            .collect(),
        Area::Values => p
            .data_fields
            .iter()
            .map(|d| AreaEntry {
                label: d.name.clone(),
                field: Some(d.field),
            })
            .collect(),
    }
}

/// What dropping `source` means. `target`: the area it was dropped on and
/// the entry it was dropped before (the number of entries: after the last
/// one); `None` when it was dropped outside the areas.
pub fn drop_op(p: &PivotTable, source: DragSource, target: Option<(Area, usize)>, locale: &Locale) -> Option<PaneOp> {
    let Some((to, slot)) = target else {
        // Dragging an entry out of its area removes it.
        return match source {
            DragSource::Area(area, index) => Some(PaneOp::Remove { area, index }),
            DragSource::List(_) => None,
        };
    };
    // Leaving a place before the slot moves the slot up by one.
    let within = |index: usize| {
        let position = if slot > index { slot - 1 } else { slot };
        (position != index).then_some(PaneOp::Move {
            from: to,
            index,
            to,
            position,
        })
    };
    match source {
        DragSource::List(field) => {
            // A field already in the area moves inside it (the values can
            // hold the same field twice).
            let entries = area_entries(p, to, locale);
            match entries.iter().position(|e| e.field == Some(field)) {
                Some(index) if to != Area::Values => within(index),
                _ => Some(PaneOp::Add {
                    field,
                    area: to,
                    position: Some(slot),
                }),
            }
        }
        DragSource::Area(from, index) if from == to => within(index),
        DragSource::Area(from, index) => {
            let entry = area_entries(p, from, locale).into_iter().nth(index)?;
            // "Σ Values" only lives in the rows or the columns.
            if entry.field.is_none() && !matches!(to, Area::Rows | Area::Columns) {
                return None;
            }
            Some(PaneOp::Move {
                from,
                index,
                to,
                position: slot,
            })
        }
    }
}

/// Apply a change of the pane to a definition. (A change made at once
/// rather than deferred goes through `run_op`, which also groups dates.)
pub fn apply_op(p: &mut PivotTable, op: PaneOp, locale: &Locale) {
    match op {
        PaneOp::Check(field) => {
            let area = p.default_area(field);
            p.add_field(field, area, None, locale);
        }
        PaneOp::Uncheck(field) => {
            for index in (0..p.data_fields.len()).rev() {
                if p.data_fields[index].field == field {
                    p.remove_data_field(index);
                }
            }
            p.remove_field(field);
        }
        PaneOp::Add { field, area, position } => p.add_field(field, area, position, locale),
        PaneOp::Move {
            from,
            index,
            to,
            position,
        } => {
            p.move_entry(from, index, to, position, locale);
        }
        PaneOp::Remove {
            area: Area::Values,
            index,
        } => p.remove_data_field(index),
        PaneOp::Remove { area, index } => match area_entries(p, area, locale).get(index).map(|e| e.field) {
            Some(Some(field)) => p.remove_field(field),
            // Removing "Σ Values" removes the value fields.
            Some(None) => {
                for index in (0..p.data_fields.len()).rev() {
                    p.remove_data_field(index);
                }
            }
            None => {}
        },
    }
}

/// Make a change of the pane in the workbook. A date field that reaches the
/// rows or the columns for the first time is grouped as Excel groups it.
pub fn run_op(wb: &mut Workbook, id: u32, op: PaneOp, overwrite: bool) -> Result<(), PivotError> {
    let locale = wb.locale;
    let (_, p) = wb.pivot(id).ok_or(PivotError::NoSuchPivot)?;
    let grouped = |field: usize, area: Area| {
        matches!(area, Area::Rows | Area::Columns) && !p.cache.auto_date_periods(field).is_empty()
    };
    let through_engine = match op {
        PaneOp::Check(field) => Some((field, p.default_area(field), None)),
        PaneOp::Add { field, area, position } if area != Area::Values => Some((field, area, position)),
        PaneOp::Move {
            from: Area::Filters,
            index,
            to,
            position,
        } => p
            .page_fields
            .get(index)
            .map(|pf| pf.field)
            .filter(|field| grouped(*field, to))
            .map(|field| (field, to, Some(position))),
        _ => None,
    };
    match through_engine {
        Some((field, area, position)) => wb.pivot_add_field(id, field, area, position, overwrite),
        None => wb.pivot_update(id, overwrite, |p| apply_op(p, op, locale)),
    }
}

/// What the menu of an area entry offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EntryMoves {
    pub up: bool,
    pub down: bool,
    pub to_filters: bool,
    pub to_rows: bool,
    pub to_columns: bool,
    pub to_values: bool,
    /// The entry is a value field ("Value Field Settings…").
    pub value: bool,
    /// The entry is "Σ Values": it has no settings.
    pub sigma: bool,
}

pub fn entry_moves(p: &PivotTable, area: Area, index: usize, locale: &Locale) -> EntryMoves {
    let entries = area_entries(p, area, locale);
    let sigma = entries.get(index).is_some_and(|e| e.field.is_none());
    EntryMoves {
        up: index > 0,
        down: index + 1 < entries.len(),
        to_filters: area != Area::Filters && !sigma,
        to_rows: area != Area::Rows,
        to_columns: area != Area::Columns,
        to_values: area != Area::Values && !sigma,
        value: area == Area::Values,
        sigma,
    }
}

// ------------------------------------------------------------------ filters

/// The comparisons of the Label Filter dialog, in the order of its list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelTest {
    Equals,
    NotEquals,
    GreaterThan,
    GreaterOrEqual,
    LessThan,
    LessOrEqual,
    BeginsWith,
    NotBeginsWith,
    EndsWith,
    NotEndsWith,
    Contains,
    NotContains,
    Between,
    NotBetween,
}

impl LabelTest {
    pub const ALL: [LabelTest; 14] = [
        LabelTest::Equals,
        LabelTest::NotEquals,
        LabelTest::GreaterThan,
        LabelTest::GreaterOrEqual,
        LabelTest::LessThan,
        LabelTest::LessOrEqual,
        LabelTest::BeginsWith,
        LabelTest::NotBeginsWith,
        LabelTest::EndsWith,
        LabelTest::NotEndsWith,
        LabelTest::Contains,
        LabelTest::NotContains,
        LabelTest::Between,
        LabelTest::NotBetween,
    ];

    /// The groups of the "Label Filters" menu, separated by lines.
    pub const MENU: [&'static [LabelTest]; 5] = [
        &[LabelTest::Equals, LabelTest::NotEquals],
        &[
            LabelTest::BeginsWith,
            LabelTest::NotBeginsWith,
            LabelTest::EndsWith,
            LabelTest::NotEndsWith,
        ],
        &[LabelTest::Contains, LabelTest::NotContains],
        &[
            LabelTest::GreaterThan,
            LabelTest::GreaterOrEqual,
            LabelTest::LessThan,
            LabelTest::LessOrEqual,
        ],
        &[LabelTest::Between, LabelTest::NotBetween],
    ];

    /// As the list of the dialog words it.
    pub fn label(self) -> &'static str {
        match self {
            LabelTest::Equals => t("equals", "est égal à"),
            LabelTest::NotEquals => t("does not equal", "est différent de"),
            LabelTest::GreaterThan => t("is greater than", "est supérieur à"),
            LabelTest::GreaterOrEqual => t("is greater than or equal to", "est supérieur ou égal à"),
            LabelTest::LessThan => t("is less than", "est inférieur à"),
            LabelTest::LessOrEqual => t("is less than or equal to", "est inférieur ou égal à"),
            LabelTest::BeginsWith => t("begins with", "commence par"),
            LabelTest::NotBeginsWith => t("does not begin with", "ne commence pas par"),
            LabelTest::EndsWith => t("ends with", "se termine par"),
            LabelTest::NotEndsWith => t("does not end with", "ne se termine pas par"),
            LabelTest::Contains => t("contains", "contient"),
            LabelTest::NotContains => t("does not contain", "ne contient pas"),
            LabelTest::Between => t("is between", "est compris entre"),
            LabelTest::NotBetween => t("is not between", "n'est pas compris entre"),
        }
    }

    /// As the menus word it.
    pub fn menu_label(self) -> &'static str {
        match self {
            LabelTest::Equals => t("Equals...", "Est égal à..."),
            LabelTest::NotEquals => t("Does Not Equal...", "Est différent de..."),
            LabelTest::GreaterThan => t("Greater Than...", "Supérieur à..."),
            LabelTest::GreaterOrEqual => t("Greater Than Or Equal To...", "Supérieur ou égal à..."),
            LabelTest::LessThan => t("Less Than...", "Inférieur à..."),
            LabelTest::LessOrEqual => t("Less Than Or Equal To...", "Inférieur ou égal à..."),
            LabelTest::BeginsWith => t("Begins With...", "Commence par..."),
            LabelTest::NotBeginsWith => t("Does Not Begin With...", "Ne commence pas par..."),
            LabelTest::EndsWith => t("Ends With...", "Se termine par..."),
            LabelTest::NotEndsWith => t("Does Not End With...", "Ne se termine pas par..."),
            LabelTest::Contains => t("Contains...", "Contient..."),
            LabelTest::NotContains => t("Does Not Contain...", "Ne contient pas..."),
            LabelTest::Between => t("Between...", "Entre..."),
            LabelTest::NotBetween => t("Not Between...", "Non comprise entre..."),
        }
    }

    /// The test takes two operands.
    pub fn two(self) -> bool {
        matches!(self, LabelTest::Between | LabelTest::NotBetween)
    }

    pub fn build(self, a: &str, b: &str) -> LabelFilter {
        let (a, b) = (a.to_string(), b.to_string());
        match self {
            LabelTest::Equals => LabelFilter::Equals(a),
            LabelTest::NotEquals => LabelFilter::NotEquals(a),
            LabelTest::GreaterThan => LabelFilter::GreaterThan(a),
            LabelTest::GreaterOrEqual => LabelFilter::GreaterOrEqual(a),
            LabelTest::LessThan => LabelFilter::LessThan(a),
            LabelTest::LessOrEqual => LabelFilter::LessOrEqual(a),
            LabelTest::BeginsWith => LabelFilter::BeginsWith(a),
            LabelTest::NotBeginsWith => LabelFilter::NotBeginsWith(a),
            LabelTest::EndsWith => LabelFilter::EndsWith(a),
            LabelTest::NotEndsWith => LabelFilter::NotEndsWith(a),
            LabelTest::Contains => LabelFilter::Contains(a),
            LabelTest::NotContains => LabelFilter::NotContains(a),
            LabelTest::Between => LabelFilter::Between(a, b),
            LabelTest::NotBetween => LabelFilter::NotBetween(a, b),
        }
    }

    /// The test and the operands of a filter.
    pub fn of(filter: &LabelFilter) -> (LabelTest, String, String) {
        let one = |test: LabelTest, a: &String| (test, a.clone(), String::new());
        match filter {
            LabelFilter::Equals(a) => one(LabelTest::Equals, a),
            LabelFilter::NotEquals(a) => one(LabelTest::NotEquals, a),
            LabelFilter::GreaterThan(a) => one(LabelTest::GreaterThan, a),
            LabelFilter::GreaterOrEqual(a) => one(LabelTest::GreaterOrEqual, a),
            LabelFilter::LessThan(a) => one(LabelTest::LessThan, a),
            LabelFilter::LessOrEqual(a) => one(LabelTest::LessOrEqual, a),
            LabelFilter::BeginsWith(a) => one(LabelTest::BeginsWith, a),
            LabelFilter::NotBeginsWith(a) => one(LabelTest::NotBeginsWith, a),
            LabelFilter::EndsWith(a) => one(LabelTest::EndsWith, a),
            LabelFilter::NotEndsWith(a) => one(LabelTest::NotEndsWith, a),
            LabelFilter::Contains(a) => one(LabelTest::Contains, a),
            LabelFilter::NotContains(a) => one(LabelTest::NotContains, a),
            LabelFilter::Between(a, b) => (LabelTest::Between, a.clone(), b.clone()),
            LabelFilter::NotBetween(a, b) => (LabelTest::NotBetween, a.clone(), b.clone()),
        }
    }
}

/// The comparisons of the Value Filter dialog, in the order of its list
/// (and of the "Value Filters" menu).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueTest {
    Equals,
    NotEquals,
    GreaterThan,
    GreaterOrEqual,
    LessThan,
    LessOrEqual,
    Between,
    NotBetween,
}

impl ValueTest {
    pub const ALL: [ValueTest; 8] = [
        ValueTest::Equals,
        ValueTest::NotEquals,
        ValueTest::GreaterThan,
        ValueTest::GreaterOrEqual,
        ValueTest::LessThan,
        ValueTest::LessOrEqual,
        ValueTest::Between,
        ValueTest::NotBetween,
    ];

    fn as_label(self) -> LabelTest {
        match self {
            ValueTest::Equals => LabelTest::Equals,
            ValueTest::NotEquals => LabelTest::NotEquals,
            ValueTest::GreaterThan => LabelTest::GreaterThan,
            ValueTest::GreaterOrEqual => LabelTest::GreaterOrEqual,
            ValueTest::LessThan => LabelTest::LessThan,
            ValueTest::LessOrEqual => LabelTest::LessOrEqual,
            ValueTest::Between => LabelTest::Between,
            ValueTest::NotBetween => LabelTest::NotBetween,
        }
    }

    pub fn label(self) -> &'static str {
        self.as_label().label()
    }

    pub fn menu_label(self) -> &'static str {
        self.as_label().menu_label()
    }

    pub fn two(self) -> bool {
        self.as_label().two()
    }

    pub fn build(self, a: f64, b: f64) -> ValueFilter {
        match self {
            ValueTest::Equals => ValueFilter::Equals(a),
            ValueTest::NotEquals => ValueFilter::NotEquals(a),
            ValueTest::GreaterThan => ValueFilter::GreaterThan(a),
            ValueTest::GreaterOrEqual => ValueFilter::GreaterOrEqual(a),
            ValueTest::LessThan => ValueFilter::LessThan(a),
            ValueTest::LessOrEqual => ValueFilter::LessOrEqual(a),
            ValueTest::Between => ValueFilter::Between(a.min(b), a.max(b)),
            ValueTest::NotBetween => ValueFilter::NotBetween(a.min(b), a.max(b)),
        }
    }

    pub fn of(filter: &ValueFilter) -> (ValueTest, f64, Option<f64>) {
        match *filter {
            ValueFilter::Equals(a) => (ValueTest::Equals, a, None),
            ValueFilter::NotEquals(a) => (ValueTest::NotEquals, a, None),
            ValueFilter::GreaterThan(a) => (ValueTest::GreaterThan, a, None),
            ValueFilter::GreaterOrEqual(a) => (ValueTest::GreaterOrEqual, a, None),
            ValueFilter::LessThan(a) => (ValueTest::LessThan, a, None),
            ValueFilter::LessOrEqual(a) => (ValueTest::LessOrEqual, a, None),
            ValueFilter::Between(a, b) => (ValueTest::Between, a, Some(b)),
            ValueFilter::NotBetween(a, b) => (ValueTest::NotBetween, a, Some(b)),
        }
    }
}

/// The label filter of a field, if it has one.
pub fn label_filter(p: &PivotTable, field: usize) -> Option<&LabelFilter> {
    p.filters.iter().find_map(|f| match &f.kind {
        FilterKind::Label(l) if f.field == field => Some(l),
        _ => None,
    })
}

/// The value filter (a test or a Top 10) of a field, if it has one.
pub fn value_filter(p: &PivotTable, field: usize) -> Option<&FilterKind> {
    p.filters
        .iter()
        .find(|f| f.field == field && !matches!(f.kind, FilterKind::Label(_)))
        .map(|f| &f.kind)
}

/// The items a check-list leaves visible when OK is pressed. `checked`:
/// the state of every item; `shown`: the items a search narrowed the list
/// to (`None` without a search). As in Excel, the result of a search
/// replaces the selection: what the search hides is hidden.
pub fn items_to_show(items: &[u32], checked: &[bool], shown: Option<&[usize]>) -> Vec<u32> {
    items
        .iter()
        .enumerate()
        .filter(|(i, _)| checked.get(*i).copied().unwrap_or(false) && shown.is_none_or(|s| s.contains(i)))
        .map(|(_, item)| *item)
        .collect()
}

/// The sentence the Sort dialog shows under its options.
pub fn sort_summary(field: &str, order: SortOrder, by_value: Option<&str>) -> String {
    let direction = match order {
        SortOrder::Manual => {
            return t(
                "Drag items of the {} field to display them in any order",
                "Déplacer des éléments du champ {} pour les afficher dans n'importe quel ordre",
            )
            .replace("{}", field);
        }
        SortOrder::Ascending => t("in ascending order", "par ordre croissant"),
        SortOrder::Descending => t("in descending order", "par ordre décroissant"),
    };
    match by_value {
        Some(value) => t("Sort {0} by {1} {2}", "Trier {0} par {1} {2}")
            .replace("{0}", field)
            .replace("{1}", value)
            .replace("{2}", direction),
        None => t("Sort {0} {2}", "Trier {0} {2}")
            .replace("{0}", field)
            .replace("{2}", direction),
    }
}

pub fn top10_by_label(by: Top10By) -> &'static str {
    match by {
        Top10By::Items => t("Items", "Éléments"),
        Top10By::Percent => t("Percent", "Pourcentage"),
        Top10By::Sum => t("Sum", "Somme"),
    }
}

// ----------------------------------------------------------- show values as

/// The calculations of "Show Values As", in the order of Excel's list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShowKind {
    Normal,
    PercentOfTotal,
    PercentOfColumn,
    PercentOfRow,
    PercentOf,
    PercentOfParentRow,
    PercentOfParentColumn,
    PercentOfParent,
    Difference,
    PercentDifference,
    RunningTotal,
    PercentRunningTotal,
    RankAscending,
    RankDescending,
    Index,
}

impl ShowKind {
    pub const ALL: [ShowKind; 15] = [
        ShowKind::Normal,
        ShowKind::PercentOfTotal,
        ShowKind::PercentOfColumn,
        ShowKind::PercentOfRow,
        ShowKind::PercentOf,
        ShowKind::PercentOfParentRow,
        ShowKind::PercentOfParentColumn,
        ShowKind::PercentOfParent,
        ShowKind::Difference,
        ShowKind::PercentDifference,
        ShowKind::RunningTotal,
        ShowKind::PercentRunningTotal,
        ShowKind::RankAscending,
        ShowKind::RankDescending,
        ShowKind::Index,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ShowKind::Normal => t("No Calculation", "Aucun calcul"),
            ShowKind::PercentOfTotal => t("% of Grand Total", "% du total général"),
            ShowKind::PercentOfColumn => t("% of Column Total", "% du total de la colonne"),
            ShowKind::PercentOfRow => t("% of Row Total", "% du total de la ligne"),
            ShowKind::PercentOf => t("% Of", "% de"),
            ShowKind::PercentOfParentRow => t("% of Parent Row Total", "% du total de la ligne parente"),
            ShowKind::PercentOfParentColumn => t("% of Parent Column Total", "% du total de la colonne parente"),
            ShowKind::PercentOfParent => t("% of Parent Total", "% du total du parent"),
            ShowKind::Difference => t("Difference From", "Différence par rapport"),
            ShowKind::PercentDifference => t("% Difference From", "Différence en % par rapport"),
            ShowKind::RunningTotal => t("Running Total In", "Résultat cumulé par"),
            ShowKind::PercentRunningTotal => t("% Running Total In", "% résultat cumulé dans"),
            ShowKind::RankAscending => t("Rank Smallest to Largest", "Rang - Du plus petit au plus grand"),
            ShowKind::RankDescending => t("Rank Largest to Smallest", "Rang - Du plus grand au plus petit"),
            ShowKind::Index => t("Index", "Index"),
        }
    }

    /// The calculation runs along a base field.
    pub fn needs_field(self) -> bool {
        !matches!(
            self,
            ShowKind::Normal
                | ShowKind::PercentOfTotal
                | ShowKind::PercentOfColumn
                | ShowKind::PercentOfRow
                | ShowKind::PercentOfParentRow
                | ShowKind::PercentOfParentColumn
                | ShowKind::Index
        )
    }

    /// The calculation compares with a base item.
    pub fn needs_item(self) -> bool {
        matches!(self, ShowKind::PercentOf | ShowKind::Difference | ShowKind::PercentDifference)
    }

    pub fn of(show: &ShowAs) -> (ShowKind, Option<usize>, Option<BaseItem>) {
        match *show {
            ShowAs::Normal => (ShowKind::Normal, None, None),
            ShowAs::PercentOfTotal => (ShowKind::PercentOfTotal, None, None),
            ShowAs::PercentOfColumn => (ShowKind::PercentOfColumn, None, None),
            ShowAs::PercentOfRow => (ShowKind::PercentOfRow, None, None),
            ShowAs::PercentOf { field, item } => (ShowKind::PercentOf, Some(field), Some(item)),
            ShowAs::PercentOfParentRow => (ShowKind::PercentOfParentRow, None, None),
            ShowAs::PercentOfParentColumn => (ShowKind::PercentOfParentColumn, None, None),
            ShowAs::PercentOfParent { field } => (ShowKind::PercentOfParent, Some(field), None),
            ShowAs::Difference { field, item } => (ShowKind::Difference, Some(field), Some(item)),
            ShowAs::PercentDifference { field, item } => (ShowKind::PercentDifference, Some(field), Some(item)),
            ShowAs::RunningTotal { field } => (ShowKind::RunningTotal, Some(field), None),
            ShowAs::PercentRunningTotal { field } => (ShowKind::PercentRunningTotal, Some(field), None),
            ShowAs::RankAscending { field } => (ShowKind::RankAscending, Some(field), None),
            ShowAs::RankDescending { field } => (ShowKind::RankDescending, Some(field), None),
            ShowAs::Index => (ShowKind::Index, None, None),
        }
    }

    /// The calculation with its base field and item; `None` when a base
    /// it needs is missing.
    pub fn build(self, field: Option<usize>, item: Option<BaseItem>) -> Option<ShowAs> {
        Some(match self {
            ShowKind::Normal => ShowAs::Normal,
            ShowKind::PercentOfTotal => ShowAs::PercentOfTotal,
            ShowKind::PercentOfColumn => ShowAs::PercentOfColumn,
            ShowKind::PercentOfRow => ShowAs::PercentOfRow,
            ShowKind::PercentOf => ShowAs::PercentOf {
                field: field?,
                item: item?,
            },
            ShowKind::PercentOfParentRow => ShowAs::PercentOfParentRow,
            ShowKind::PercentOfParentColumn => ShowAs::PercentOfParentColumn,
            ShowKind::PercentOfParent => ShowAs::PercentOfParent { field: field? },
            ShowKind::Difference => ShowAs::Difference {
                field: field?,
                item: item?,
            },
            ShowKind::PercentDifference => ShowAs::PercentDifference {
                field: field?,
                item: item?,
            },
            ShowKind::RunningTotal => ShowAs::RunningTotal { field: field? },
            ShowKind::PercentRunningTotal => ShowAs::PercentRunningTotal { field: field? },
            ShowKind::RankAscending => ShowAs::RankAscending { field: field? },
            ShowKind::RankDescending => ShowAs::RankDescending { field: field? },
            ShowKind::Index => ShowAs::Index,
        })
    }
}

/// The fields a calculation can run along: those of the rows and columns.
pub fn base_fields(p: &PivotTable) -> Vec<usize> {
    p.row_fields.iter().chain(&p.col_fields).filter_map(|a| a.field()).collect()
}

/// The functions of the "Summarize Values By" menu, ahead of "More
/// Options...".
pub const QUICK_FUNCTIONS: [Aggregation; 6] = [
    Aggregation::Sum,
    Aggregation::Count,
    Aggregation::Average,
    Aggregation::Max,
    Aggregation::Min,
    Aggregation::Product,
];

// ------------------------------------------------------------------ the sheet

/// What the commands of the "Active Field" group act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// A field of the rows, the columns or the filters.
    Field(usize),
    /// A value field (index into `data_fields`).
    Data(usize),
}

/// The field a cell of a PivotTable stands for.
pub fn target_of(cell: &PivotCell) -> Option<Target> {
    match cell.part {
        PivotPart::Value | PivotPart::DataCaption => cell.data_field.map(Target::Data),
        PivotPart::RowHeaderCaption | PivotPart::ColumnHeaderCaption => cell.fields.first().copied().map(Target::Field),
        PivotPart::RowGrandTotalLabel | PivotPart::ColumnGrandTotalLabel => cell.data_field.map(Target::Data),
        PivotPart::Placeholder | PivotPart::Blank => None,
        _ => cell
            .field
            .map(Target::Field)
            .or(cell.data_field.map(Target::Data)),
    }
}

/// The name the "Active Field" box shows.
pub fn target_name(p: &PivotTable, target: Target) -> String {
    match target {
        Target::Field(f) => p.field_caption(f).to_string(),
        Target::Data(d) => p.data_fields.get(d).map(|d| d.name.clone()).unwrap_or_default(),
    }
}

/// The items the selection picks out for "Group Selection": the labels of
/// one field of one PivotTable. `None` when the selection holds no label,
/// or labels of several fields.
pub fn selected_items(wb: &Workbook, sheet: usize, ranges: &[Rect]) -> Option<(u32, usize, Vec<u32>)> {
    let mut found: Option<(u32, usize)> = None;
    let mut items: Vec<u32> = Vec::new();
    for range in ranges {
        for p in wb.sheets.get(sheet)?.pivots.iter() {
            let Some(part) = p.location.and_then(|body| body.intersection(range)) else {
                continue;
            };
            // Labels only live in the header rows and the label columns.
            if part.rows() as u64 * part.cols() as u64 > 100_000 {
                return None;
            }
            for row in part.r0..=part.r1 {
                for col in part.c0..=part.c1 {
                    let Some(cell) = p.cell(row, col) else {
                        continue;
                    };
                    if !matches!(cell.part, PivotPart::RowLabel | PivotPart::ColumnLabel) {
                        continue;
                    }
                    let (Some(field), Some(item)) = (cell.field, cell.item) else {
                        continue;
                    };
                    match found {
                        None => found = Some((p.id, field)),
                        Some(other) if other != (p.id, field) => return None,
                        _ => {}
                    }
                    if !items.contains(&item) {
                        items.push(item);
                    }
                }
            }
        }
    }
    found.map(|(id, field)| (id, field, items))
}

/// What "Select" can select.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectWhat {
    /// The report and its filter cells.
    Entire,
    /// The report: "Labels and Values".
    Body,
    Values,
    Labels,
}

/// The cells "Select" selects in a PivotTable.
pub fn select_rect(p: &PivotTable, what: SelectWhat) -> Option<Rect> {
    let body = p.location?;
    let (top, left) = (p.report.first_data_row(), p.report.first_data_col());
    match what {
        SelectWhat::Entire => Some(p.page_location.map_or(body, |pages| pages.union(&body))),
        SelectWhat::Body => Some(body),
        SelectWhat::Values => {
            (body.r0 + top <= body.r1 && body.c0 + left <= body.c1).then(|| Rect::new(body.r0 + top, body.c0 + left, body.r1, body.c1))
        }
        SelectWhat::Labels => {
            (left > 0 && body.r0 + top <= body.r1).then(|| Rect::new(body.r0 + top, body.c0, body.r1, body.c0 + left - 1))
        }
    }
}

/// Room a filter button takes at the right of its cell, in pixels at 100 %.
pub const FILTER_BUTTON: f32 = 17.0;
/// One step of indentation, as the grid draws it.
const INDENT: f32 = 9.0;

/// A box on the grid: x0, y0, x1, y1.
pub type Box4 = (f32, f32, f32, f32);

/// Where the drop-down button of a header or filter cell is drawn: at the
/// bottom right of the cell, 15 pixels wide at 100 %, smaller in a cell
/// that is not that high. `None` in a cell too small to show one.
pub fn filter_button_box((x0, y0, x1, y1): Box4, zoom: f32) -> Option<Box4> {
    if x1 - x0 < 6.0 || y1 - y0 < 6.0 {
        return None;
    }
    let size = (15.0 * zoom).round().min(y1 - y0 - 3.0).min(x1 - x0 - 3.0).max(5.0);
    Some((x1 - 2.0 - size, y1 - 2.0 - size, x1 - 2.0, y1 - 2.0))
}

/// Where the "+" / "-" box of a label is drawn: at the left of the cell,
/// after the indentation of the label's level, centred on the row. The
/// engine makes room for it by indenting the text one step more. `None`
/// when the cell cannot hold it.
pub fn drill_button_box((x0, y0, x1, y1): Box4, indent: u8, zoom: f32) -> Option<Box4> {
    if x1 - x0 < 6.0 || y1 - y0 < 6.0 {
        return None;
    }
    let size = (9.0 * zoom).round().max(5.0).min(y1 - y0 - 2.0);
    let left = (x0 + (2.0 + indent as f32 * INDENT) * zoom).round();
    let top = (y0 + (y1 - y0 - size) / 2.0).round();
    (left + size <= x1).then_some((left, top, left + size, top + size))
}

/// The entry of an area box a field dragged to height `y` (from the top of
/// the first entry, scrolling included) would go before: the one whose
/// middle the pointer is above, or `count` below the last one.
pub fn drop_slot(y: f32, entry_height: f32, count: usize) -> usize {
    ((y / entry_height + 0.5).floor().max(0.0) as usize).min(count)
}
/// More rows than this are not measured (the first ones give the widths).
const FIT_ROWS: u32 = 3000;

/// The widths Excel's "Autofit column widths on update" gives the columns
/// of a PivotTable: (column, pixels at 100 %). `measure` gives the width of
/// a text in the font of a style. Columns holding nothing are left alone.
pub fn fit_widths(
    wb: &Workbook,
    sheet: usize,
    p: &PivotTable,
    mut measure: impl FnMut(&str, &Style) -> f32,
) -> Vec<(u32, f32)> {
    let mut widths: Vec<(u32, f32)> = Vec::new();
    if p.report.size() == (0, 0) || p.cell(p.anchor.0, p.anchor.1).is_some_and(|c| c.part == PivotPart::Placeholder) {
        return widths;
    }
    let (top, left) = (p.report.first_data_row(), p.report.first_data_col());
    for (rect, is_body) in [(p.location, true), (p.page_location, false)] {
        let Some(rect) = rect else {
            continue;
        };
        for col in rect.c0..=rect.c1 {
            let mut widest: f32 = 0.0;
            for row in rect.r0..=rect.r1.min(rect.r0 + FIT_ROWS) {
                let text = wb.display_text(sheet, row, col).text;
                if text.is_empty() {
                    continue;
                }
                let display = wb.display(sheet, row, col);
                let mut width = measure(&text, &display.style) + display.style.indent as f32 * INDENT;
                // Only headers, labels and filter cells carry a drop-down.
                let may_have_button = !is_body || row - rect.r0 < top || col - rect.c0 < left;
                if may_have_button
                    && matches!(
                        p.cell(row, col).and_then(|c| c.button),
                        Some(PivotButton::Filter { .. })
                    )
                {
                    width += FILTER_BUTTON;
                }
                widest = widest.max(width);
            }
            if widest > 0.0 {
                let width = (widest + INDENT).ceil().clamp(12.0, 1800.0);
                match widths.iter_mut().find(|(c, _)| *c == col) {
                    Some((_, w)) => *w = w.max(width),
                    None => widths.push((col, width)),
                }
            }
        }
    }
    widths
}

// ------------------------------------------------------------ what a cell is

/// The field "Expand Field" and "Collapse Field" act on: the one of a
/// label that has a field below it on its axis.
pub fn drill_field(p: &PivotTable, cell: &PivotCell) -> Option<usize> {
    let Target::Field(field) = target_of(cell)? else {
        return None;
    };
    let below = |axis: &[AxisField]| {
        axis.iter()
            .position(|a| *a == AxisField::Field(field))
            .is_some_and(|at| axis[at + 1..].iter().any(|a| a.field().is_some()))
    };
    (below(&p.row_fields) || below(&p.col_fields)).then_some(field)
}

/// The field of a cell when it groups by interval or by period: a number
/// field, a date field, or one of the periods of a date field.
pub fn range_group_field(p: &PivotTable, cell: &PivotCell) -> Option<usize> {
    let Target::Field(field) = target_of(cell)? else {
        return None;
    };
    let by_hand = matches!(
        p.cache.fields.get(field)?.group,
        Some(FieldGroup {
            kind: GroupKind::Discrete,
            ..
        })
    );
    (!by_hand && p.cache.is_numeric(p.cache.database_field(field))).then_some(field)
}

/// The grouped field of a cell, and the group to dissolve when the cell is
/// the label of a hand-made group.
pub fn ungroup_target(p: &PivotTable, cell: &PivotCell) -> Option<(usize, Option<u32>)> {
    let Target::Field(field) = target_of(cell)? else {
        return None;
    };
    match &p.cache.fields.get(field)?.group {
        Some(group) => Some((field, cell.item.filter(|_| group.kind == GroupKind::Discrete))),
        // A date field under its periods.
        None => (!p.cache.group_children(field).is_empty()).then_some((field, None)),
    }
}

/// The area a field sits in and its position there.
pub fn place_of(p: &PivotTable, field: usize) -> Option<(Area, usize)> {
    let at = |axis: &[AxisField]| axis.iter().position(|a| *a == AxisField::Field(field));
    match p.axis_of(field)? {
        Area::Rows => at(&p.row_fields).map(|i| (Area::Rows, i)),
        Area::Columns => at(&p.col_fields).map(|i| (Area::Columns, i)),
        area => p.page_fields.iter().position(|pf| pf.field == field).map(|i| (area, i)),
    }
}

/// How the two sort entries of a field are worded: A to Z for text,
/// smallest to largest for numbers, oldest to newest for dates.
pub fn sort_labels(p: &PivotTable, field: usize) -> (&'static str, &'static str) {
    let source = p.cache.database_field(field);
    let grouped = p.cache.fields.get(field).is_some_and(|f| f.group.is_some());
    if grouped || !p.cache.is_numeric(source) {
        (t("Sort A to Z", "Trier de A à Z"), t("Sort Z to A", "Trier de Z à A"))
    } else if p.cache.is_date(source) {
        (
            t("Sort Oldest to Newest", "Trier du plus ancien au plus récent"),
            t("Sort Newest to Oldest", "Trier du plus récent au plus ancien"),
        )
    } else {
        (
            t("Sort Smallest to Largest", "Trier du plus petit au plus grand"),
            t("Sort Largest to Smallest", "Trier du plus grand au plus petit"),
        )
    }
}

// ------------------------------------------------------- changes to a field

/// Switch the subtotals of a field on (automatic) or off.
pub fn toggle_subtotal(p: &mut PivotTable, field: usize) {
    if let Some(f) = p.fields.get_mut(field) {
        f.subtotals = match f.subtotals {
            Subtotals::None => Subtotals::Automatic,
            _ => Subtotals::None,
        };
    }
}

/// "Keep Only Selected Items" (`keep`) or "Hide Selected Items": of the
/// items showing, only the given ones stay, or all but them. Hiding every
/// item would empty the report: nothing changes then, as in Excel.
pub fn filter_items(p: &mut PivotTable, field: usize, items: &[u32], keep: bool) {
    let Some(f) = p.fields.get(field) else {
        return;
    };
    let visible: Vec<u32> = f
        .items
        .iter()
        .filter(|it| !it.hidden && items.contains(&it.x) == keep)
        .map(|it| it.x)
        .collect();
    if !visible.is_empty() {
        p.set_visible_items(field, &visible);
    }
}

/// Move an item among the items its field shows. The order on screen
/// becomes the manual order of the field first; hidden items stay where
/// they are.
pub fn move_item(p: &mut PivotTable, field: usize, item: u32, to: MoveTo) {
    let shown = p.items_in_order(field);
    let Some(f) = p.fields.get_mut(field) else {
        return;
    };
    f.items.sort_by_key(|it| shown.iter().position(|x| *x == it.x).unwrap_or(usize::MAX));
    let visible: Vec<u32> = f.items.iter().filter(|it| !it.hidden).map(|it| it.x).collect();
    let Some(at) = visible.iter().position(|x| *x == item) else {
        return;
    };
    // The item takes the place of the visible item it steps to.
    let neighbour = visible[to.target(at, visible.len())];
    let position = f.items.iter().position(|it| it.x == neighbour).unwrap_or(0);
    p.move_item(field, item, position);
}

/// Give a PivotTable the areas and filters of a definition prepared while
/// layout updates were deferred.
pub fn adopt_layout(p: &mut PivotTable, pending: &PivotTable) {
    p.row_fields = pending.row_fields.clone();
    p.col_fields = pending.col_fields.clone();
    p.page_fields = pending.page_fields.clone();
    p.data_fields = pending.data_fields.clone();
    p.filters = pending.filters.clone();
}

/// "Show Detail": put `detail` below the field of a label, showing it
/// under the item that was double-clicked only.
pub fn show_detail(p: &mut PivotTable, field: usize, item: u32, detail: usize, locale: &Locale) {
    let Some((area, index)) = place_of(p, field) else {
        return;
    };
    p.add_field(detail, area, Some(index + 1), locale);
    p.set_field_collapsed(field, true);
    p.set_collapsed(field, item, false);
}

// --------------------------------------------------------- the context menu

/// Where the "Move" menu sends an item or a field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveTo {
    Beginning,
    Up,
    Down,
    End,
}

impl MoveTo {
    /// The position an entry at `index` among `count` moves to.
    pub fn target(self, index: usize, count: usize) -> usize {
        match self {
            MoveTo::Beginning => 0,
            MoveTo::Up => index.saturating_sub(1),
            MoveTo::Down => (index + 1).min(count.saturating_sub(1)),
            MoveTo::End => count.saturating_sub(1),
        }
    }
}

/// What an entry of the context menu does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Act {
    Copy,
    FormatCells,
    NumberFormat(usize),
    Refresh,
    Sort {
        field: usize,
        order: SortOrder,
        by_value: Option<usize>,
    },
    SortOptions(usize),
    ClearFilter(usize),
    KeepSelected,
    HideSelected,
    Top10(usize),
    LabelFilters(usize),
    ValueFilters(usize),
    ToggleSubtotal(usize),
    CollapseItem {
        field: usize,
        item: u32,
        collapsed: bool,
    },
    CollapseField {
        field: usize,
        collapsed: bool,
    },
    Group,
    Ungroup,
    MoveItem {
        field: usize,
        item: u32,
        to: MoveTo,
    },
    MoveField {
        field: usize,
        to: MoveTo,
    },
    MoveFieldTo {
        field: usize,
        area: Area,
    },
    Remove(Target),
    Function {
        data: usize,
        function: Aggregation,
    },
    ShowAs {
        data: usize,
        kind: ShowKind,
    },
    ShowDetails,
    FieldSettings(usize),
    /// Value Field Settings, on one of its two tabs.
    ValueSettings {
        data: usize,
        tab: usize,
    },
    RemoveGrandTotal,
    Options,
    FieldList,
}

/// An entry of the context menu of a PivotTable cell.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub label: String,
    pub act: Option<Act>,
    pub enabled: bool,
    pub checked: bool,
    pub separator: bool,
    pub children: Vec<Item>,
}

impl Item {
    fn new(label: impl Into<String>, act: Act) -> Item {
        Item {
            label: label.into(),
            act: Some(act),
            enabled: true,
            checked: false,
            separator: false,
            children: Vec::new(),
        }
    }

    /// An entry named after something: `pattern` holds `{}`.
    fn named(pattern: &'static str, name: &str, act: Act) -> Item {
        Item::new(pattern.replace("{}", name), act)
    }

    fn separator() -> Item {
        Item {
            label: String::new(),
            act: None,
            enabled: true,
            checked: false,
            separator: true,
            children: Vec::new(),
        }
    }

    fn submenu(label: &'static str, children: Vec<Item>) -> Item {
        Item {
            label: label.to_string(),
            act: None,
            enabled: !children.is_empty(),
            checked: false,
            separator: false,
            children,
        }
    }

    fn enabled(mut self, on: bool) -> Item {
        self.enabled = on;
        self
    }

    fn checked(mut self, on: bool) -> Item {
        self.checked = on;
        self
    }
}

/// Excel's context menu for a cell of a PivotTable. `ranges`: the
/// selection; `list_shown`: the field list is showing.
pub fn context_menu(wb: &Workbook, sheet: usize, row: u32, col: u32, ranges: &[Rect], list_shown: bool) -> Vec<Item> {
    let Some(p) = wb.pivot_at(sheet, row, col) else {
        return Vec::new();
    };
    let Some(cell) = p.cell(row, col) else {
        return Vec::new();
    };
    let locale = wb.locale;
    let mut menu = vec![
        Item::new(t("Copy", "Copier"), Act::Copy),
        Item::new(t("Format Cells...", "Format de cellule..."), Act::FormatCells),
    ];
    let target = target_of(&cell);
    if let (PivotPart::Value, Some(Target::Data(data))) = (cell.part, target) {
        menu.push(Item::new(t("Number Format...", "Format de nombre..."), Act::NumberFormat(data)));
    }
    menu.push(Item::new(t("Refresh", "Actualiser"), Act::Refresh));

    match target {
        Some(Target::Data(data)) => {
            let name = p.data_fields.get(data).map(|d| d.name.as_str()).unwrap_or_default();
            // A value sorts the items of its row (else of its column) by
            // the totals of its value field.
            let sorted = cell.row_items.last().or(cell.col_items.last()).map(|(field, _)| *field);
            if let (PivotPart::Value, Some(field)) = (cell.part, sorted) {
                let sort = |order| Act::Sort {
                    field,
                    order,
                    by_value: Some(data),
                };
                menu.push(Item::separator());
                menu.push(Item::submenu(
                    t("Sort", "Trier"),
                    vec![
                        Item::new(
                            t("Sort Smallest to Largest", "Trier du plus petit au plus grand"),
                            sort(SortOrder::Ascending),
                        ),
                        Item::new(
                            t("Sort Largest to Smallest", "Trier du plus grand au plus petit"),
                            sort(SortOrder::Descending),
                        ),
                        Item::separator(),
                        Item::new(t("More Sort Options...", "Autres options de tri..."), Act::SortOptions(field)),
                    ],
                ));
            }
            menu.push(Item::separator());
            menu.push(Item::named(
                t("Remove \"{}\"", "Supprimer « {} »"),
                name,
                Act::Remove(Target::Data(data)),
            ));
            menu.push(Item::separator());
            let function = p.data_fields.get(data).map(|d| d.function);
            let mut functions: Vec<Item> = QUICK_FUNCTIONS
                .iter()
                .map(|f| {
                    Item::new(f.label(locale), Act::Function { data, function: *f }).checked(function == Some(*f))
                })
                .collect();
            functions.push(Item::separator());
            functions.push(Item::new(
                t("More Options...", "Autres options..."),
                Act::ValueSettings { data, tab: 0 },
            ));
            menu.push(Item::submenu(t("Summarize Values By", "Synthèse des valeurs par"), functions));
            let shown = p.data_fields.get(data).map(|d| ShowKind::of(&d.show_as).0);
            let has_base = !base_fields(p).is_empty();
            let mut kinds: Vec<Item> = ShowKind::ALL
                .iter()
                .map(|kind| {
                    Item::new(kind.label(), Act::ShowAs { data, kind: *kind })
                        .checked(shown == Some(*kind))
                        .enabled(has_base || !kind.needs_field())
                })
                .collect();
            kinds.insert(1, Item::separator());
            kinds.push(Item::separator());
            kinds.push(Item::new(
                t("More Options...", "Autres options..."),
                Act::ValueSettings { data, tab: 1 },
            ));
            menu.push(Item::submenu(t("Show Values As", "Afficher les valeurs"), kinds));
            if cell.part == PivotPart::Value {
                menu.push(Item::separator());
                menu.push(Item::new(t("Show Details", "Afficher les détails"), Act::ShowDetails));
            }
            menu.push(Item::separator());
            menu.push(Item::new(
                t("Value Field Settings...", "Paramètres des champs de valeurs..."),
                Act::ValueSettings { data, tab: 0 },
            ));
        }
        Some(Target::Field(field)) => {
            let caption = p.field_caption(field);
            let place = place_of(p, field);
            let on_axis = matches!(place, Some((Area::Rows | Area::Columns, _)));
            let in_columns = matches!(place, Some((Area::Columns, _)));
            if on_axis {
                let (ascending, descending) = sort_labels(p, field);
                let sort = |order| Act::Sort {
                    field,
                    order,
                    by_value: None,
                };
                menu.push(Item::separator());
                menu.push(Item::submenu(
                    t("Sort", "Trier"),
                    vec![
                        Item::new(ascending, sort(SortOrder::Ascending)),
                        Item::new(descending, sort(SortOrder::Descending)),
                        Item::separator(),
                        Item::new(t("More Sort Options...", "Autres options de tri..."), Act::SortOptions(field)),
                    ],
                ));
                let picked = selected_items(wb, sheet, ranges).is_some_and(|(id, f, _)| id == p.id && f == field);
                menu.push(Item::submenu(
                    t("Filter", "Filtrer"),
                    vec![
                        Item::named(
                            t("Clear Filter From \"{}\"", "Effacer le filtre de « {} »"),
                            caption,
                            Act::ClearFilter(field),
                        )
                        .enabled(p.is_filtered(field)),
                        Item::separator(),
                        Item::new(
                            t("Keep Only Selected Items", "Conserver uniquement les éléments sélectionnés"),
                            Act::KeepSelected,
                        )
                        .enabled(picked),
                        Item::new(t("Hide Selected Items", "Masquer les éléments sélectionnés"), Act::HideSelected)
                            .enabled(picked),
                        Item::separator(),
                        Item::new(t("Top 10...", "10 premiers..."), Act::Top10(field)).enabled(!p.data_fields.is_empty()),
                        Item::new(
                            t("Label Filters...", "Filtres s'appliquant aux étiquettes..."),
                            Act::LabelFilters(field),
                        ),
                        Item::new(
                            t("Value Filters...", "Filtres s'appliquant aux valeurs..."),
                            Act::ValueFilters(field),
                        )
                        .enabled(!p.data_fields.is_empty()),
                    ],
                ));
                menu.push(Item::separator());
                menu.push(
                    Item::named(t("Subtotal \"{}\"", "Sous-total de « {} »"), caption, Act::ToggleSubtotal(field))
                        .checked(p.fields[field].subtotals != Subtotals::None),
                );
            }
            if drill_field(p, &cell).is_some() {
                let collapsed = cell
                    .item
                    .map(|item| p.fields[field].items.iter().any(|it| it.x == item && it.collapsed));
                let one = |collapse: bool| match cell.item {
                    Some(item) => Act::CollapseItem {
                        field,
                        item,
                        collapsed: collapse,
                    },
                    None => Act::CollapseField {
                        field,
                        collapsed: collapse,
                    },
                };
                menu.push(Item::separator());
                menu.push(Item::submenu(
                    t("Expand/Collapse", "Afficher/Masquer"),
                    vec![
                        Item::new(t("Expand", "Développer"), one(false)).enabled(collapsed != Some(false)),
                        Item::new(t("Collapse", "Réduire"), one(true)).enabled(collapsed != Some(true)),
                        Item::separator(),
                        Item::new(
                            t("Expand Entire Field", "Développer le champ entier"),
                            Act::CollapseField {
                                field,
                                collapsed: false,
                            },
                        ),
                        Item::new(
                            t("Collapse Entire Field", "Réduire le champ entier"),
                            Act::CollapseField { field, collapsed: true },
                        ),
                    ],
                ));
            }
            if on_axis {
                let several = selected_items(wb, sheet, ranges).is_some_and(|(_, _, items)| items.len() > 1);
                menu.push(Item::separator());
                menu.push(
                    Item::new(t("Group...", "Grouper..."), Act::Group)
                        .enabled(several || range_group_field(p, &cell).is_some()),
                );
                menu.push(
                    Item::new(t("Ungroup...", "Dissocier le groupe..."), Act::Ungroup)
                        .enabled(ungroup_target(p, &cell).is_some()),
                );
            }
            if let Some((area, index)) = place {
                let count = area_entries(p, area, locale).len();
                let mut moves: Vec<Item> = Vec::new();
                let (back, forth) = if in_columns {
                    (
                        t("Move \"{}\" Left", "Déplacer « {} » vers la gauche"),
                        t("Move \"{}\" Right", "Déplacer « {} » vers la droite"),
                    )
                } else {
                    (
                        t("Move \"{}\" Up", "Déplacer « {} » vers le haut"),
                        t("Move \"{}\" Down", "Déplacer « {} » vers le bas"),
                    )
                };
                let (to_start, to_end) = (
                    t("Move \"{}\" to Beginning", "Déplacer « {} » au début"),
                    t("Move \"{}\" to End", "Déplacer « {} » à la fin"),
                );
                // The item of a label moves among the items shown.
                if let (true, Some(item)) = (is_label(cell.part), cell.item) {
                    let shown: Vec<u32> = p
                        .items_in_order(field)
                        .into_iter()
                        .filter(|x| !p.fields[field].items.iter().any(|it| it.x == *x && it.hidden))
                        .collect();
                    let at = shown.iter().position(|x| *x == item).unwrap_or(0);
                    let text = p.item_text(field, item, locale);
                    let step = |pattern: &'static str, to: MoveTo, possible: bool| {
                        Item::named(pattern, &text, Act::MoveItem { field, item, to }).enabled(possible)
                    };
                    moves.push(step(to_start, MoveTo::Beginning, at > 0));
                    moves.push(step(back, MoveTo::Up, at > 0));
                    moves.push(step(forth, MoveTo::Down, at + 1 < shown.len()));
                    moves.push(step(to_end, MoveTo::End, at + 1 < shown.len()));
                    moves.push(Item::separator());
                }
                let step = |pattern: &'static str, to: MoveTo, possible: bool| {
                    Item::named(pattern, caption, Act::MoveField { field, to }).enabled(possible)
                };
                moves.push(step(to_start, MoveTo::Beginning, index > 0));
                moves.push(step(back, MoveTo::Up, index > 0));
                moves.push(step(forth, MoveTo::Down, index + 1 < count));
                moves.push(step(to_end, MoveTo::End, index + 1 < count));
                moves.push(Item::separator());
                if area != Area::Rows {
                    moves.push(Item::named(
                        t("Move \"{}\" to Rows", "Déplacer « {} » vers les lignes"),
                        caption,
                        Act::MoveFieldTo { field, area: Area::Rows },
                    ));
                }
                if area != Area::Columns {
                    moves.push(Item::named(
                        t("Move \"{}\" to Columns", "Déplacer « {} » vers les colonnes"),
                        caption,
                        Act::MoveFieldTo {
                            field,
                            area: Area::Columns,
                        },
                    ));
                }
                menu.push(Item::separator());
                menu.push(Item::submenu(t("Move", "Déplacer"), moves));
            }
            menu.push(Item::separator());
            menu.push(Item::named(
                t("Remove \"{}\"", "Supprimer « {} »"),
                caption,
                Act::Remove(Target::Field(field)),
            ));
            menu.push(Item::separator());
            menu.push(Item::new(t("Field Settings...", "Paramètres de champ..."), Act::FieldSettings(field)));
        }
        None => {
            if matches!(cell.part, PivotPart::RowGrandTotalLabel | PivotPart::ColumnGrandTotalLabel) {
                menu.push(Item::separator());
                menu.push(Item::new(
                    t("Remove Grand Total", "Supprimer le total général"),
                    Act::RemoveGrandTotal,
                ));
            }
            menu.push(Item::separator());
        }
    }
    menu.push(Item::new(
        t("PivotTable Options...", "Options du tableau croisé dynamique..."),
        Act::Options,
    ));
    menu.push(Item::new(
        if list_shown {
            t("Hide Field List", "Masquer la liste de champs")
        } else {
            t("Show Field List", "Afficher la liste de champs")
        },
        Act::FieldList,
    ));
    menu
}

fn is_label(part: PivotPart) -> bool {
    matches!(part, PivotPart::RowLabel | PivotPart::ColumnLabel)
}

// ------------------------------------------------- sources and destinations

/// What the "Table/Range" box of the Create PivotTable dialog proposes:
/// the table the active cell is in, else the selected range, else the
/// block of data around the active cell. Empty when there is no data.
pub fn default_source(wb: &Workbook, sheet: usize, active: (u32, u32), selection: Rect) -> String {
    if let Some(table) = wb.table_at(sheet, active.0, active.1) {
        return table.name.clone();
    }
    let range = if selection.is_single() {
        wb.current_region(sheet, active.0, active.1)
    } else {
        wb.clamp_to_used(sheet, selection)
    };
    if range.is_single() {
        return String::new();
    }
    PivotSource::Range {
        sheet: wb.sheets[sheet].name.clone(),
        range,
    }
    .to_text()
}

/// The cell typed in a "Location" box: (sheet, row, column). A range
/// stands for its first cell.
pub fn parse_destination(text: &str, sheet: usize, wb: &Workbook) -> Option<(usize, u32, u32)> {
    let text = text.trim().trim_start_matches('=');
    match opensheet_engine::parser::parse_formula(text, wb.locale).ok()? {
        Expr::Ref(r) => {
            let sheet = match r.sheet.as_deref() {
                Some(name) => wb.sheet_index(name)?,
                None => sheet,
            };
            let rect = r.rect();
            Some((sheet, rect.r0, rect.c0))
        }
        _ => None,
    }
}

/// A cell as a "Location" box shows it: `Sheet1!$A$3`.
pub fn destination_text(wb: &Workbook, sheet: usize, row: u32, col: u32) -> String {
    PivotSource::Range {
        sheet: wb.sheets[sheet].name.clone(),
        range: Rect::cell(row, col),
    }
    .to_text()
}

/// The form of the report as "Report Layout" shows it: the form all the
/// fields share, if they do.
pub fn report_layout(p: &PivotTable) -> Option<FieldLayout> {
    let first = p.fields.first()?.layout;
    p.fields.iter().all(|f| f.layout == first).then_some(first)
}

/// A number typed in a dialog, in the workbook's language.
pub fn parse_number(text: &str, locale: &Locale) -> Option<f64> {
    opensheet_engine::input::parse_numeric_text(text, locale)
}

/// A date or a time typed in the Grouping dialog.
pub fn parse_date(text: &str, locale: &Locale) -> Option<f64> {
    opensheet_engine::input::parse_date_time(text, locale)
        .map(|(serial, _)| serial)
        .or_else(|| locale.parse_number(text.trim()))
}

/// A number as the fields of a dialog show it.
pub fn number_text(n: f64, locale: &Locale) -> String {
    opensheet_engine::format::format_value(&Value::Number(n), "General", locale).text
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::pivot::{PivotDestination, PivotSource};

    const DATA: &[[&str; 4]] = &[
        ["Region", "Product", "Date", "Sales"],
        ["East", "Apples", "2023-01-05", "100"],
        ["East", "Pears", "2023-06-12", "150"],
        ["West", "Apples", "2024-02-02", "200"],
        ["West", "Pears", "2024-11-18", "50"],
    ];
    const REGION: usize = 0;
    const PRODUCT: usize = 1;
    const DATE: usize = 2;
    const SALES: usize = 3;

    /// A workbook with the data on its first sheet and an empty PivotTable
    /// on a second one.
    fn workbook() -> (Workbook, u32) {
        let mut wb = Workbook::new(Locale::en());
        for (r, row) in DATA.iter().enumerate() {
            for (c, text) in row.iter().enumerate() {
                wb.set_cell_text(0, r as u32, c as u32, text).unwrap();
            }
        }
        wb.commit("data");
        let source = PivotSource::Range {
            sheet: wb.sheets[0].name.clone(),
            range: Rect::new(0, 0, DATA.len() as u32 - 1, 3),
        };
        let id = wb.pivot_create(source, PivotDestination::NewSheet { index: 1 }, false).unwrap();
        wb.commit("pivot");
        (wb, id)
    }

    fn run(wb: &mut Workbook, id: u32, op: PaneOp) {
        run_op(wb, id, op, false).unwrap();
        wb.commit("pane");
    }

    fn labels(wb: &Workbook, id: u32, area: Area) -> Vec<String> {
        let (_, p) = wb.pivot(id).unwrap();
        area_entries(p, area, wb.locale).into_iter().map(|e| e.label).collect()
    }

    #[test]
    fn ticking_a_field_puts_it_where_excel_does() {
        let (mut wb, id) = workbook();
        run(&mut wb, id, PaneOp::Check(REGION));
        run(&mut wb, id, PaneOp::Check(SALES));
        assert_eq!(labels(&wb, id, Area::Rows), ["Region"]);
        assert_eq!(labels(&wb, id, Area::Values), ["Sum of Sales"]);
        // A date is grouped by the periods Excel picks, with itself below.
        run(&mut wb, id, PaneOp::Check(DATE));
        assert_eq!(
            labels(&wb, id, Area::Rows),
            ["Region", "Years (Date)", "Quarters (Date)", "Months (Date)", "Date"]
        );
        // Unticking takes a field out of every area.
        run(&mut wb, id, PaneOp::Check(SALES));
        assert_eq!(labels(&wb, id, Area::Values), ["Sum of Sales", "Sum of Sales2"]);
        assert_eq!(labels(&wb, id, Area::Columns), ["Σ Values"]);
        run(&mut wb, id, PaneOp::Uncheck(SALES));
        assert!(labels(&wb, id, Area::Values).is_empty());
        assert!(labels(&wb, id, Area::Columns).is_empty());
    }

    #[test]
    fn drops_become_moves_additions_and_removals() {
        let (mut wb, id) = workbook();
        run(&mut wb, id, PaneOp::Check(REGION));
        run(&mut wb, id, PaneOp::Check(PRODUCT));
        run(&mut wb, id, PaneOp::Check(SALES));
        let locale = wb.locale;
        let drop = |wb: &Workbook, source, target| drop_op(wb.pivot(id).unwrap().1, source, target, locale);

        // Within an area: dropped before itself or just after, nothing moves.
        let region = DragSource::Area(Area::Rows, 0);
        assert_eq!(drop(&wb, region, Some((Area::Rows, 0))), None);
        assert_eq!(drop(&wb, region, Some((Area::Rows, 1))), None);
        let moved = drop(&wb, region, Some((Area::Rows, 2))).unwrap();
        assert_eq!(
            moved,
            PaneOp::Move {
                from: Area::Rows,
                index: 0,
                to: Area::Rows,
                position: 1
            }
        );
        run(&mut wb, id, moved);
        assert_eq!(labels(&wb, id, Area::Rows), ["Product", "Region"]);

        // To another area.
        let to_columns = drop(&wb, DragSource::Area(Area::Rows, 1), Some((Area::Columns, 0))).unwrap();
        run(&mut wb, id, to_columns);
        assert_eq!(labels(&wb, id, Area::Rows), ["Product"]);
        assert_eq!(labels(&wb, id, Area::Columns), ["Region"]);

        // From the list: a field already in the area moves, another is added.
        assert_eq!(drop(&wb, DragSource::List(PRODUCT), Some((Area::Rows, 1))), None);
        let add = drop(&wb, DragSource::List(SALES), Some((Area::Values, 1))).unwrap();
        assert_eq!(
            add,
            PaneOp::Add {
                field: SALES,
                area: Area::Values,
                position: Some(1)
            }
        );
        run(&mut wb, id, add);
        assert_eq!(labels(&wb, id, Area::Columns), ["Region", "Σ Values"]);

        // "Σ Values" goes to the rows, not to the filters.
        let sigma = DragSource::Area(Area::Columns, 1);
        assert_eq!(drop(&wb, sigma, Some((Area::Filters, 0))), None);
        let to_rows = drop(&wb, sigma, Some((Area::Rows, 0))).unwrap();
        run(&mut wb, id, to_rows);
        assert_eq!(labels(&wb, id, Area::Rows), ["Σ Values", "Product"]);

        // Dropped outside the areas: removed; a field of the list stays.
        assert_eq!(drop(&wb, DragSource::List(REGION), None), None);
        let removal = drop(&wb, DragSource::Area(Area::Values, 0), None).unwrap();
        run(&mut wb, id, removal);
        assert_eq!(labels(&wb, id, Area::Values), ["Sum of Sales2"]);
        assert_eq!(labels(&wb, id, Area::Rows), ["Product"]);
    }

    #[test]
    fn deferred_changes_work_on_a_copy() {
        let (wb, id) = workbook();
        let mut pending = wb.pivot(id).unwrap().1.clone();
        for op in [
            PaneOp::Check(REGION),
            PaneOp::Check(SALES),
            PaneOp::Check(SALES),
            PaneOp::Remove {
                area: Area::Columns,
                index: 0,
            },
        ] {
            apply_op(&mut pending, op, wb.locale);
        }
        // Removing "Σ Values" removed both value fields.
        assert!(pending.data_fields.is_empty());
        assert_eq!(pending.row_fields, [AxisField::Field(REGION)]);
        // The workbook has not changed.
        assert!(wb.pivot(id).unwrap().1.row_fields.is_empty());
    }

    #[test]
    fn the_menu_of_an_entry_knows_where_it_can_go() {
        let (mut wb, id) = workbook();
        for op in [PaneOp::Check(REGION), PaneOp::Check(PRODUCT), PaneOp::Check(SALES), PaneOp::Check(SALES)] {
            run(&mut wb, id, op);
        }
        let (_, p) = wb.pivot(id).unwrap();
        let first = entry_moves(p, Area::Rows, 0, wb.locale);
        assert!(!first.up && first.down && first.to_filters && !first.to_rows && first.to_columns && first.to_values);
        let sigma = entry_moves(p, Area::Columns, 0, wb.locale);
        assert!(sigma.sigma && sigma.to_rows && !sigma.to_columns && !sigma.to_filters && !sigma.to_values);
        let value = entry_moves(p, Area::Values, 1, wb.locale);
        assert!(value.value && value.up && !value.down && !value.to_values && value.to_rows);
    }

    #[test]
    fn filter_tests_round_trip() {
        for test in LabelTest::ALL {
            assert_eq!(LabelTest::of(&test.build("a", "b")).0, test);
            assert!(!test.label().is_empty() && test.menu_label().ends_with("..."));
        }
        assert_eq!(LabelTest::MENU.iter().map(|group| group.len()).sum::<usize>(), LabelTest::ALL.len());
        for test in ValueTest::ALL {
            assert_eq!(ValueTest::of(&test.build(1.0, 2.0)).0, test);
        }
        // The bounds of "between" are put in order.
        assert_eq!(ValueTest::Between.build(5.0, 2.0), ValueFilter::Between(2.0, 5.0));
        assert_eq!(
            LabelTest::of(&LabelFilter::Between("a".into(), "m".into())),
            (LabelTest::Between, "a".to_string(), "m".to_string())
        );
    }

    #[test]
    fn show_values_as_round_trips() {
        for kind in ShowKind::ALL {
            let show = kind.build(Some(1), Some(BaseItem::Previous)).unwrap();
            let (back, field, item) = ShowKind::of(&show);
            assert_eq!(back, kind);
            assert_eq!(field.is_some(), kind.needs_field());
            assert_eq!(item.is_some(), kind.needs_item());
            // Without the base it needs, there is no calculation.
            assert_eq!(kind.build(None, None).is_some(), !kind.needs_field());
        }
    }

    #[test]
    fn a_search_replaces_the_selection() {
        let items = [10, 11, 12, 13];
        let checked = [true, true, false, true];
        assert_eq!(items_to_show(&items, &checked, None), [10, 11, 13]);
        assert_eq!(items_to_show(&items, &checked, Some(&[1, 2])), [11]);
    }

    #[test]
    fn sort_summaries() {
        assert_eq!(sort_summary("Region", SortOrder::Ascending, None), "Sort Region in ascending order");
        assert_eq!(
            sort_summary("Region", SortOrder::Descending, Some("Sum of Sales")),
            "Sort Region by Sum of Sales in descending order"
        );
        assert!(sort_summary("Region", SortOrder::Manual, None).starts_with("Drag items of the Region field"));
    }

    #[test]
    fn targets_and_selections() {
        let (mut wb, id) = workbook();
        for op in [PaneOp::Check(REGION), PaneOp::Check(PRODUCT), PaneOp::Check(SALES)] {
            run(&mut wb, id, op);
        }
        // A3 "Row Labels", B3 "Sum of Sales", A4 East, A5 Apples, B5 a value.
        let cell = |r: u32, c: u32| wb.pivot_cell(1, r, c).unwrap();
        assert_eq!(target_of(&cell(2, 0)), Some(Target::Field(REGION)));
        assert_eq!(target_of(&cell(2, 1)), Some(Target::Data(0)));
        assert_eq!(target_of(&cell(3, 0)), Some(Target::Field(REGION)));
        assert_eq!(target_of(&cell(4, 0)), Some(Target::Field(PRODUCT)));
        assert_eq!(target_of(&cell(4, 1)), Some(Target::Data(0)));
        let (_, p) = wb.pivot(id).unwrap();
        assert_eq!(target_name(p, Target::Data(0)), "Sum of Sales");
        assert_eq!(target_name(p, Target::Field(PRODUCT)), "Product");

        // Two products selected: one field, two items.
        let picked = selected_items(&wb, 1, &[Rect::new(4, 0, 5, 1)]).unwrap();
        assert_eq!((picked.0, picked.1, picked.2.len()), (id, PRODUCT, 2));
        // A region and a product: labels of two fields.
        assert_eq!(selected_items(&wb, 1, &[Rect::new(3, 0, 4, 0)]), None);
        // Values only: nothing to group.
        assert_eq!(selected_items(&wb, 1, &[Rect::new(4, 1, 5, 1)]), None);

        assert_eq!(select_rect(p, SelectWhat::Entire), p.location);
        assert_eq!(select_rect(p, SelectWhat::Values), Some(Rect::new(3, 1, 9, 1)));
        assert_eq!(select_rect(p, SelectWhat::Labels), Some(Rect::new(3, 0, 9, 0)));
    }

    #[test]
    fn columns_fit_their_report() {
        let (mut wb, id) = workbook();
        // The placeholder leaves the columns alone.
        let (_, p) = wb.pivot(id).unwrap();
        let by_chars = |text: &str, _: &Style| text.chars().count() as f32 * 7.0;
        assert!(fit_widths(&wb, 1, p, by_chars).is_empty());

        for op in [PaneOp::Check(REGION), PaneOp::Check(PRODUCT), PaneOp::Check(SALES)] {
            run(&mut wb, id, op);
        }
        let (_, p) = wb.pivot(id).unwrap();
        let widths = fit_widths(&wb, 1, p, by_chars);
        // "Row Labels" (10 letters) with its drop-down is narrower than
        // "Grand Total" (11), which is narrower than the products with
        // their two steps of indentation ("Apples": 6 letters + 18).
        let label_col = widths.iter().find(|(c, _)| *c == 0).unwrap().1;
        assert_eq!(label_col, (11.0f32 * 7.0).max(10.0 * 7.0 + FILTER_BUTTON).max(6.0 * 7.0 + 18.0) + 9.0);
        // "Sum of Sales" gives the width of the values.
        assert_eq!(widths.iter().find(|(c, _)| *c == 1).unwrap().1, 12.0 * 7.0 + 9.0);
        assert_eq!(widths.len(), 2);

        // A filter field adds its cells above the report.
        run(
            &mut wb,
            id,
            PaneOp::Add {
                field: DATE,
                area: Area::Filters,
                position: None,
            },
        );
        let (_, p) = wb.pivot(id).unwrap();
        assert!(p.page_location.is_some());
        let widths = fit_widths(&wb, 1, p, by_chars);
        // "(All)" and its drop-down are narrower than "Sum of Sales".
        assert_eq!(widths.iter().find(|(c, _)| *c == 1).unwrap().1, 12.0 * 7.0 + 9.0);
    }

    #[test]
    fn numbers_and_dates_of_the_dialogs() {
        let fr = Locale::by_id("fr");
        assert_eq!(parse_number("12,5", fr), Some(12.5));
        assert_eq!(parse_number("x", fr), None);
        assert_eq!(number_text(12.5, fr), "12,5");
        assert_eq!(parse_date("2024-01-01", Locale::en()), Some(45292.0));
        assert_eq!(parse_date("45292", Locale::en()), Some(45292.0));
    }

    /// The labels of a menu, submenus in brackets, `-` for a separator,
    /// `*` before a ticked entry, `!` before a disabled one.
    fn outline(items: &[Item]) -> String {
        items
            .iter()
            .map(|item| {
                if item.separator {
                    return "-".to_string();
                }
                let mut text = String::new();
                if item.checked {
                    text.push('*');
                }
                if !item.enabled {
                    text.push('!');
                }
                text.push_str(&item.label);
                if !item.children.is_empty() {
                    text.push_str(&format!(" [{}]", outline(&item.children)));
                }
                text
            })
            .collect::<Vec<_>>()
            .join(" | ")
    }

    #[test]
    fn the_context_menu_of_a_label() {
        let (mut wb, id) = workbook();
        for op in [PaneOp::Check(REGION), PaneOp::Check(PRODUCT), PaneOp::Check(SALES)] {
            run(&mut wb, id, op);
        }
        // A4 is "East", the first region, with products below it.
        let menu = context_menu(&wb, 1, 3, 0, &[Rect::cell(3, 0)], true);
        assert_eq!(
            outline(&menu),
            "Copy | Format Cells... | Refresh | - | \
             Sort [Sort A to Z | Sort Z to A | - | More Sort Options...] | \
             Filter [!Clear Filter From \"Region\" | - | Keep Only Selected Items | Hide Selected Items | - | \
             Top 10... | Label Filters... | Value Filters...] | - | \
             *Subtotal \"Region\" | - | \
             Expand/Collapse [!Expand | Collapse | - | Expand Entire Field | Collapse Entire Field] | - | \
             !Group... | !Ungroup... | - | \
             Move [!Move \"East\" to Beginning | !Move \"East\" Up | Move \"East\" Down | Move \"East\" to End | - | \
             !Move \"Region\" to Beginning | !Move \"Region\" Up | Move \"Region\" Down | Move \"Region\" to End | - | \
             Move \"Region\" to Columns] | - | \
             Remove \"Region\" | - | Field Settings... | PivotTable Options... | Hide Field List"
        );
        // Two products selected can be grouped; the innermost field has
        // nothing to expand.
        let menu = context_menu(&wb, 1, 4, 0, &[Rect::new(4, 0, 5, 0)], false);
        let text = outline(&menu);
        assert!(text.contains("| Group... | !Ungroup..."), "{text}");
        assert!(!text.contains("Expand/Collapse"), "{text}");
        assert!(text.ends_with("Show Field List"), "{text}");
        // The header of the labels stands for the first row field.
        let menu = context_menu(&wb, 1, 2, 0, &[Rect::cell(2, 0)], true);
        assert!(outline(&menu).contains("Remove \"Region\""));
        // Outside a PivotTable there is no such menu.
        assert!(context_menu(&wb, 0, 0, 0, &[Rect::cell(0, 0)], true).is_empty());
    }

    #[test]
    fn the_context_menu_of_a_value() {
        let (mut wb, id) = workbook();
        for op in [PaneOp::Check(REGION), PaneOp::Check(SALES)] {
            run(&mut wb, id, op);
        }
        // B4 is the sales of the first region.
        let menu = context_menu(&wb, 1, 3, 1, &[Rect::cell(3, 1)], true);
        assert_eq!(
            outline(&menu),
            "Copy | Format Cells... | Number Format... | Refresh | - | \
             Sort [Sort Smallest to Largest | Sort Largest to Smallest | - | More Sort Options...] | - | \
             Remove \"Sum of Sales\" | - | \
             Summarize Values By [*Sum | Count | Average | Max | Min | Product | - | More Options...] | \
             Show Values As [*No Calculation | - | % of Grand Total | % of Column Total | % of Row Total | % Of | \
             % of Parent Row Total | % of Parent Column Total | % of Parent Total | Difference From | \
             % Difference From | Running Total In | % Running Total In | Rank Smallest to Largest | \
             Rank Largest to Smallest | Index | - | More Options...] | - | \
             Show Details | - | Value Field Settings... | PivotTable Options... | Hide Field List"
        );
        // The caption of the values: the value field without "Show Details".
        let menu = context_menu(&wb, 1, 2, 1, &[Rect::cell(2, 1)], true);
        let text = outline(&menu);
        assert!(text.contains("Value Field Settings...") && !text.contains("Show Details"), "{text}");
        // The grand total label and the empty report.
        let menu = context_menu(&wb, 1, 5, 0, &[Rect::cell(5, 0)], true);
        assert_eq!(
            outline(&menu),
            "Copy | Format Cells... | Refresh | - | Remove Grand Total | - | PivotTable Options... | Hide Field List"
        );
        let (wb, _) = workbook();
        let menu = context_menu(&wb, 1, 4, 1, &[Rect::cell(4, 1)], true);
        assert_eq!(
            outline(&menu),
            "Copy | Format Cells... | Refresh | - | PivotTable Options... | Hide Field List"
        );
    }

    #[test]
    fn what_a_cell_can_expand_group_and_ungroup() {
        let (mut wb, id) = workbook();
        for op in [PaneOp::Check(REGION), PaneOp::Check(PRODUCT), PaneOp::Check(SALES)] {
            run(&mut wb, id, op);
        }
        let (_, p) = wb.pivot(id).unwrap();
        let cell = |r: u32, c: u32| p.cell(r, c).unwrap();
        assert_eq!(drill_field(p, &cell(3, 0)), Some(REGION));
        assert_eq!(drill_field(p, &cell(4, 0)), None);
        assert_eq!(range_group_field(p, &cell(3, 0)), None);
        assert_eq!(ungroup_target(p, &cell(3, 0)), None);
        assert_eq!(place_of(p, PRODUCT), Some((Area::Rows, 1)));
        assert_eq!(place_of(p, DATE), None);
        assert_eq!(sort_labels(p, REGION).0, "Sort A to Z");
        assert_eq!(sort_labels(p, DATE).0, "Sort Oldest to Newest");
        assert_eq!(sort_labels(p, SALES).0, "Sort Smallest to Largest");

        // Two products become "Group1" in a field of their own: its labels
        // can be ungrouped one by one.
        wb.pivot_group_items(id, PRODUCT, &[0, 1], false).unwrap();
        wb.commit("group");
        let (_, p) = wb.pivot(id).unwrap();
        let groups = p.field_index("Product2").unwrap();
        let label = (p.location.unwrap().r0..=p.location.unwrap().r1)
            .filter_map(|r| p.cell(r, 0))
            .find(|c| c.field == Some(groups) && c.item.is_some())
            .unwrap();
        assert_eq!(ungroup_target(p, &label), Some((groups, label.item)));
        assert_eq!(range_group_field(p, &label), None);

        // Dates in the rows: the periods and the date itself regroup and
        // ungroup as a whole.
        let (mut wb, id) = workbook();
        run(&mut wb, id, PaneOp::Check(DATE));
        let (_, p) = wb.pivot(id).unwrap();
        let years = p.field_index("Years (Date)").unwrap();
        let label = p.cell(3, 0).unwrap();
        assert_eq!(label.field, Some(years));
        assert_eq!(range_group_field(p, &label), Some(years));
        assert_eq!(ungroup_target(p, &label), Some((years, None)));
    }

    #[test]
    fn sources_and_destinations() {
        let (wb, _) = workbook();
        // The block of data around the active cell, or the selection.
        assert_eq!(default_source(&wb, 0, (2, 1), Rect::cell(2, 1)), "Sheet1!$A$1:$D$5");
        assert_eq!(default_source(&wb, 0, (1, 0), Rect::new(0, 0, 2, 1)), "Sheet1!$A$1:$B$3");
        assert_eq!(default_source(&wb, 0, (30, 9), Rect::cell(30, 9)), "");
        assert_eq!(parse_destination("H3", 0, &wb), Some((0, 2, 7)));
        assert_eq!(parse_destination("=Sheet2!$B$4", 0, &wb), Some((1, 3, 1)));
        assert_eq!(parse_destination("Sheet2!C5:D9", 0, &wb), Some((1, 4, 2)));
        assert_eq!(parse_destination("Nowhere!A1", 0, &wb), None);
        assert_eq!(parse_destination("hello", 0, &wb), None);
        assert_eq!(destination_text(&wb, 1, 2, 0), "Sheet2!$A$3");
    }

    #[test]
    fn moves_stay_in_range() {
        assert_eq!(MoveTo::Up.target(0, 3), 0);
        assert_eq!(MoveTo::Up.target(2, 3), 1);
        assert_eq!(MoveTo::Down.target(2, 3), 2);
        assert_eq!(MoveTo::Down.target(0, 3), 1);
        assert_eq!(MoveTo::Beginning.target(2, 3), 0);
        assert_eq!(MoveTo::End.target(0, 3), 2);
        assert_eq!(MoveTo::End.target(0, 0), 0);
    }

    /// The labels of the first column of a report, from its first row of
    /// values down.
    fn row_labels(wb: &Workbook, id: u32) -> Vec<String> {
        let (sheet, p) = wb.pivot(id).unwrap();
        let body = p.location.unwrap();
        (body.r0 + p.report.first_data_row()..=body.r1)
            .map(|row| wb.display_text(sheet, row, body.c0).text)
            .collect()
    }

    #[test]
    fn items_move_hide_and_show_their_detail() {
        let (mut wb, id) = workbook();
        // More regions, to have something to move.
        for (r, row) in [["North", "Apples", "2024-03-01", "70"], ["South", "Pears", "2024-04-01", "30"]]
            .iter()
            .enumerate()
        {
            for (c, text) in row.iter().enumerate() {
                wb.set_cell_text(0, 5 + r as u32, c as u32, text).unwrap();
            }
        }
        wb.commit("more data");
        let source = PivotSource::Range {
            sheet: "Sheet1".to_string(),
            range: Rect::new(0, 0, 6, 3),
        };
        wb.pivot_change_source(id, source, false).unwrap();
        for op in [PaneOp::Check(REGION), PaneOp::Check(SALES)] {
            run(&mut wb, id, op);
        }
        // The regions that came with the new source follow the others, as
        // new items do in Excel.
        assert_eq!(row_labels(&wb, id), ["East", "West", "North", "South", "Grand Total"]);
        let item = |wb: &Workbook, label: &str| {
            let (_, p) = wb.pivot(id).unwrap();
            (0..4u32).find(|x| p.item_text(REGION, *x, wb.locale) == label).unwrap()
        };
        let (east, north, south, west) = (item(&wb, "East"), item(&wb, "North"), item(&wb, "South"), item(&wb, "West"));
        let change = |wb: &mut Workbook, edit: &dyn Fn(&mut PivotTable)| {
            wb.pivot_update(id, false, edit).unwrap();
            wb.commit("change");
        };

        // Moving among the items shown.
        change(&mut wb, &|p| move_item(p, REGION, west, MoveTo::Beginning));
        assert_eq!(row_labels(&wb, id), ["West", "East", "North", "South", "Grand Total"]);
        change(&mut wb, &|p| move_item(p, REGION, west, MoveTo::Down));
        assert_eq!(row_labels(&wb, id), ["East", "West", "North", "South", "Grand Total"]);
        change(&mut wb, &|p| move_item(p, REGION, east, MoveTo::End));
        assert_eq!(row_labels(&wb, id), ["West", "North", "South", "East", "Grand Total"]);
        change(&mut wb, &|p| move_item(p, REGION, east, MoveTo::Up));
        assert_eq!(row_labels(&wb, id), ["West", "North", "East", "South", "Grand Total"]);
        // At the end already: nothing moves.
        change(&mut wb, &|p| move_item(p, REGION, south, MoveTo::Down));
        assert_eq!(row_labels(&wb, id), ["West", "North", "East", "South", "Grand Total"]);

        // Hiding the selection, then keeping only one item; a hidden item
        // is stepped over by a move.
        change(&mut wb, &|p| filter_items(p, REGION, &[north], false));
        assert_eq!(row_labels(&wb, id), ["West", "East", "South", "Grand Total"]);
        change(&mut wb, &|p| move_item(p, REGION, east, MoveTo::Up));
        assert_eq!(row_labels(&wb, id), ["East", "West", "South", "Grand Total"]);
        change(&mut wb, &|p| filter_items(p, REGION, &[south, west], true));
        assert_eq!(row_labels(&wb, id), ["West", "South", "Grand Total"]);
        // Hiding everything that shows is not done.
        change(&mut wb, &|p| filter_items(p, REGION, &[south, west], false));
        assert_eq!(row_labels(&wb, id), ["West", "South", "Grand Total"]);

        // "Show Detail" on West: the products go below, shown for West only.
        let locale = wb.locale;
        change(&mut wb, &|p| {
            p.clear_filters(REGION);
            show_detail(p, REGION, west, PRODUCT, locale);
        });
        assert_eq!(labels(&wb, id, Area::Rows), ["Region", "Product"]);
        assert_eq!(
            row_labels(&wb, id),
            ["East", "West", "Apples", "Pears", "North", "South", "Grand Total"]
        );

        // Subtotals of the regions off, then on again.
        let subtotals = |wb: &Workbook| wb.pivot(id).unwrap().1.fields[REGION].subtotals.clone();
        change(&mut wb, &|p| toggle_subtotal(p, REGION));
        assert_eq!(subtotals(&wb), Subtotals::None);
        change(&mut wb, &|p| toggle_subtotal(p, REGION));
        assert_eq!(subtotals(&wb), Subtotals::Automatic);
    }

    #[test]
    fn a_deferred_layout_is_adopted_whole() {
        let (mut wb, id) = workbook();
        run(&mut wb, id, PaneOp::Check(REGION));
        let mut pending = wb.pivot(id).unwrap().1.clone();
        for op in [
            PaneOp::Uncheck(REGION),
            PaneOp::Add {
                field: PRODUCT,
                area: Area::Columns,
                position: None,
            },
            PaneOp::Check(SALES),
        ] {
            apply_op(&mut pending, op, wb.locale);
        }
        wb.pivot_update(id, false, |p| adopt_layout(p, &pending)).unwrap();
        wb.commit("update");
        assert!(labels(&wb, id, Area::Rows).is_empty());
        assert_eq!(labels(&wb, id, Area::Columns), ["Product"]);
        assert_eq!(labels(&wb, id, Area::Values), ["Sum of Sales"]);
        // One undo step brings the report back.
        wb.undo();
        assert_eq!(labels(&wb, id, Area::Rows), ["Region"]);
    }

    #[test]
    fn the_cells_of_a_pivot_refuse_edits() {
        let (mut wb, id) = workbook();
        for op in [PaneOp::Check(REGION), PaneOp::Check(SALES)] {
            run(&mut wb, id, op);
        }
        // The report is A3:B6 of the second sheet.
        let body = wb.pivot(id).unwrap().1.location.unwrap();
        assert_eq!(body, Rect::new(2, 0, 5, 1));
        let refused = |intent: Intent| refusal(&wb, 1, &intent);
        // Typing into a cell, clearing or pasting over part of the report.
        assert_eq!(refused(Intent::EditCells(Rect::cell(3, 1))), Some(cannot_change_part()));
        assert_eq!(refused(Intent::EditCells(Rect::new(3, 0, 4, 1))), Some(would_affect_pivot()));
        assert_eq!(refused(Intent::EditCells(Rect::new(4, 1, 9, 5))), Some(would_affect_pivot()));
        // The whole report, or cells beside it, may be cleared.
        assert_eq!(refused(Intent::EditCells(body)), None);
        assert_eq!(refused(Intent::EditCells(Rect::new(0, 0, 20, 8))), None);
        assert_eq!(refused(Intent::EditCells(Rect::cell(3, 2))), None);
        // Formatting its cells is allowed, sorting them is not.
        assert_eq!(refused(Intent::FormatCells(Rect::cell(3, 1))), None);
        assert_eq!(refused(Intent::Sort(Rect::new(3, 0, 5, 1))), Some(would_affect_pivot()));
        assert_eq!(refused(Intent::Sort(Rect::new(8, 0, 9, 1))), None);
        // Rows and columns: not through the report, but before it, after
        // it, or all of it at once.
        assert_eq!(refused(Intent::InsertRows { at: 4, count: 1 }), Some(would_affect_pivot()));
        assert_eq!(refused(Intent::InsertRows { at: 2, count: 1 }), None);
        assert_eq!(refused(Intent::InsertRows { at: 6, count: 3 }), None);
        assert_eq!(refused(Intent::DeleteRows { first: 3, last: 3 }), Some(would_affect_pivot()));
        assert_eq!(refused(Intent::DeleteRows { first: 2, last: 5 }), None);
        assert_eq!(refused(Intent::DeleteRows { first: 0, last: 1 }), None);
        assert_eq!(refused(Intent::InsertColumns { at: 1, count: 1 }), Some(would_affect_pivot()));
        assert_eq!(refused(Intent::InsertColumns { at: 0, count: 1 }), None);
        assert_eq!(refused(Intent::DeleteColumns { first: 1, last: 1 }), Some(would_affect_pivot()));
        assert_eq!(refused(Intent::DeleteColumns { first: 0, last: 1 }), None);
        // The data sheet has no PivotTable: nothing is refused there.
        assert_eq!(refusal(&wb, 0, &Intent::EditCells(Rect::cell(1, 1))), None);
        assert_eq!(refusal(&wb, 0, &Intent::Sort(Rect::new(0, 0, 4, 3))), None);
    }

    #[test]
    fn only_edits_of_the_contents_take_a_pivot_away() {
        let paste = |mode| ChangeCause::Paste {
            from: None,
            cut: false,
            mode,
        };
        assert!(replaces_contents(ChangeCause::Typing));
        assert!(replaces_contents(ChangeCause::Fill));
        assert!(replaces_contents(ChangeCause::Clear(ClearMode::Contents)));
        assert!(replaces_contents(ChangeCause::Clear(ClearMode::All)));
        assert!(!replaces_contents(ChangeCause::Clear(ClearMode::Formats)));
        assert!(!replaces_contents(ChangeCause::Clear(ClearMode::Comments)));
        assert!(replaces_contents(paste(PasteMode::All)));
        assert!(replaces_contents(paste(PasteMode::Values)));
        assert!(replaces_contents(paste(PasteMode::ValuesAndNumberFormats)));
        assert!(!replaces_contents(paste(PasteMode::Formats)));
        assert!(!replaces_contents(paste(PasteMode::ColumnWidths)));
        assert!(!replaces_contents(paste(PasteMode::Validation)));
    }

    #[test]
    fn buttons_stay_inside_their_cell() {
        let inside = |b: Box4, cell: Box4| b.0 >= cell.0 && b.1 >= cell.1 && b.2 <= cell.2 && b.3 <= cell.3;
        for zoom in [0.5f32, 1.0, 1.75, 4.0] {
            // A cell of the default size at this zoom.
            let cell = (40.0, 60.0, 40.0 + 64.0 * zoom, 60.0 + 20.0 * zoom);
            let button = filter_button_box(cell, zoom).unwrap();
            assert!(inside(button, cell), "{zoom}");
            // Square, in the bottom right corner.
            assert_eq!(button.2 - button.0, button.3 - button.1);
            assert_eq!((button.2, button.3), (cell.2 - 2.0, cell.3 - 2.0));
            for indent in 0..3u8 {
                let drill = drill_button_box(cell, indent, zoom).unwrap();
                assert!(inside(drill, cell), "{zoom} {indent}");
                // The text starts after its padding and one step more of
                // indentation than its level: clear of the box.
                let text = cell.0 + (3.0 * zoom).round().max(1.0) + (indent as f32 + 1.0) * INDENT * zoom;
                assert!(drill.2 <= text + 0.5, "{zoom} {indent}: {} > {text}", drill.2);
            }
        }
        // At 100 % the drop-down button is 15 pixels wide: what the fitted
        // columns leave room for.
        let button = filter_button_box((0.0, 0.0, 64.0, 20.0), 1.0).unwrap();
        assert_eq!(button.2 - button.0, 15.0);
        assert!(button.2 - button.0 + 2.0 <= FILTER_BUTTON);
        // A low row gets a smaller button, a hidden one none.
        let low = filter_button_box((0.0, 0.0, 64.0, 12.0), 1.0).unwrap();
        assert_eq!(low.3 - low.1, 9.0);
        assert_eq!(filter_button_box((0.0, 0.0, 64.0, 2.0), 1.0), None);
        assert_eq!(drill_button_box((0.0, 0.0, 4.0, 20.0), 0, 1.0), None);
        // A label too deep for its column has no box.
        assert_eq!(drill_button_box((0.0, 0.0, 30.0, 20.0), 3, 1.0), None);
    }

    #[test]
    fn a_dragged_field_lands_between_entries() {
        // Entries are 24 pixels high: above the middle of the first one,
        // before it; past the middle, after it.
        assert_eq!(drop_slot(-5.0, 24.0, 3), 0);
        assert_eq!(drop_slot(11.0, 24.0, 3), 0);
        assert_eq!(drop_slot(13.0, 24.0, 3), 1);
        assert_eq!(drop_slot(40.0, 24.0, 3), 2);
        assert_eq!(drop_slot(500.0, 24.0, 3), 3);
        assert_eq!(drop_slot(50.0, 24.0, 0), 0);
    }

    #[test]
    fn report_layout_is_the_shared_form() {
        let (mut wb, id) = workbook();
        assert_eq!(report_layout(wb.pivot(id).unwrap().1), Some(FieldLayout::Compact));
        wb.pivot_update(id, false, |p| p.fields[0].layout = FieldLayout::Tabular).unwrap();
        assert_eq!(report_layout(wb.pivot(id).unwrap().1), None);
    }
}
