//! Review ▸ Check Accessibility: Excel's rules run over the workbook, the
//! Accessibility pane that lists what they find as Errors, Warnings and
//! Tips, and the indicator of the status bar.
//!
//! The rules, with Excel's names: Missing alt text (pictures, shapes
//! without text, SmartArt and groups that are neither described nor
//! marked as decorative), Missing table header, Hard-to-read text
//! contrast, Use of merged cells, Avoid red formatting (negative numbers
//! told from the others by their colour alone), Blank table rows (and
//! columns), Unclear hyperlink text and Default sheet name.

use crate::app::{ExcelApp, MenuAction};
use crate::features::{
    Cx, Intent, StatusItem, drawings, edit_tools, format_cells, object_id, tables,
};
use crate::i18n::t;
use crate::theme::{self, c};
use crate::widgets::Button;
use gpui::{
    Context, IntoElement, ScrollHandle, SharedString, Subscription, WeakEntity, Window, div,
    prelude::*, px,
};
use opensheet_engine::drawing::{Drawing, DrawingKind};
use opensheet_engine::format::format_value;
use opensheet_engine::hyperlink::LinkKind;
use opensheet_engine::locale::Locale;
use opensheet_engine::style::Style;
use opensheet_engine::workbook::Content;
use opensheet_engine::{Rect, Value, Workbook};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;

pub(super) const PANE: &str = "proofing-accessibility";

// -------------------------------------------------------------------- rules

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Severity {
    Error,
    Warning,
    Tip,
}

impl Severity {
    pub const ALL: [Severity; 3] = [Severity::Error, Severity::Warning, Severity::Tip];

    fn title(self) -> &'static str {
        match self {
            Severity::Error => t("Errors", "Erreurs"),
            Severity::Warning => t("Warnings", "Avertissements"),
            Severity::Tip => t("Tips", "Conseils"),
        }
    }

    /// What Excel says a result of this kind means.
    fn meaning(self) -> &'static str {
        match self {
            Severity::Error => t(
                "Errors indicate content that people with disabilities are unable to read.",
                "Les erreurs indiquent le contenu non lisible par les personnes en situation de handicap.",
            ),
            Severity::Warning => t(
                "Warnings indicate content that people with disabilities find difficult to read.",
                "Les avertissements signalent le contenu difficile à lire pour les personnes en situation de handicap.",
            ),
            Severity::Tip => t(
                "Tips indicate content that people with disabilities might find difficult to read.",
                "Les conseils signalent le contenu potentiellement difficile à lire pour les personnes en situation de handicap.",
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Rule {
    MissingAltText,
    MissingTableHeader,
    Contrast,
    MergedCells,
    RedNegative,
    BlankTableRows,
    UnclearLink,
    DefaultSheetName,
}

impl Rule {
    pub fn severity(self) -> Severity {
        match self {
            Rule::MissingAltText | Rule::MissingTableHeader => Severity::Error,
            Rule::DefaultSheetName => Severity::Tip,
            _ => Severity::Warning,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Rule::MissingAltText => t("Missing alt text", "Texte de remplacement manquant"),
            Rule::MissingTableHeader => t("Missing table header", "En-tête de tableau manquant"),
            Rule::Contrast => t(
                "Hard-to-read text contrast",
                "Contraste de texte dur à lire",
            ),
            Rule::MergedCells => t("Use of merged cells", "Utilisation de cellules fusionnées"),
            Rule::RedNegative => t("Avoid red formatting", "Éviter la mise en forme rouge"),
            Rule::BlankTableRows => t("Blank table rows", "Rangées de tableau vides"),
            Rule::UnclearLink => t(
                "Unclear hyperlink text",
                "Texte de lien hypertexte pas assez explicite",
            ),
            Rule::DefaultSheetName => t("Default sheet name", "Nom de la feuille par défaut"),
        }
    }

    /// "Why Fix?"
    pub fn why(self) -> &'static str {
        match self {
            Rule::MissingAltText => t(
                "Alternative text for images and other objects is very important for people who can't see the screen. Screen readers read alternative text aloud, so it's the only information many have about the image. Good alternative text helps them understand the image.",
                "Les textes de remplacement associés aux images et à d’autres objets sont très importants pour les personnes qui ne peuvent pas voir l’écran. Les lecteurs d’écran lisent le texte de remplacement à voix haute ; il s’agit donc des seules informations dont disposent de nombreuses personnes sur l’image. Des textes de remplacement pertinents leur permettent donc de mieux comprendre l’image.",
            ),
            Rule::MissingTableHeader => t(
                "A table header row contains column headings that provide context and aid navigation of the data in the table.",
                "Une ligne d’en-tête de tableau contient les en-têtes de colonne qui donnent un contexte aidant la navigation dans les données du tableau.",
            ),
            Rule::Contrast => t(
                "Text becomes difficult to read when its color is too similar to the color behind it. Increasing contrast makes text easier to read, especially when viewing documents in bright light.",
                "Le texte devient difficile à lire lorsque sa couleur est trop proche de celle de l’arrière-plan. L’augmentation du contraste permet de faciliter la lecture du texte, en particulier lors de la lecture de documents dans un environnement avec une lumière vive.",
            ),
            Rule::MergedCells => t(
                "Spreadsheets should have simple structure so that they can be easily navigated and understood by people with disabilities. Merged cells cause a screen reader to navigate in unexpected ways, such as repeating or skipping a row.",
                "Les feuilles de calcul doivent avoir une structure simple afin que les personnes en situation de handicap puissent facilement y naviguer et en avoir une bonne compréhension. Les cellules fusionnées peuvent affecter la navigation d’un lecteur d’écran (par exemple, répétition ou saut d’une ligne).",
            ),
            Rule::RedNegative => t(
                "Users who have difficulty distinguishing color won't be able to tell the difference between positive and negative values.",
                "Les utilisateurs qui distinguent difficilement les couleurs ne pourront pas faire la différence entre les valeurs positives et négatives.",
            ),
            Rule::BlankTableRows => t(
                "Blank table cells can mislead a person with a vision disability into thinking there is nothing more in the table.",
                "Les cellules de tableau vides peuvent induire en erreur une personne en situation de handicap visuel et lui faire penser qu’il n’y a plus rien dans le tableau.",
            ),
            Rule::UnclearLink => t(
                "Hyperlink text should make sense. It should give people a clear understanding of the destination, so they can choose whether to follow the link.",
                "Le texte d’un lien hypertexte doit avoir du sens. Il doit fournir une description claire de la destination du lien de sorte que les personnes choisissent de suivre le lien ou non.",
            ),
            Rule::DefaultSheetName => t(
                "Default sheet names like \"Sheet1\" make navigation through the workbook more difficult for people with disabilities.",
                "Les noms de feuille par défaut tels que « Feuil1 » rendent la navigation dans le classeur plus difficile pour les personnes atteintes de handicaps.",
            ),
        }
    }

    /// "Steps To Fix:"
    pub fn steps(self) -> &'static str {
        match self {
            Rule::MissingAltText => t(
                "1. Right-click the object, then select View Alt Text.\n2. Type a description of the object in the text box on the Alt Text pane.",
                "1. Cliquez avec le bouton droit sur l’objet, puis sélectionnez Afficher le texte de remplacement.\n2. Tapez une description de l’objet dans la zone de texte du volet Texte de remplacement.",
            ),
            Rule::MissingTableHeader => t(
                "To set a header row\n1. Select the table, then select the Design tab in Table Tools.\n2. In the Table Style Options group, check Header Row.",
                "Pour définir une ligne d’en-tête :\n1. Sélectionnez le tableau, puis l’onglet Création des Outils de tableau.\n2. Dans le groupe Options de style de tableau, sélectionnez Ligne d’en-tête.",
            ),
            Rule::Contrast => t(
                "Change the cell format:\n1. Select the cell range.\n2. On the Home tab, change the Font color or the Fill color of the cell to increase the contrast.",
                "Modifiez le format de la cellule :\n1. Sélectionnez la plage de cellules.\n2. Dans l’onglet Accueil, modifiez la couleur de police ou la couleur de remplissage de la cellule pour augmenter le contraste.",
            ),
            Rule::MergedCells => t(
                "To simplify the spreadsheet structure:\n1. Select the merged cells.\n2. On Home tab, select Merge & Center button.\n3. Choose the appropriate option to change cells.",
                "Pour simplifier la structure de la feuille de calcul :\n1. Sélectionnez les cellules fusionnées.\n2. Dans l’onglet Accueil, sélectionnez le bouton Fusionner et centrer.\n3. Sélectionnez l’option appropriée pour modifier les cellules.",
            ),
            Rule::RedNegative => t(
                "Change the cell number format:\n1. Select the cell range.\n2. Press Ctrl+1 to open the Format Cells dialog, and in the Number category, choose one of the negative number formats that uses a black font.",
                "Modifiez le format de nombre de la cellule :\n1. Sélectionnez la plage de cellules.\n2. Appuyez sur Ctrl+1 pour ouvrir la boîte de dialogue Format de cellule, puis dans la catégorie Nombre, choisissez un des formats de nombre négatif qui utilise une police noire.",
            ),
            Rule::BlankTableRows => t(
                "Delete unnecessary blank cells. Avoid using blank cells or rows for formatting.",
                "Supprimez les cellules vides inutiles. Évitez d’utiliser des cellules ou des lignes vides pour la mise en forme.",
            ),
            Rule::UnclearLink => t(
                "To make the hyperlink text more accessible:\n1. Click Hyperlink on the Insert tab to open the Edit Hyperlink dialog.\n2. Type a phrase in Text to display which describes the link destination.  Note: You can also click ScreenTip in the Edit Hyperlink dialog to add a meaningful description to the hyperlink.",
                "Pour rendre le texte du lien hypertexte plus accessible :\n1. Dans l’onglet Insertion, cliquez sur Lien hypertexte pour ouvrir la boîte de dialogue Modifier le lien hypertexte.\n2. Dans Texte à afficher, tapez une phrase qui décrive la destination du lien. Remarque : vous pouvez également cliquer sur Info-bulle dans la boîte de dialogue Modifier le lien hypertexte pour ajouter une description explicite au lien hypertexte.",
            ),
            Rule::DefaultSheetName => t(
                "Sheet names should provide information about what is found on the sheet. Blank sheets should be removed.\n\nTo rename a sheet:\n1. Right-click on the sheet tab and choose Rename.\n2. Type a brief name that is descriptive of the sheet contents.\n\nTo delete a sheet, right-click on the sheet tab and choose Delete.",
                "Le nom des feuilles doit fournir des informations sur leur contenu. Les feuilles vides doivent être supprimées.\n\nPour renommer une feuille :\n1. Cliquez avec le bouton droit sur l’onglet de la feuille, puis sélectionnez Renommer.\n2. Tapez un nom court décrivant le contenu de la feuille.\n\nPour supprimer une feuille, cliquez avec le bouton droit sur l’onglet de la feuille, puis cliquez sur Supprimer.",
            ),
        }
    }

    /// The "Recommended Actions" of a result of this rule.
    pub fn actions(self) -> &'static [Action] {
        match self {
            Rule::MissingAltText => &[Action::AddDescription, Action::MarkDecorative],
            Rule::MissingTableHeader => &[Action::UseFirstRowAsHeader],
            Rule::Contrast => &[Action::FormatCellFont],
            Rule::MergedCells => &[Action::Unmerge],
            Rule::RedNegative => &[Action::FormatCellNumber],
            Rule::BlankTableRows => &[],
            Rule::UnclearLink => &[Action::EditLink],
            Rule::DefaultSheetName => &[Action::RenameSheet],
        }
    }
}

/// What a result is about.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Target {
    /// A picture or a shape: its sheet and the id of its drawing.
    Object(usize, u32),
    Table(usize, u32),
    Cells(usize, Rect),
    Sheet(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Finding {
    pub rule: Rule,
    pub target: Target,
    /// As the pane lists it: "Picture 1 (Sheet1)".
    pub label: String,
}

/// Something the pane can do about a result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    AddDescription,
    MarkDecorative,
    UseFirstRowAsHeader,
    Unmerge,
    RenameSheet,
    FormatCellFont,
    FormatCellNumber,
    EditLink,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::AddDescription => t("Add a description", "Ajouter une description"),
            Action::MarkDecorative => t("Mark as decorative", "Marquer comme décoratif"),
            Action::UseFirstRowAsHeader => t(
                "Use first row as header",
                "Utiliser la première ligne comme en-tête",
            ),
            Action::Unmerge => t("Unmerge", "Annuler la fusion"),
            Action::RenameSheet => t("Rename sheet", "Renommer la feuille"),
            Action::FormatCellFont => t("Format cell font", "Format de police de cellule"),
            Action::FormatCellNumber => t("Format cell number", "Format de nombre de cellule"),
            Action::EditLink => t("Edit Link…", "Modifier le lien…"),
        }
    }
}

// --------------------------------------------------------------- inspection

/// How easily a text is read on its background: the contrast ratio of
/// two 0xRRGGBB colours, from 1 (the same colour) to 21 (black on white).
pub(super) fn contrast_ratio(a: u32, b: u32) -> f64 {
    fn luminance(rgb: u32) -> f64 {
        let channel = |shift: u32| {
            let value = ((rgb >> shift) & 0xFF) as f64 / 255.0;
            if value <= 0.03928 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
    }
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The contrast under which text is hard to read (what the Web Content
/// Accessibility Guidelines ask of ordinary text).
const MIN_CONTRAST: f64 = 4.5;

fn hard_to_read(style: &Style) -> bool {
    let text = style.font.color.map_or(0x000000, |color| color.value());
    let behind = style.fill.map_or(0xFFFFFF, |color| color.value());
    contrast_ratio(text, behind) < MIN_CONTRAST
}

/// Whether a number format tells negative numbers from the others by a
/// colour alone: red, without a minus sign or brackets.
pub(super) fn red_only_negative(code: &str, locale: &Locale) -> bool {
    let shown = format_value(&Value::Number(-1234.5), code, locale);
    let reddish = shown
        .color
        .is_some_and(|color| color.r() > 160 && color.g() < 110 && color.b() < 110);
    reddish
        && !shown.text.contains(['-', '−', '('])
        && shown.text.chars().any(|ch| ch.is_ascii_digit())
}

/// Whether a sheet still has the name it was created with, in any of the
/// languages of the interface ("Sheet1", "Feuil2", "Tabelle3"...).
pub(super) fn is_default_sheet_name(name: &str) -> bool {
    ["en", "fr", "es", "de", "it"].iter().any(|id| {
        let prefix = Locale::by_id(id).sheet_prefix.to_lowercase();
        let name = name.to_lowercase();
        name.strip_prefix(&prefix)
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit()))
    })
}

/// The cells of a set as rectangles: runs along each row, joined with the
/// same run of the rows below.
pub(super) fn ranges_of(cells: &BTreeSet<(u32, u32)>) -> Vec<Rect> {
    let mut runs: Vec<Rect> = Vec::new();
    for (row, col) in cells {
        match runs.last_mut() {
            Some(run) if run.r0 == *row && run.c1 + 1 == *col => run.c1 = *col,
            _ => runs.push(Rect::new(*row, *col, *row, *col)),
        }
    }
    let mut out: Vec<Rect> = Vec::new();
    for run in runs {
        match out
            .iter_mut()
            .find(|rect| rect.c0 == run.c0 && rect.c1 == run.c1 && rect.r1 + 1 == run.r0)
        {
            Some(rect) => rect.r1 = run.r0,
            None => out.push(run),
        }
    }
    out
}

/// Excel's "|1 (|0)": what and on which sheet.
fn on_sheet(what: &str, sheet: &str) -> String {
    format!("{what} ({sheet})")
}

fn needs_alt_text(drawing: &Drawing) -> bool {
    if drawing.hidden || drawing.decorative || !drawing.alt_text.trim().is_empty() {
        return false;
    }
    // The text of a shape is what a screen reader says of it.
    match &drawing.kind {
        DrawingKind::Shape(shape) => shape.text.trim().is_empty(),
        _ => true,
    }
}

/// Run every rule over the workbook. The results come sheet by sheet
/// within each rule, in the order of `Rule`.
pub(super) fn inspect(wb: &Workbook) -> Vec<Finding> {
    let mut found = Vec::new();
    let mut red_formats: HashMap<std::sync::Arc<str>, bool> = HashMap::new();
    for (index, sh) in wb.sheets.iter().enumerate() {
        let name = sh.name.as_str();
        let cells = |rule: Rule, set: &BTreeSet<(u32, u32)>, found: &mut Vec<Finding>| {
            found.extend(ranges_of(set).into_iter().map(|rect| Finding {
                rule,
                target: Target::Cells(index, rect),
                label: on_sheet(&rect.name(), name),
            }));
        };
        for drawing in sh.drawings.iter().filter(|drawing| needs_alt_text(drawing)) {
            found.push(Finding {
                rule: Rule::MissingAltText,
                target: Target::Object(index, drawing.id),
                label: on_sheet(&drawing.name, name),
            });
        }
        for table in &sh.tables {
            if !table.header_row {
                found.push(Finding {
                    rule: Rule::MissingTableHeader,
                    target: Target::Table(index, table.id),
                    label: on_sheet(&table.name, name),
                });
            }
            // Rows and columns of the data left blank from end to end; a
            // table of one empty row is a table not filled in yet.
            let Some(data) = table.data_range().filter(|data| data.r1 > data.r0) else {
                continue;
            };
            let blank = |row: u32, col: u32| {
                sh.cells
                    .get(&(row, col))
                    .is_none_or(|cell| matches!(cell.content, Content::Empty))
            };
            let mut blanks = BTreeSet::new();
            for row in data.r0..=data.r1 {
                if (data.c0..=data.c1).all(|col| blank(row, col)) {
                    blanks.extend((data.c0..=data.c1).map(|col| (row, col)));
                }
            }
            for col in data.c0..=data.c1 {
                if (data.r0..=data.r1).all(|row| blank(row, col)) {
                    blanks.extend((data.r0..=data.r1).map(|row| (row, col)));
                }
            }
            cells(Rule::BlankTableRows, &blanks, &mut found);
        }
        // What shows in the cells: its contrast and its negative numbers.
        let plain = sh.tables.is_empty() && sh.cond_formats.is_empty();
        let mut by_style: HashMap<u32, bool> = HashMap::new();
        let (mut faint, mut red) = (BTreeSet::new(), BTreeSet::new());
        for (at, cell) in &sh.cells {
            if cell.value.is_empty() {
                continue;
            }
            let unreadable = if plain {
                *by_style
                    .entry(cell.style.0)
                    .or_insert_with(|| hard_to_read(wb.style_at(index, at.0, at.1)))
            } else {
                hard_to_read(&wb.display(index, at.0, at.1).style)
            };
            if unreadable {
                faint.insert(*at);
            }
            if matches!(cell.value, Value::Number(_)) {
                let code = &wb.style_at(index, at.0, at.1).num_fmt;
                let red_only = match red_formats.get(code) {
                    Some(known) => *known,
                    None => {
                        let known = red_only_negative(code, wb.locale);
                        red_formats.insert(code.clone(), known);
                        known
                    }
                };
                if red_only {
                    red.insert(*at);
                }
            }
        }
        cells(Rule::Contrast, &faint, &mut found);
        for merge in &sh.merges {
            found.push(Finding {
                rule: Rule::MergedCells,
                target: Target::Cells(index, *merge),
                label: on_sheet(&merge.name(), name),
            });
        }
        cells(Rule::RedNegative, &red, &mut found);
        for link in &sh.hyperlinks {
            let shown = wb.display_text(index, link.range.r0, link.range.c0).text;
            let shown = shown.trim().to_lowercase();
            let bare = shown.is_empty()
                || shown == link.address.trim().to_lowercase()
                || ["http://", "https://", "ftp://", "www.", "mailto:"]
                    .iter()
                    .any(|scheme| shown.starts_with(scheme));
            // A place in the workbook named by its reference reads well.
            if bare && link.kind() != LinkKind::Place {
                found.push(Finding {
                    rule: Rule::UnclearLink,
                    target: Target::Cells(index, link.range),
                    label: on_sheet(&link.range.name(), name),
                });
            }
        }
        let used = sh.cells.values().any(|cell| !cell.value.is_empty())
            || !sh.charts.is_empty()
            || !sh.drawings.is_empty();
        if used && is_default_sheet_name(name) {
            found.push(Finding {
                rule: Rule::DefaultSheetName,
                target: Target::Sheet(index),
                label: name.to_string(),
            });
        }
    }
    found.sort_by_key(|finding| finding.rule);
    found
}

/// Past this many cells the workbook is only checked when the pane asks,
/// as Excel does with large workbooks.
const BACKGROUND_LIMIT: usize = 200_000;

fn cell_count(wb: &Workbook) -> usize {
    wb.sheets.iter().map(|sheet| sheet.cells.len()).sum()
}

/// What tells one state of a workbook from another, cheaply: where it
/// lives, how many times it was calculated (every committed change
/// calculates), and a little of its shape.
fn stamp(wb: &Workbook) -> (usize, u64, usize, usize) {
    let objects: usize = wb
        .sheets
        .iter()
        .map(|sheet| sheet.drawings.len() + sheet.name.len())
        .sum();
    (
        wb as *const Workbook as usize,
        wb.generation,
        cell_count(wb),
        wb.sheets.len() * 1000 + objects,
    )
}

thread_local! {
    /// The results of the last inspections, by the stamp of their
    /// workbook: the status bar asks at every repaint.
    static CHECKED: RefCell<Vec<((usize, u64, usize, usize), Rc<Vec<Finding>>)>> = const { RefCell::new(Vec::new()) };
}

/// The results for a workbook as it is now.
pub(super) fn findings(wb: &Workbook) -> Rc<Vec<Finding>> {
    let stamp = stamp(wb);
    CHECKED.with(|checked| {
        let mut checked = checked.borrow_mut();
        if let Some((_, known)) = checked.iter().find(|(known, _)| *known == stamp) {
            return known.clone();
        }
        let fresh = Rc::new(inspect(wb));
        // One entry per window is plenty; older states are dropped.
        checked.retain(|(known, _)| known.0 != stamp.0);
        if checked.len() >= 8 {
            checked.remove(0);
        }
        checked.push((stamp, fresh.clone()));
        fresh
    })
}

// --------------------------------------------------------------- status bar

/// "Accessibility: Good to go" or "Accessibility: Investigate"; a click
/// opens the pane.
pub(super) fn status_items(app: &ExcelApp) -> Vec<StatusItem> {
    let text = if cell_count(&app.wb) > BACKGROUND_LIMIT && !app.pane_open(PANE) {
        t(
            "Workbook too large for background checking. Click to check accessibility",
            "Classeur trop volumineux pour une vérification en arrière-plan. Cliquez pour vérifier l’accessibilité.",
        )
    } else if findings(&app.wb).is_empty() {
        t(
            "Accessibility: Good to go",
            "Accessibilité : vérification terminée",
        )
    } else {
        t(
            "Accessibility: Investigate",
            "Accessibilité : consultez nos recommandations",
        )
    };
    let mut item = StatusItem::new(text);
    let open: MenuAction = Rc::new(|app, _, cx| open(app, cx));
    item.on_click = Some(open);
    vec![item]
}

// ------------------------------------------------------------------ actions

/// Show what a result is about: its sheet comes to the front and its
/// cells or its object are selected.
pub(super) fn go_to(app: &mut ExcelApp, target: Target, window: &mut Window, cx: &mut Cx) {
    let sheet = match target {
        Target::Object(sheet, _)
        | Target::Table(sheet, _)
        | Target::Cells(sheet, _)
        | Target::Sheet(sheet) => sheet,
    };
    if sheet >= app.wb.sheets.len() {
        return;
    }
    if sheet != app.sheet {
        app.switch_sheet(sheet, window, cx);
    }
    match target {
        Target::Object(_, id) => {
            if let Some(drawing) = app.wb.drawing(sheet, id) {
                let (row, col) = drawing.anchor;
                app.ensure_visible(row, col);
            }
            app.select_object(Some(object_id(&drawings::Drawings, id)), cx);
        }
        Target::Table(_, id) => {
            if let Some(range) = app.wb.table(sheet, id).map(|table| table.range) {
                app.deselect_object(cx);
                app.select_rect(range, cx);
            }
        }
        Target::Cells(_, rect) => {
            app.deselect_object(cx);
            app.select_rect(rect, cx);
        }
        Target::Sheet(_) => app.refresh(cx),
    }
}

/// Do one of the Recommended Actions of a result.
pub(super) fn act(
    app: &mut ExcelApp,
    finding: &Finding,
    action: Action,
    window: &mut Window,
    cx: &mut Cx,
) {
    go_to(app, finding.target, window, cx);
    match (action, finding.target) {
        (Action::AddDescription, Target::Object(..)) => {
            app.run_command(drawings::Cmd::AltText, window, cx)
        }
        (Action::MarkDecorative, Target::Object(sheet, id)) => {
            if app.may(
                Intent::EditObject(Some(object_id(&drawings::Drawings, id))),
                cx,
            ) {
                app.wb
                    .update_drawing(sheet, id, |drawing| drawing.decorative = true);
                app.commit(Action::MarkDecorative.label(), cx);
            }
        }
        (Action::UseFirstRowAsHeader, Target::Table(..)) => {
            app.run_command(tables::Cmd::HeaderRow, window, cx)
        }
        (Action::Unmerge, Target::Cells(sheet, rect)) => {
            if app.may(Intent::ChangeSheet, cx) && app.wb.unmerge(sheet, rect) {
                app.commit(t("Unmerge Cells", "Annuler la fusion des cellules"), cx);
            }
        }
        (Action::RenameSheet, Target::Sheet(sheet)) => app.begin_rename_sheet(sheet, window, cx),
        (Action::FormatCellFont, Target::Cells(..)) => {
            app.run_command(format_cells::Cmd::Font, window, cx)
        }
        (Action::FormatCellNumber, Target::Cells(..)) => {
            app.run_command(format_cells::Cmd::Number, window, cx)
        }
        (Action::EditLink, Target::Cells(..)) => {
            app.run_command(edit_tools::Cmd::EditLink, window, cx)
        }
        _ => {}
    }
}

// --------------------------------------------------------------------- pane

#[derive(Default)]
pub(super) struct AccessibilityUi {
    pub pane: Option<gpui::Entity<AccessibilityPane>>,
}

pub(super) struct AccessibilityPane {
    app: WeakEntity<ExcelApp>,
    /// The result whose explanation and actions show.
    pub selected: Option<Finding>,
    scroll: ScrollHandle,
    _subscription: Subscription,
}

impl AccessibilityPane {
    /// The results as the pane lists them now.
    pub fn listed(&self, cx: &gpui::App) -> Rc<Vec<Finding>> {
        match self.app.upgrade() {
            Some(app) => findings(&app.read(cx).wb),
            None => Rc::new(Vec::new()),
        }
    }

    /// A press on a result: it is selected here and shown on the sheet.
    pub fn select(&mut self, finding: Finding, window: &mut Window, cx: &mut Context<Self>) {
        let target = finding.target;
        self.selected = Some(finding);
        if let Some(app) = self.app.upgrade() {
            app.update(cx, |app, cx| go_to(app, target, window, cx));
        }
        cx.notify();
    }

    pub fn run(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(finding), Some(app)) = (self.selected.clone(), self.app.upgrade()) else {
            return;
        };
        app.update(cx, |app, cx| act(app, &finding, action, window, cx));
        self.forget_fixed(cx);
        cx.notify();
    }

    /// A result that was fixed is no longer the selected one.
    fn forget_fixed(&mut self, cx: &gpui::App) {
        let listed = self.listed(cx);
        if self
            .selected
            .as_ref()
            .is_some_and(|selected| !listed.contains(selected))
        {
            self.selected = None;
        }
    }

    fn heading(text: &'static str) -> gpui::Div {
        div()
            .pt(px(8.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .child(text)
    }

    /// A text of Excel in which `\n` ends a line.
    fn paragraphs(text: &str) -> Vec<gpui::Div> {
        text.split('\n')
            .map(|line| {
                div()
                    .min_h(px(8.))
                    .child(SharedString::from(line.trim().to_string()))
            })
            .collect()
    }
}

impl Render for AccessibilityPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.forget_fixed(cx);
        let listed = self.listed(cx);
        let mut results: Vec<gpui::AnyElement> = Vec::new();
        let mut row = 0usize;
        for severity in Severity::ALL {
            let of_kind: Vec<&Finding> = listed
                .iter()
                .filter(|finding| finding.rule.severity() == severity)
                .collect();
            if of_kind.is_empty() {
                continue;
            }
            results.push(
                div()
                    .pt(px(6.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(severity.title())
                    .into_any_element(),
            );
            let mut rule = None;
            for finding in of_kind {
                if rule != Some(finding.rule) {
                    rule = Some(finding.rule);
                    let count = listed
                        .iter()
                        .filter(|other| other.rule == finding.rule)
                        .count();
                    results.push(
                        div()
                            .pl(px(10.))
                            .pt(px(2.))
                            .child(format!("{} ({count})", finding.rule.title()))
                            .into_any_element(),
                    );
                }
                let chosen = self.selected.as_ref() == Some(finding);
                let pressed = finding.clone();
                let id = row;
                row += 1;
                results.push(
                    div()
                        .id(("accessibility-result", id))
                        .debug_selector(move || format!("accessibility-result-{id}"))
                        .ml(px(20.))
                        .px(px(6.))
                        .py(px(2.))
                        .rounded(px(3.))
                        .when(chosen, |d| d.bg(c(theme::ACCENT_LIGHT)))
                        .when(!chosen, |d| d.hover(|s| s.bg(c(theme::HOVER))))
                        .child(SharedString::from(finding.label.clone()))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select(pressed.clone(), window, cx)
                        }))
                        .into_any_element(),
                );
                if chosen && !finding.rule.actions().is_empty() {
                    let buttons =
                        finding
                            .rule
                            .actions()
                            .iter()
                            .enumerate()
                            .map(|(index, action)| {
                                let action = *action;
                                Button::new(("accessibility-action", index), action.label())
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.run(action, window, cx)
                                    }))
                            });
                    results.push(
                        div()
                            .ml(px(26.))
                            .pb(px(4.))
                            .flex()
                            .flex_col()
                            .items_start()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_color(c(theme::TEXT_MUTED))
                                    .child(t("Recommended Actions", "Actions recommandées")),
                            )
                            .children(buttons)
                            .into_any_element(),
                    );
                }
            }
        }
        let good = listed.is_empty();
        let mut pane = div()
            .id("accessibility")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .px(px(12.))
            .pb(px(12.))
            .flex()
            .flex_col()
            .gap(px(1.))
            .line_height(px(18.))
            .child(Self::heading(t(
                "Inspection Results",
                "Résultats de l'inspection",
            )));
        if good {
            pane = pane.child(div().pt(px(4.)).child(t(
                "No accessibility issues found. People with disabilities should not have difficulty reading this workbook.",
                "Aucun problème d’accessibilité n’a été détecté. Les personnes souffrant d’un handicap ne devraient avoir aucune difficulté à lire ce classeur.",
            )));
        } else {
            pane = pane.children(results);
        }
        pane = pane.child(
            Self::heading(t("Additional Information", "Informations complémentaires")).pt(px(14.)),
        );
        match &self.selected {
            Some(finding) => {
                pane = pane
                    .child(
                        div()
                            .text_color(c(theme::TEXT_MUTED))
                            .child(finding.rule.severity().meaning()),
                    )
                    .child(Self::heading(t(
                        "Why Fix?",
                        "Pourquoi corriger le problème ?",
                    )))
                    .child(div().child(finding.rule.why()))
                    .child(Self::heading(t(
                        "Steps To Fix:",
                        "Étapes à suivre pour résoudre le problème :",
                    )))
                    .children(Self::paragraphs(finding.rule.steps()));
            }
            None if !good => {
                pane = pane.child(div().text_color(c(theme::TEXT_MUTED)).child(t(
                    "To make this document more accessible for people with disabilities, select and fix the issues in the Inspection Results box.",
                    "Pour rendre ce document accessible aux personnes souffrant d’un handicap, sélectionnez et corrigez les problèmes dans la zone Résultats de l’inspection.",
                )));
            }
            None => {}
        }
        pane
    }
}

/// Review ▸ Check Accessibility, and the click on the status bar.
pub(super) fn open(app: &mut ExcelApp, cx: &mut Cx) {
    let pane = match app
        .ext::<AccessibilityUi>()
        .and_then(|ui| ui.pane.clone())
        .filter(|_| app.pane_open(PANE))
    {
        Some(pane) => pane,
        None => {
            let entity = cx.entity();
            let pane = cx.new(|cx| AccessibilityPane {
                app: entity.downgrade(),
                selected: None,
                scroll: ScrollHandle::new(),
                // The pane follows the workbook.
                _subscription: cx.observe(&entity, |_, _, cx| cx.notify()),
            });
            app.ext_mut::<AccessibilityUi>().pane = Some(pane.clone());
            pane
        }
    };
    app.open_pane(PANE, t("Accessibility", "Accessibilité"), pane, cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_is_measured() {
        assert!((contrast_ratio(0x000000, 0xFFFFFF) - 21.0).abs() < 0.01);
        assert!((contrast_ratio(0x777777, 0x777777) - 1.0).abs() < 1e-9);
        // Grey #767676 on white is the lightest grey that passes.
        assert!(contrast_ratio(0x767676, 0xFFFFFF) >= MIN_CONTRAST);
        assert!(contrast_ratio(0x777777, 0xFFFFFF) < MIN_CONTRAST);
        assert!(contrast_ratio(0xFFFF00, 0xFFFFFF) < 1.2);
    }

    #[test]
    fn red_only_formats_are_told() {
        let en = Locale::en();
        assert!(red_only_negative("#,##0.00;[Red]#,##0.00", en));
        assert!(red_only_negative("0;[Red]0", en));
        assert!(!red_only_negative("#,##0.00;[Red]-#,##0.00", en));
        assert!(!red_only_negative("#,##0.00_);[Red](#,##0.00)", en));
        assert!(!red_only_negative("General", en));
        assert!(!red_only_negative("0;[Blue]0", en));
        assert!(!red_only_negative("[Red]General", en));
    }

    #[test]
    fn default_names_are_told_in_every_language() {
        for name in [
            "Sheet1",
            "sheet12",
            "Feuil3",
            "Hoja2",
            "Tabelle10",
            "Foglio1",
        ] {
            assert!(is_default_sheet_name(name), "{name}");
        }
        for name in ["Sheet", "Sheet1b", "Budget", "Sheet 1", "Feuille1", ""] {
            assert!(!is_default_sheet_name(name), "{name}");
        }
    }

    #[test]
    fn cells_are_gathered_in_ranges() {
        let cells: BTreeSet<(u32, u32)> = [(0, 0), (0, 1), (1, 0), (1, 1), (2, 0), (4, 4), (4, 6)]
            .into_iter()
            .collect();
        let names: Vec<String> = ranges_of(&cells).iter().map(Rect::name).collect();
        assert_eq!(names, vec!["A1:B2", "A3", "E5", "G5"]);
        assert!(ranges_of(&BTreeSet::new()).is_empty());
    }
}
