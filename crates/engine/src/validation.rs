//! Data validation: what may be typed into cells, with in-cell drop-down
//! lists, input messages and error alerts (`Sheet::validations`).
//!
//! A [`Validation`] is one `<dataValidation>` element of a file: settings
//! plus the ranges they apply to. Its formulas are written for the top-left
//! cell of its first range and read relative to it everywhere else, as in
//! the file. On a sheet a cell belongs to at most one rule: applying a rule
//! cuts its ranges out of the others.
//!
//! Rules are only checked when the user types (`Workbook::check_entry`):
//! pasting, filling and recalculation never refuse a value, which is why
//! cells can hold values their rule forbids (`Workbook::invalid_cells`).

use crate::ast::{Expr, StructuralEdit, UnaryOp};
use crate::eval::{Calc, compare_text};
use crate::format;
use crate::input;
use crate::locale::Locale;
use crate::parser::{ParseError, parse_formula};
use crate::refs::{MAX_COLS, MAX_ROWS, Rect, Reference};
use crate::value::Value;
use crate::workbook::{Cell, Content, Sheet, Workbook};
use std::cmp::Ordering;
use std::sync::Arc;

/// The text for the workbook's language; `texts` follows `Locale::all()`:
/// en, fr, es, de, it. The wording is Excel's own.
pub(crate) fn localized(locale: &Locale, texts: [&'static str; 5]) -> &'static str {
    let index = Locale::all().iter().position(|l| l.id == locale.id).unwrap_or(0);
    texts[index]
}

/// What a rule allows ("Allow" in the dialog, `type` in the file).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ValidationKind {
    /// No restriction; the rule only carries an input message.
    #[default]
    Any,
    Whole,
    Decimal,
    List,
    Date,
    Time,
    TextLength,
    /// A formula that must be true.
    Custom,
}

impl ValidationKind {
    /// In the order of the "Allow" list.
    pub const ALL: [ValidationKind; 8] = [
        ValidationKind::Any,
        ValidationKind::Whole,
        ValidationKind::Decimal,
        ValidationKind::List,
        ValidationKind::Date,
        ValidationKind::Time,
        ValidationKind::TextLength,
        ValidationKind::Custom,
    ];

    /// The `type` attribute of `<dataValidation>`.
    pub fn ooxml_name(self) -> &'static str {
        match self {
            ValidationKind::Any => "none",
            ValidationKind::Whole => "whole",
            ValidationKind::Decimal => "decimal",
            ValidationKind::List => "list",
            ValidationKind::Date => "date",
            ValidationKind::Time => "time",
            ValidationKind::TextLength => "textLength",
            ValidationKind::Custom => "custom",
        }
    }

    pub fn from_ooxml_name(name: &str) -> Option<ValidationKind> {
        ValidationKind::ALL.into_iter().find(|k| k.ooxml_name() == name)
    }

    /// The entry of the "Allow" list.
    pub fn label(self, locale: &Locale) -> &'static str {
        localized(
            locale,
            match self {
                ValidationKind::Any => ["Any value", "Tout", "Cualquier valor", "Jeden Wert", "Qualsiasi valore"],
                ValidationKind::Whole => [
                    "Whole number",
                    "Nombre entier",
                    "Número entero",
                    "Ganze Zahl",
                    "Numero intero",
                ],
                ValidationKind::Decimal => ["Decimal", "Décimal", "Decimal", "Dezimal", "Decimale"],
                ValidationKind::List => ["List", "Liste", "Lista", "Liste", "Elenco"],
                ValidationKind::Date => ["Date", "Date", "Fecha", "Datum", "Data"],
                ValidationKind::Time => ["Time", "Heure", "Hora", "Zeit", "Ora"],
                ValidationKind::TextLength => [
                    "Text length",
                    "Longueur du texte",
                    "Longitud del texto",
                    "Textlänge",
                    "Lunghezza testo",
                ],
                ValidationKind::Custom => [
                    "Custom",
                    "Personnalisé",
                    "Personalizada",
                    "Benutzerdefiniert",
                    "Personalizzato",
                ],
            },
        )
    }

    /// Whether the rule compares a number with one or two bounds (the
    /// "Data" list of the dialog applies).
    pub fn has_operator(self) -> bool {
        !matches!(
            self,
            ValidationKind::Any | ValidationKind::List | ValidationKind::Custom
        )
    }
}

/// How the entry is compared with the bounds ("Data" in the dialog).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Operator {
    #[default]
    Between,
    NotBetween,
    Equal,
    NotEqual,
    GreaterThan,
    LessThan,
    GreaterOrEqual,
    LessOrEqual,
}

impl Operator {
    /// In the order of the "Data" list.
    pub const ALL: [Operator; 8] = [
        Operator::Between,
        Operator::NotBetween,
        Operator::Equal,
        Operator::NotEqual,
        Operator::GreaterThan,
        Operator::LessThan,
        Operator::GreaterOrEqual,
        Operator::LessOrEqual,
    ];

    /// The `operator` attribute of `<dataValidation>`.
    pub fn ooxml_name(self) -> &'static str {
        match self {
            Operator::Between => "between",
            Operator::NotBetween => "notBetween",
            Operator::Equal => "equal",
            Operator::NotEqual => "notEqual",
            Operator::GreaterThan => "greaterThan",
            Operator::LessThan => "lessThan",
            Operator::GreaterOrEqual => "greaterThanOrEqual",
            Operator::LessOrEqual => "lessThanOrEqual",
        }
    }

    pub fn from_ooxml_name(name: &str) -> Option<Operator> {
        Operator::ALL.into_iter().find(|o| o.ooxml_name() == name)
    }

    /// The entry of the "Data" list.
    pub fn label(self, locale: &Locale) -> &'static str {
        localized(
            locale,
            match self {
                Operator::Between => ["between", "comprise entre", "entre", "zwischen", "tra"],
                Operator::NotBetween => [
                    "not between",
                    "non comprise entre",
                    "no está entre",
                    "nicht zwischen",
                    "non compreso tra",
                ],
                Operator::Equal => ["equal to", "égale à", "igual a", "gleich", "uguale a"],
                Operator::NotEqual => ["not equal to", "différente de", "no igual a", "ungleich", "diverso da"],
                Operator::GreaterThan => ["greater than", "supérieure à", "mayor que", "größer als", "maggiore di"],
                Operator::LessThan => ["less than", "inférieure à", "menor que", "kleiner als", "minore di"],
                Operator::GreaterOrEqual => [
                    "greater than or equal to",
                    "supérieure ou égale à",
                    "mayor o igual que",
                    "größer oder gleich",
                    "maggiore o uguale a",
                ],
                Operator::LessOrEqual => [
                    "less than or equal to",
                    "inférieure ou égale à",
                    "menor o igual que",
                    "kleiner oder gleich",
                    "minore o uguale a",
                ],
            },
        )
    }

    /// Between and not between take two bounds, the others one.
    pub fn has_two_operands(self) -> bool {
        matches!(self, Operator::Between | Operator::NotBetween)
    }

    fn test(self, x: f64, a: f64, b: f64) -> bool {
        match self {
            Operator::Between => x >= a && x <= b,
            Operator::NotBetween => x < a || x > b,
            Operator::Equal => x == a,
            Operator::NotEqual => x != a,
            Operator::GreaterThan => x > a,
            Operator::LessThan => x < a,
            Operator::GreaterOrEqual => x >= a,
            Operator::LessOrEqual => x <= a,
        }
    }
}

/// The kind of error alert, which decides what the user may do next:
/// Stop offers Retry / Cancel (the entry is never kept), Warning offers
/// Yes / No / Cancel (Yes keeps it), Information offers OK / Cancel (OK
/// keeps it).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AlertStyle {
    #[default]
    Stop,
    Warning,
    Information,
}

impl AlertStyle {
    pub const ALL: [AlertStyle; 3] = [AlertStyle::Stop, AlertStyle::Warning, AlertStyle::Information];

    /// The `errorStyle` attribute of `<dataValidation>`.
    pub fn ooxml_name(self) -> &'static str {
        match self {
            AlertStyle::Stop => "stop",
            AlertStyle::Warning => "warning",
            AlertStyle::Information => "information",
        }
    }

    pub fn from_ooxml_name(name: &str) -> Option<AlertStyle> {
        AlertStyle::ALL.into_iter().find(|s| s.ooxml_name() == name)
    }

    /// The entry of the "Style" list.
    pub fn label(self, locale: &Locale) -> &'static str {
        localized(
            locale,
            match self {
                AlertStyle::Stop => ["Stop", "Stop", "Alto", "Stopp", "Interruzione"],
                AlertStyle::Warning => ["Warning", "Avertissement", "Advertencia", "Warnung", "Avviso"],
                AlertStyle::Information => [
                    "Information",
                    "Informations",
                    "Información",
                    "Informationen",
                    "Informazione",
                ],
            },
        )
    }
}

/// Input method mode of the cells (East Asian keyboards). Never acted
/// upon; kept so that files keep it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ImeMode {
    #[default]
    NoControl,
    Off,
    On,
    Disabled,
    Hiragana,
    FullKatakana,
    HalfKatakana,
    FullAlpha,
    HalfAlpha,
    FullHangul,
    HalfHangul,
}

impl ImeMode {
    pub const ALL: [ImeMode; 11] = [
        ImeMode::NoControl,
        ImeMode::Off,
        ImeMode::On,
        ImeMode::Disabled,
        ImeMode::Hiragana,
        ImeMode::FullKatakana,
        ImeMode::HalfKatakana,
        ImeMode::FullAlpha,
        ImeMode::HalfAlpha,
        ImeMode::FullHangul,
        ImeMode::HalfHangul,
    ];

    /// The `imeMode` attribute of `<dataValidation>`.
    pub fn ooxml_name(self) -> &'static str {
        match self {
            ImeMode::NoControl => "noControl",
            ImeMode::Off => "off",
            ImeMode::On => "on",
            ImeMode::Disabled => "disabled",
            ImeMode::Hiragana => "hiragana",
            ImeMode::FullKatakana => "fullKatakana",
            ImeMode::HalfKatakana => "halfKatakana",
            ImeMode::FullAlpha => "fullAlpha",
            ImeMode::HalfAlpha => "halfAlpha",
            ImeMode::FullHangul => "fullHangul",
            ImeMode::HalfHangul => "halfHangul",
        }
    }

    pub fn from_ooxml_name(name: &str) -> Option<ImeMode> {
        ImeMode::ALL.into_iter().find(|m| m.ooxml_name() == name)
    }
}

/// One validation rule and the ranges it applies to.
#[derive(Clone, Debug, PartialEq)]
pub struct Validation {
    /// `sqref`. The top-left cell of the first range is the cell the
    /// formulas are written for.
    pub ranges: Vec<Rect>,
    pub kind: ValidationKind,
    /// Only meaningful when `kind.has_operator()`.
    pub operator: Operator,
    /// The value, the lower bound, the list source or the custom formula.
    /// A list typed in the dialog is the text constant `"a,b,c"`, always
    /// separated by commas whatever the language.
    pub formula1: Option<Arc<Expr>>,
    /// The upper bound of between / not between.
    pub formula2: Option<Arc<Expr>>,
    /// "Ignore blank": empty entries are accepted, and so is anything
    /// while a bound refers to an empty cell.
    pub allow_blank: bool,
    /// "In-cell dropdown" of a list (the file stores the opposite, as
    /// `showDropDown`).
    pub in_cell_dropdown: bool,
    /// "Show input message when cell is selected".
    pub show_input: bool,
    pub input_title: String,
    pub input_message: String,
    /// "Show error alert after invalid data is entered". When off, invalid
    /// entries are accepted silently.
    pub show_error: bool,
    pub error_style: AlertStyle,
    pub error_title: String,
    pub error_message: String,
    pub ime_mode: ImeMode,
}

/// The dialog takes titles of this many characters at most.
pub const MAX_TITLE_LEN: usize = 32;

/// The dialog takes messages, and lists typed in the Source box, of this
/// many characters at most.
pub const MAX_MESSAGE_LEN: usize = 255;

/// A new rule as the dialog first shows it. (A file means something else
/// by leaving attributes out: see `file_default`.)
impl Default for Validation {
    fn default() -> Validation {
        Validation {
            ranges: Vec::new(),
            kind: ValidationKind::Any,
            operator: Operator::Between,
            formula1: None,
            formula2: None,
            allow_blank: true,
            in_cell_dropdown: true,
            show_input: true,
            input_title: String::new(),
            input_message: String::new(),
            show_error: true,
            error_style: AlertStyle::Stop,
            error_title: String::new(),
            error_message: String::new(),
            ime_mode: ImeMode::NoControl,
        }
    }
}

/// One of the boxes of the dialog that take a value or a formula.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    Minimum,
    Maximum,
    Formula,
    Source,
    StartDate,
    EndDate,
    StartTime,
    EndTime,
    Value,
    Date,
    Time,
    Length,
}

impl Field {
    /// The names Excel uses in its messages: "a Minimum", "the Minimum"
    /// and "The Minimum", per language.
    fn names(self) -> [[&'static str; 5]; 3] {
        match self {
            Field::Minimum => [
                [
                    "a Minimum",
                    "un minimum",
                    "un mínimo",
                    "ein Minimum",
                    "un valore minimo",
                ],
                [
                    "the Minimum",
                    "Le minimum",
                    "el mínimo",
                    "das Minimum",
                    "Il valore minimo",
                ],
                [
                    "The Minimum",
                    "Le minimum",
                    "el mínimo",
                    "Das Minimum",
                    "Il valore minimo",
                ],
            ],
            Field::Maximum => [
                [
                    "a Maximum",
                    "un maximum",
                    "un máximo",
                    "ein Maximum",
                    "un valore massimo",
                ],
                [
                    "the Maximum",
                    "le maximum",
                    "el máximo",
                    "das Maximum",
                    "il valore massimo",
                ],
                [
                    "The Maximum",
                    "Le maximum",
                    "el máximo",
                    "Das Maximum",
                    "il valore massimo",
                ],
            ],
            Field::Formula => [
                [
                    "a Formula",
                    "une formule",
                    "una fórmula",
                    "eine Funktion",
                    "una formula",
                ],
                ["the Formula", "la formule", "la fórmula", "die Funktion", "la formula"],
                ["The Formula", "La formule", "la fórmula", "Die Funktion", "la formula"],
            ],
            Field::Source => [
                ["a Source", "une source", "un origen", "eine Quelle", "una sorgente"],
                ["the Source", "la source", "el origen", "die Quelle", "l'origine"],
                ["The Source", "La source", "El origen", "Die Quelle", "L'origine"],
            ],
            Field::StartDate => [
                [
                    "a Start Date",
                    "une date de Début",
                    "una fecha de inicio",
                    "ein Anfangsdatum",
                    "una data di inizio",
                ],
                [
                    "the Start Date",
                    "la date de début",
                    "fecha inicial",
                    "das Anfangsdatum",
                    "Data di inizio",
                ],
                [
                    "The Start Date",
                    "La date de début",
                    "fecha inicial",
                    "Das Anfangsdatum",
                    "Data di inizio",
                ],
            ],
            Field::EndDate => [
                [
                    "an End Date",
                    "une date de Fin",
                    "una fecha de finalización",
                    "ein Enddatum",
                    "una data di fine",
                ],
                [
                    "the End Date",
                    "la date de fin",
                    "fecha final",
                    "das Enddatum",
                    "Data di fine",
                ],
                [
                    "The End Date",
                    "La date de fin",
                    "fecha final",
                    "Das Enddatum",
                    "Data di fine",
                ],
            ],
            Field::StartTime => [
                [
                    "a Start Time",
                    "une heure de Début",
                    "una hora de inicio",
                    "eine Anfangszeit",
                    "un'ora di inizio",
                ],
                [
                    "the Start Time",
                    "l'heure de début",
                    "la hora de inicio",
                    "die Anfangszeit",
                    "l'ora di inizio",
                ],
                [
                    "The Start Time",
                    "L'heure de début",
                    "la hora de inicio",
                    "Die Anfangszeit",
                    "L'ora di inizio",
                ],
            ],
            Field::EndTime => [
                [
                    "an End Time",
                    "une heure de Fin",
                    "una hora de finalización",
                    "eine Endzeit",
                    "un'ora di fine",
                ],
                [
                    "the End Time",
                    "l'heure de fin",
                    "la hora de finalización",
                    "die Endzeit",
                    "l'ora di fine",
                ],
                [
                    "The End Time",
                    "L'heure de fin",
                    "la hora de finalización",
                    "Die Endzeit",
                    "L'ora di fine",
                ],
            ],
            Field::Value => [
                ["a Value", "une valeur", "un valor", "ein Wert", "un valore"],
                ["the Value", "la valeur", "el valor", "der Wert", "il valore"],
                ["The Value", "la valeur", "el valor", "Der Wert", "Il valore"],
            ],
            Field::Date => [
                ["a Date", "une date", "una fecha", "ein Datum", "una data"],
                ["the Date", "la date", "la fecha", "das Datum", "la data"],
                ["The Date", "la date", "la fecha", "Das Datum", "La data"],
            ],
            Field::Time => [
                ["a Time", "une heure", "una hora", "eine Zeit", "un'ora"],
                ["the Time", "l'heure", "la hora", "die Zeit", "l'ora"],
                ["The Time", "l'heure", "la hora", "Die Zeit", "L'ora"],
            ],
            Field::Length => [
                ["a Length", "une longueur", "un largo", "eine Länge", "una lunghezza"],
                ["the Length", "la longueur", "el largo", "die Länge", "la lunghezza"],
                ["The Length", "la longueur", "El largo", "Die Länge", "La lunghezza"],
            ],
        }
    }

    fn a(self, locale: &Locale) -> &'static str {
        localized(locale, self.names()[0])
    }

    fn the(self, locale: &Locale) -> &'static str {
        localized(locale, self.names()[1])
    }

    fn capitalized(self, locale: &Locale) -> &'static str {
        localized(locale, self.names()[2])
    }
}

/// The boxes the dialog shows for a kind of rule and an operator: the one
/// filling `formula1` and the one filling `formula2`.
pub fn operand_fields(kind: ValidationKind, operator: Operator) -> (Option<Field>, Option<Field>) {
    let (low, high, single) = match kind {
        ValidationKind::Any => return (None, None),
        ValidationKind::List => return (Some(Field::Source), None),
        ValidationKind::Custom => return (Some(Field::Formula), None),
        ValidationKind::Whole | ValidationKind::Decimal => (Field::Minimum, Field::Maximum, Field::Value),
        ValidationKind::Date => (Field::StartDate, Field::EndDate, Field::Date),
        ValidationKind::Time => (Field::StartTime, Field::EndTime, Field::Time),
        ValidationKind::TextLength => (Field::Minimum, Field::Maximum, Field::Length),
    };
    match operator {
        Operator::Between | Operator::NotBetween => (Some(low), Some(high)),
        Operator::Equal | Operator::NotEqual => (Some(single), None),
        Operator::GreaterThan | Operator::GreaterOrEqual => (Some(low), None),
        Operator::LessThan | Operator::LessOrEqual => (Some(high), None),
    }
}

/// Why the dialog refuses a rule.
#[derive(Clone, Debug, PartialEq)]
pub enum RuleError {
    /// A box was left empty.
    Missing(Field),
    /// Both boxes were left empty.
    MissingBoth(Field, Field),
    /// Neither a number nor a formula.
    NotNumeric(Field),
    InvalidDate(Field),
    InvalidTime(Field),
    /// A number with decimals where only whole numbers make sense.
    Decimal(ValidationKind),
    /// A negative text length.
    Negative(ValidationKind),
    /// The upper bound (first) is below the lower bound (second).
    MaxBelowMin(Field, Field),
    /// The list source is a block of several rows and columns.
    ListSource,
    /// The formula does not parse.
    Formula(ParseError),
}

impl RuleError {
    /// Excel's message for the error.
    pub fn message(&self, locale: &Locale) -> String {
        let fill = |texts: [&'static str; 5], with: &str| localized(locale, texts).replace('|', with);
        match self {
            RuleError::Missing(field) => fill(
                ["You must enter |.", "Vous devez taper |.", "Debe introducir |.", "Sie müssen | eingeben.", "Immettere |."],
                field.a(locale),
            ),
            RuleError::MissingBoth(first, second) => localized(
                locale,
                [
                    "You must enter both |0 and |1.",
                    "Tapez |0 et aussi |1.",
                    "Debe introducir |0 y |1.",
                    "Sie müssen |0 und |1 eingeben.",
                    "È necessario immettere sia |0 che |1.",
                ],
            )
            .replace("|0", first.a(locale))
            .replace("|1", second.a(locale)),
            RuleError::NotNumeric(field) => fill(
                [
                    "| must be a numeric value, numeric expression, or refer to a cell containing a numeric value.",
                    "| doit être une valeur numérique, une expression numérique ou faire référence à une cellule contenant une valeur numérique.",
                    "| debe ser un valor numérico, una expresión numérica o referirse a una celda que contenga un valor numérico.",
                    "| muss ein numerischer Wert, numerischer Ausdruck oder ein Bezug auf eine Zelle, die einen numerischen Wert enthält, sein.",
                    "| deve essere un valore numerico un'espressione numerica o un riferimento di cella contenente un valore numerico.",
                ],
                field.capitalized(locale),
            ),
            RuleError::InvalidDate(field) => fill(
                [
                    "The date you entered for | is invalid.",
                    "La date que vous avez tapée pour | n'est pas valide.",
                    "La fecha introducida para | no es válida.",
                    "Das für | eingegebene Datum ist ungültig.",
                    "La data immessa per | non è valida.",
                ],
                field.the(locale),
            ),
            RuleError::InvalidTime(field) => fill(
                [
                    "The time you entered for | is invalid.",
                    "Le temps que vous avez saisi pour | n'est pas valide.",
                    "El tiempo introducido para | no es válido.",
                    "Die für | eingegebene Zeit ist ungültig.",
                    "L'ora immessa per | non è valida.",
                ],
                field.the(locale),
            ),
            RuleError::Decimal(kind) => fill(
                [
                    "Decimal values cannot be used for | conditions.",
                    "Impossible d'utiliser des valeurs décimales pour des conditions |.",
                    "No se pueden utilizar valores decimales para | condiciones.",
                    "Dezimale Werte können für | Bedingungen nicht verwendet werden.",
                    "I valori decimali non possono essere usati per |.",
                ],
                kind.label(locale),
            ),
            RuleError::Negative(kind) => fill(
                [
                    "Negative values cannot be used for | conditions.",
                    "Les valeurs négatives ne peuvent être utilisées comme | conditions.",
                    "No se pueden usar valores negativos para | condiciones.",
                    "Negative Werte können für | Bedingungen nicht verwendet werden.",
                    "I valori negativi non sono utilizzabili per le condizioni |.",
                ],
                kind.label(locale),
            ),
            RuleError::MaxBelowMin(max, min) => localized(
                locale,
                [
                    "|0 must be greater than or equal to |1.",
                    "|0 doit être supérieur ou égal à |1.",
                    "|0 debe ser igual o mayor que |1.",
                    "|0 muss größer oder gleich sein als |1.",
                    "|0 deve essere maggiore o uguale a |1.",
                ],
            )
            .replace("|0", max.capitalized(locale))
            .replace("|1", min.the(locale)),
            RuleError::ListSource => localized(
                locale,
                [
                    "The list source must be a delimited list, or a reference to single row or column.",
                    "La liste Source doit être délimitée ou faire référence à une seule ligne ou colonne.",
                    "La fuente de la lista debe ser una lista delimitada o bien una referencia a una fila o columna individual.",
                    "Die Quelle muss eine getrennte Liste oder ein Bezug auf eine einzelne Zeile oder Spalte sein.",
                    "L'origine dell'elenco deve essere un elenco delimitato o un riferimento ad una singola riga o colonna.",
                ],
            )
            .to_string(),
            RuleError::Formula(error) => error.message.clone(),
        }
    }
}

/// What is wrong, for now, with the formulas typed in the dialog: found
/// when OK is pressed (`Workbook::rule_formula_warning`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormulaWarning {
    /// The list source is a name nobody defined: the rule is refused.
    UnknownName,
    /// The formula of this box gives an error; the user may go on.
    Error(Field),
}

impl FormulaWarning {
    /// Excel's message: a refusal, or the question whether to continue.
    pub fn message(self, locale: &Locale) -> String {
        match self {
            FormulaWarning::UnknownName => localized(
                locale,
                [
                    "A named range you specified cannot be found.",
                    "Nous ne trouvons pas une plage nommée que vous avez spécifiée.",
                    "No se puede encontrar uno de los rangos especificados.",
                    "Der angegebene benannte Bereich wurde nicht gefunden.",
                    "L'intervallo denominato specificato non è stato trovato.",
                ],
            )
            .to_string(),
            FormulaWarning::Error(field) => localized(
                locale,
                [
                    "| currently evaluates to an error. Do you want to continue?",
                    "| est reconnu comme erroné. Voulez-vous continuer ?",
                    "| actualmente evalúa un error. ¿Desea continuar?",
                    "| untersucht gerade einen möglichen Fehler. Möchten Sie den Vorgang fortsetzen?",
                    "| restituisce attualmente un errore. Continuare?",
                ],
            )
            .replace('|', field.capitalized(locale)),
        }
    }
}

/// A constant written as a number, possibly signed.
fn constant_number(expr: &Expr) -> Option<f64> {
    match expr {
        Expr::Number(n) => Some(*n),
        Expr::Unary(UnaryOp::Neg, inner) => constant_number(inner).map(|n| -n),
        Expr::Unary(UnaryOp::Plus, inner) | Expr::Paren(inner) => constant_number(inner),
        _ => None,
    }
}

/// Read what was typed in one box of the dialog. `=...` is a formula; in
/// the Source box anything else is a list separated by the list separator
/// of the language; elsewhere a number, a date or a time as the kind of
/// rule expects, or a formula typed without its `=`. The formula reads as
/// typed, that is from the active cell.
pub fn parse_operand(kind: ValidationKind, field: Field, text: &str, locale: &Locale) -> Result<Expr, RuleError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(RuleError::Missing(field));
    }
    if let Some(formula) = text.strip_prefix('=') {
        return parse_formula(formula, locale).map_err(RuleError::Formula);
    }
    match kind {
        ValidationKind::List => {
            let items: Vec<&str> = text.split(locale.arg_sep).map(str::trim).collect();
            Ok(Expr::Text(Arc::from(items.join(","))))
        }
        ValidationKind::Any | ValidationKind::Custom => parse_formula(text, locale).map_err(RuleError::Formula),
        _ => {
            let typed = locale.parse_number(text).or_else(|| match kind {
                ValidationKind::Date | ValidationKind::Time => input::parse_date_time(text, locale).map(|(n, _)| n),
                _ => None,
            });
            if let Some(n) = typed {
                let whole_only = matches!(kind, ValidationKind::Whole | ValidationKind::TextLength);
                if whole_only && n.fract() != 0.0 {
                    return Err(RuleError::Decimal(kind));
                }
                if kind == ValidationKind::TextLength && n < 0.0 {
                    return Err(RuleError::Negative(kind));
                }
                return Ok(Expr::Number(n));
            }
            // A reference or a function typed without its `=`.
            match parse_formula(text, locale) {
                Ok(expr) if !matches!(expr, Expr::Name(_) | Expr::Text(_)) => Ok(expr),
                _ => Err(match kind {
                    ValidationKind::Date => RuleError::InvalidDate(field),
                    ValidationKind::Time => RuleError::InvalidTime(field),
                    _ => RuleError::NotNumeric(field),
                }),
            }
        }
    }
}

/// The text the dialog shows for a formula of a rule: a typed list with
/// the list separator of the language, a constant as a number, a date or a
/// time, anything else as a formula with its `=`.
pub fn operand_text(kind: ValidationKind, expr: &Expr, locale: &Locale) -> String {
    if let (ValidationKind::List, Expr::Text(items)) = (kind, expr) {
        return items.replace(',', &locale.arg_sep.to_string());
    }
    match constant_number(expr) {
        Some(n) if matches!(kind, ValidationKind::Date | ValidationKind::Time) => {
            input::edit_text(&Content::Value(Value::Number(n)), locale.short_date_format, locale)
        }
        Some(n) => locale.number_to_text(n),
        None => format!("={}", expr.to_text(locale)),
    }
}

/// What the user is told when an entry breaks its rule.
#[derive(Clone, Debug, PartialEq)]
pub struct Alert {
    pub style: AlertStyle,
    /// The title of the rule; empty when it has none (the application
    /// then shows its own name, as Excel does).
    pub title: String,
    /// The message of the rule, or Excel's own when it has none; a warning
    /// ends with the question whether to continue.
    pub message: String,
}

/// The verdict on an entry.
#[derive(Clone, Debug, PartialEq)]
pub enum EntryCheck {
    Accepted,
    /// The entry breaks the rule of its cell: show the alert, then keep or
    /// drop the entry as its style allows.
    Invalid(Alert),
}

/// One line of the drop-down list of a cell.
#[derive(Clone, Debug, PartialEq)]
pub struct ListItem {
    /// The line as shown: the source cell as it is displayed.
    pub text: String,
    /// What choosing the line puts in the cell.
    pub value: Value,
}

/// What the rules of a selection have in common (decides how the dialog
/// opens).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SelectionValidation<'a> {
    /// No cell has a rule.
    None,
    /// Every cell has this rule.
    Uniform(&'a Validation),
    /// Some cells have this rule, the others none.
    Partial(&'a Validation),
    /// The cells have different rules.
    Mixed,
}

impl SelectionValidation<'_> {
    /// The question Excel asks before opening the dialog: whether to
    /// extend the rule to the cells without one (No edits the cells that
    /// have it), or whether to erase the rules and go on.
    pub fn question(&self, locale: &Locale) -> Option<&'static str> {
        match self {
            SelectionValidation::None | SelectionValidation::Uniform(_) => None,
            SelectionValidation::Partial(_) => Some(localized(
                locale,
                [
                    "The selection contains some cells without Data Validation settings. Do you want to extend Data Validation to these cells?",
                    "La sélection contient des cellules sans paramètres de validation de données. Voulez-vous étendre la validation de données à ces cellules ?",
                    "La selección contiene algunas celdas sin configuración para validación de datos. ¿Desea extender la validación de datos a estas celdas?",
                    "Die Auswahl enthält einige Zellen ohne Einstellungen für die Datenüberprüfung. Soll die Datenüberprüfung auf diese Zellen erweitert werden?",
                    "La selezione contiene alcune celle prive di impostazioni per la convalida dei dati. Estendere la convalida dati alle celle?",
                ],
            )),
            SelectionValidation::Mixed => Some(localized(
                locale,
                [
                    "The selection contains more than one type of validation.\n\nErase current settings and continue?",
                    "La sélection contient plus d'un type de validation.\n\nEffacer les réglages courants puis continuer ?",
                    "La selección contiene más de un tipo de validación.\n\n¿Desea eliminar la configuración actual y continuar?",
                    "Die Auswahl enthält mehr als eine Prüfungsart.\n\nSollen die aktuellen Einstellungen gelöscht und dann fortgefahren werden?",
                    "La selezione contiene più di un tipo di convalida.\n\nCancellare le impostazioni correnti e continuare?",
                ],
            )),
        }
    }
}

/// The cells "Circle Invalid Data" marks.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InvalidCells {
    /// In sheet order, `MAX_CIRCLES` at most.
    pub cells: Vec<(u32, u32)>,
    /// There are more invalid cells than Excel marks.
    pub truncated: bool,
}

/// Excel circles this many invalid cells and no more.
pub const MAX_CIRCLES: usize = 255;

/// Filling a pattern of rules over a large range gives a rule this many
/// ranges at most; the cells beyond keep no rule.
const MAX_TILED_RANGES: usize = 4096;

/// A drop-down list shows this many lines at most, as in Excel.
pub const MAX_LIST_ITEMS: usize = 32_767;

impl InvalidCells {
    /// What Excel says when it could not mark every cell.
    pub fn warning(&self, locale: &Locale) -> Option<&'static str> {
        self.truncated.then(|| {
            localized(
                locale,
                [
                    "Your worksheet contains more than 255 invalid cells. Only the first 255 cells will be marked.",
                    "Votre feuille contient plus de 255 cellules non valides. Seules les 255 premières cellules seront marquées.",
                    "La hoja de cálculo contiene más de 255 celdas no válidas. Solo se marcarán las primeras 255 celdas.",
                    "Das Arbeitsblatt enthält mehr als 255 ungültige Zellen. Nur die ersten 255 Zellen wurden markiert.",
                    "Il foglio di lavoro contiene più di 255 celle non valide. Verranno contrassegnate solo le prime 255 celle.",
                ],
            )
        })
    }
}

/// The parts of `rect` outside `cut`: above, below, left, right.
fn subtract(rect: Rect, cut: &Rect) -> Vec<Rect> {
    let Some(common) = rect.intersection(cut) else {
        return vec![rect];
    };
    let mut out = Vec::new();
    if common.r0 > rect.r0 {
        out.push(Rect::new(rect.r0, rect.c0, common.r0 - 1, rect.c1));
    }
    if common.r1 < rect.r1 {
        out.push(Rect::new(common.r1 + 1, rect.c0, rect.r1, rect.c1));
    }
    if common.c0 > rect.c0 {
        out.push(Rect::new(common.r0, rect.c0, common.r1, common.c0 - 1));
    }
    if common.c1 < rect.c1 {
        out.push(Rect::new(common.r0, common.c1 + 1, common.r1, rect.c1));
    }
    out
}

/// `rects` without the cells of `cuts`.
fn subtract_all(rects: &[Rect], cuts: &[Rect]) -> Vec<Rect> {
    let mut out = rects.to_vec();
    for cut in cuts {
        out = out.iter().flat_map(|r| subtract(*r, cut)).collect();
    }
    out
}

/// Join rectangles that together form a rectangle and drop those inside
/// another, keeping the order of the list.
fn coalesce(rects: &mut Vec<Rect>) {
    let joined = |a: &Rect, b: &Rect| -> Option<Rect> {
        let union = a.union(b);
        let inside = union == *a || union == *b;
        let stacked = a.c0 == b.c0 && a.c1 == b.c1 && a.r0 <= b.r1.saturating_add(1) && b.r0 <= a.r1.saturating_add(1);
        let side_by_side =
            a.r0 == b.r0 && a.r1 == b.r1 && a.c0 <= b.c1.saturating_add(1) && b.c0 <= a.c1.saturating_add(1);
        (inside || stacked || side_by_side).then_some(union)
    };
    let mut merged = true;
    while merged {
        merged = false;
        'search: for i in 0..rects.len() {
            for j in i + 1..rects.len() {
                if let Some(union) = joined(&rects[i], &rects[j]) {
                    rects[i] = union;
                    rects.remove(j);
                    merged = true;
                    break 'search;
                }
            }
        }
    }
}

/// How one side of a range follows rows or columns inserted or deleted:
/// the new interval and where its first cell was before the edit. `None`
/// when the whole interval was deleted.
fn adjust_span(lo: u32, hi: u32, insert: bool, at: u32, count: u32, max: u32) -> Option<(u32, u32, u32)> {
    if insert {
        let new_lo = if lo >= at { lo + count } else { lo };
        // Cells inserted right after the last one take its rule, as they
        // take its formatting.
        let new_hi = if hi + 1 >= at { hi + count } else { hi };
        (new_lo < max).then(|| (new_lo, new_hi.min(max - 1), lo))
    } else {
        let end = at + count - 1;
        if lo >= at && hi <= end {
            return None;
        }
        let (new_lo, origin) = if lo > end {
            (lo - count, lo)
        } else if lo >= at {
            (at, end + 1)
        } else {
            (lo, lo)
        };
        let new_hi = if hi > end {
            hi - count
        } else if hi >= at {
            at - 1
        } else {
            hi
        };
        Some((new_lo, new_hi, origin))
    }
}

/// A range after a structural edit, and where its top-left cell was.
fn adjust_rect(rect: Rect, edit: StructuralEdit) -> Option<(Rect, (u32, u32))> {
    match edit {
        StructuralEdit::InsertRows { at, count } | StructuralEdit::DeleteRows { at, count } => {
            let insert = matches!(edit, StructuralEdit::InsertRows { .. });
            let (r0, r1, origin) = adjust_span(rect.r0, rect.r1, insert, at, count, MAX_ROWS)?;
            Some((Rect::new(r0, rect.c0, r1, rect.c1), (origin, rect.c0)))
        }
        StructuralEdit::InsertCols { at, count } | StructuralEdit::DeleteCols { at, count } => {
            let insert = matches!(edit, StructuralEdit::InsertCols { .. });
            let (c0, c1, origin) = adjust_span(rect.c0, rect.c1, insert, at, count, MAX_COLS)?;
            Some((Rect::new(rect.r0, c0, rect.r1, c1), (rect.r0, origin)))
        }
    }
}

/// Whether two values are the same list entry: same type, text compared
/// without case.
fn same_entry(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Empty, Value::Empty) => true,
        (Value::Number(x), Value::Number(y)) => x == y,
        (Value::Text(x), Value::Text(y)) => compare_text(x, y) == Ordering::Equal,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Error(x), Value::Error(y)) => x == y,
        _ => false,
    }
}

/// The entries of a list typed in the dialog.
fn typed_items(list: &str) -> impl Iterator<Item = &str> {
    list.split(',').map(str::trim).filter(|item| !item.is_empty())
}

/// What an entry of a typed list puts in the cell: what typing it would.
fn typed_item_value(item: &str, locale: &Locale) -> Value {
    match input::parse_input(item, locale) {
        Ok(input::ParsedInput {
            content: Content::Value(value),
            ..
        }) => value,
        _ => Value::text(item),
    }
}

impl Validation {
    /// The rule a `<dataValidation>` without attributes stands for, which
    /// a reader starts from: blanks are not ignored and neither the input
    /// message nor the error alert is shown.
    pub fn file_default() -> Validation {
        Validation {
            allow_blank: false,
            show_input: false,
            show_error: false,
            ..Validation::default()
        }
    }

    /// The cell the formulas are written for.
    pub fn anchor(&self) -> Option<(u32, u32)> {
        self.ranges.first().map(|r| (r.r0, r.c0))
    }

    pub fn contains(&self, row: u32, col: u32) -> bool {
        self.ranges.iter().any(|r| r.contains(row, col))
    }

    /// A rule that restricts nothing and says nothing: the same as none.
    pub fn is_noop(&self) -> bool {
        self.kind == ValidationKind::Any && self.input_title.is_empty() && self.input_message.is_empty()
    }

    fn formula_at(&self, formula: &Option<Arc<Expr>>, row: u32, col: u32) -> Option<Expr> {
        let (r0, c0) = self.anchor().unwrap_or((row, col));
        formula
            .as_ref()
            .map(|e| e.shifted(row as i64 - r0 as i64, col as i64 - c0 as i64))
    }

    /// `formula1` as it reads in a cell of the rule.
    pub fn formula1_at(&self, row: u32, col: u32) -> Option<Expr> {
        self.formula_at(&self.formula1, row, col)
    }

    /// `formula2` as it reads in a cell of the rule.
    pub fn formula2_at(&self, row: u32, col: u32) -> Option<Expr> {
        self.formula_at(&self.formula2, row, col)
    }

    /// What the first box of the dialog shows when (`row`, `col`) is the
    /// active cell; empty when the rule has no first formula.
    pub fn operand1_text(&self, row: u32, col: u32, locale: &Locale) -> String {
        self.formula1_at(row, col)
            .map(|e| operand_text(self.kind, &e, locale))
            .unwrap_or_default()
    }

    /// What the second box of the dialog shows.
    pub fn operand2_text(&self, row: u32, col: u32, locale: &Locale) -> String {
        self.formula2_at(row, col)
            .map(|e| operand_text(self.kind, &e, locale))
            .unwrap_or_default()
    }

    /// Fill the formulas from the boxes of the dialog, with the checks the
    /// dialog makes on OK. The formulas read as typed (from the active
    /// cell): `Workbook::set_validation` moves them to the anchor.
    pub fn set_operands(&mut self, first: &str, second: &str, locale: &Locale) -> Result<(), RuleError> {
        let (field1, field2) = operand_fields(self.kind, self.operator);
        let empty = |text: &str| text.trim().is_empty();
        match (field1, field2) {
            (Some(a), Some(b)) if empty(first) && empty(second) => return Err(RuleError::MissingBoth(a, b)),
            _ => {}
        }
        let parse = |field: Option<Field>, text: &str| {
            field
                .map(|f| parse_operand(self.kind, f, text, locale).map(Arc::new))
                .transpose()
        };
        let formula1 = parse(field1, first)?;
        let formula2 = parse(field2, second)?;
        if let (Some(min), Some(max)) = (field1, field2) {
            let low = formula1.as_deref().and_then(constant_number);
            let high = formula2.as_deref().and_then(constant_number);
            if low.zip(high).is_some_and(|(low, high)| high < low) {
                return Err(RuleError::MaxBelowMin(max, min));
            }
        }
        if self.kind == ValidationKind::List {
            let mut source = formula1.as_deref();
            while let Some(Expr::Paren(inner)) = source {
                source = Some(inner);
            }
            if matches!(source, Some(Expr::Ref(r)) if r.rect().rows() > 1 && r.rect().cols() > 1) {
                return Err(RuleError::ListSource);
            }
        }
        self.formula1 = formula1;
        self.formula2 = formula2;
        Ok(())
    }

    /// Whether the two rules say the same thing, whatever their ranges
    /// (formulas are compared as they read from the same cell).
    pub fn same_settings(&self, other: &Validation) -> bool {
        let (row, col) = self.anchor().or(other.anchor()).unwrap_or((0, 0));
        self.kind == other.kind
            && self.operator == other.operator
            && self.allow_blank == other.allow_blank
            && self.in_cell_dropdown == other.in_cell_dropdown
            && self.show_input == other.show_input
            && self.input_title == other.input_title
            && self.input_message == other.input_message
            && self.show_error == other.show_error
            && self.error_style == other.error_style
            && self.error_title == other.error_title
            && self.error_message == other.error_message
            && self.ime_mode == other.ime_mode
            && self.formula1_at(row, col) == other.formula1_at(row, col)
            && self.formula2_at(row, col) == other.formula2_at(row, col)
    }

    /// The alert shown when an entry breaks the rule.
    pub fn alert(&self, locale: &Locale) -> Alert {
        let own = !self.error_message.is_empty();
        let mut message = if own {
            self.error_message.clone()
        } else {
            localized(
                locale,
                [
                    "This value doesn't match the data validation restrictions defined for this cell.",
                    "Cette valeur ne correspond pas aux restrictions de validation des données pour cette cellule.",
                    "Este valor no coincide con las restricciones de validación de datos definidas para esta celda.",
                    "Dieser Wert entspricht nicht den Einschränkungen für die Datenüberprüfung, die für diese Zelle definiert sind.",
                    "Questo valore non corrisponde alle limitazioni di convalida dei dati definite per questa cella.",
                ],
            )
            .to_string()
        };
        if self.error_style == AlertStyle::Warning {
            message.push_str(if own { "\n\n" } else { "\n" });
            message.push_str(localized(
                locale,
                [
                    "Continue?",
                    "Voulez-vous continuer\u{a0}?",
                    "¿Desea continuar?",
                    "Fortsetzen?",
                    "Continuare?",
                ],
            ));
        }
        Alert {
            style: self.error_style,
            title: self.error_title.clone(),
            message,
        }
    }

    fn map_formulas(&mut self, f: impl Fn(&Expr) -> Expr) {
        for formula in [&mut self.formula1, &mut self.formula2].into_iter().flatten() {
            let mapped = f(formula);
            if mapped != **formula {
                *formula = Arc::new(mapped);
            }
        }
    }

    /// Change the ranges; the formulas are rewritten for the new anchor.
    fn set_ranges(&mut self, ranges: Vec<Rect>) {
        let before = self.anchor();
        self.ranges = ranges;
        if let (Some(old), Some(new)) = (before, self.anchor()) {
            let (d_row, d_col) = (new.0 as i64 - old.0 as i64, new.1 as i64 - old.1 as i64);
            self.map_formulas(|e| e.shifted(d_row, d_col));
        }
    }
}

/// Take the cells of `cuts` away from every rule of the sheet. Returns
/// whether any rule changed.
fn remove_from(sheet: &mut Sheet, cuts: &[Rect]) -> bool {
    let mut changed = false;
    for rule in &mut sheet.validations {
        if rule.ranges.iter().any(|r| cuts.iter().any(|cut| cut.intersects(r))) {
            rule.set_ranges(subtract_all(&rule.ranges, cuts));
            changed = true;
        }
    }
    sheet.validations.retain(|rule| !rule.ranges.is_empty());
    changed
}

/// Add a rule whose formulas are written for the top-left cell of its
/// first range; it joins a rule with the same settings when there is one.
fn store(sheet: &mut Sheet, rule: Validation) {
    match sheet
        .validations
        .iter_mut()
        .find(|existing| existing.same_settings(&rule))
    {
        Some(existing) => {
            let mut ranges = existing.ranges.clone();
            ranges.extend(rule.ranges);
            coalesce(&mut ranges);
            existing.set_ranges(ranges);
        }
        None => sheet.validations.push(rule),
    }
}

impl Workbook {
    /// The rule of a cell.
    pub fn validation_at(&self, sheet: usize, row: u32, col: u32) -> Option<&Validation> {
        self.sheets
            .get(sheet)?
            .validations
            .iter()
            .find(|rule| rule.contains(row, col))
    }

    /// The input message to show while a cell is selected: (title, text).
    pub fn input_message(&self, sheet: usize, row: u32, col: u32) -> Option<(&str, &str)> {
        let rule = self.validation_at(sheet, row, col)?;
        (rule.show_input && !(rule.input_title.is_empty() && rule.input_message.is_empty()))
            .then_some((rule.input_title.as_str(), rule.input_message.as_str()))
    }

    /// Whether a cell shows the arrow of a drop-down list when selected.
    pub fn has_validation_dropdown(&self, sheet: usize, row: u32, col: u32) -> bool {
        self.validation_at(sheet, row, col)
            .is_some_and(|rule| rule.kind == ValidationKind::List && rule.in_cell_dropdown)
    }

    /// What the rules of the selected cells have in common.
    pub fn selection_validation(&self, sheet: usize, ranges: &[Rect]) -> SelectionValidation<'_> {
        let Some(sh) = self.sheets.get(sheet) else {
            return SelectionValidation::None;
        };
        let mut found: Option<&Validation> = None;
        let mut covered = Vec::new();
        for rule in &sh.validations {
            if !rule.ranges.iter().any(|r| ranges.iter().any(|sel| sel.intersects(r))) {
                continue;
            }
            match found {
                Some(first) if !first.same_settings(rule) => return SelectionValidation::Mixed,
                Some(_) => {}
                None => found = Some(rule),
            }
            covered.extend(rule.ranges.iter().copied());
        }
        match found {
            None => SelectionValidation::None,
            Some(rule) if subtract_all(ranges, &covered).is_empty() => SelectionValidation::Uniform(rule),
            Some(rule) => SelectionValidation::Partial(rule),
        }
    }

    /// Every range that has the same rule as a cell ("Apply these changes
    /// to all other cells with the same settings").
    pub fn validation_ranges_like(&self, sheet: usize, row: u32, col: u32) -> Vec<Rect> {
        let Some(rule) = self.validation_at(sheet, row, col) else {
            return Vec::new();
        };
        let mut ranges: Vec<Rect> = self.sheets[sheet]
            .validations
            .iter()
            .filter(|other| other.same_settings(rule))
            .flat_map(|other| other.ranges.iter().copied())
            .collect();
        coalesce(&mut ranges);
        ranges
    }

    /// Give `ranges` a rule, replacing whatever rules their cells had.
    /// The formulas of `rule` read as typed with `active` as the active
    /// cell; `rule.ranges` is ignored. A rule that restricts nothing and
    /// has no input message clears instead. Recorded for undo.
    pub fn set_validation(&mut self, sheet: usize, ranges: &[Rect], active: (u32, u32), mut rule: Validation) -> bool {
        if sheet >= self.sheets.len() || ranges.is_empty() {
            return false;
        }
        self.snapshot();
        let sh = &mut self.sheets[sheet];
        remove_from(sh, ranges);
        if rule.is_noop() {
            return true;
        }
        // No cell twice, however the selection overlaps itself.
        let mut list: Vec<Rect> = Vec::new();
        for range in ranges {
            let fresh = subtract_all(&[*range], &list);
            list.extend(fresh);
        }
        coalesce(&mut list);
        // Moving from the active cell to the anchor rewrites the formulas.
        rule.ranges = vec![Rect::cell(active.0, active.1)];
        rule.set_ranges(list);
        store(sh, rule);
        true
    }

    /// Remove the rules of `ranges` ("Clear All"). Returns false when
    /// there was none. Recorded for undo.
    pub fn clear_validation(&mut self, sheet: usize, ranges: &[Rect]) -> bool {
        let touched = self.sheets.get(sheet).is_some_and(|sh| {
            sh.validations
                .iter()
                .any(|rule| rule.ranges.iter().any(|r| ranges.iter().any(|cut| cut.intersects(r))))
        });
        if !touched {
            return false;
        }
        self.snapshot();
        remove_from(&mut self.sheets[sheet], ranges)
    }

    /// Give the block at (`dest_row`, `dest_col`) the rules of the block
    /// `src`, cell for cell, as pasting does (Paste, Paste Special ▸
    /// Validation): cells without a rule clear the rule of their
    /// destination. Recorded for undo.
    pub fn copy_validation(&mut self, src_sheet: usize, src: Rect, dest_sheet: usize, dest_row: u32, dest_col: u32) {
        if src_sheet >= self.sheets.len() || dest_sheet >= self.sheets.len() {
            return;
        }
        let rows = src.rows().min(MAX_ROWS - dest_row);
        let cols = src.cols().min(MAX_COLS - dest_col);
        let dest = Rect::new(dest_row, dest_col, dest_row + rows - 1, dest_col + cols - 1);
        let (d_row, d_col) = (dest_row as i64 - src.r0 as i64, dest_col as i64 - src.c0 as i64);
        let mut copies = Vec::new();
        for rule in &self.sheets[src_sheet].validations {
            for piece in rule.ranges.iter().filter_map(|r| r.intersection(&src)) {
                let moved = Rect::new(
                    (piece.r0 as i64 + d_row) as u32,
                    (piece.c0 as i64 + d_col) as u32,
                    (piece.r1 as i64 + d_row) as u32,
                    (piece.c1 as i64 + d_col) as u32,
                );
                let Some(moved) = moved.intersection(&dest) else {
                    continue;
                };
                // The copy says in its cells what the original says in
                // its own: the formulas keep their relative reading.
                let mut copy = rule.clone();
                copy.set_ranges(vec![Rect::cell(
                    (moved.r0 as i64 - d_row) as u32,
                    (moved.c0 as i64 - d_col) as u32,
                )]);
                copy.map_formulas(|e| e.shifted(d_row, d_col));
                copy.ranges = vec![moved];
                copies.push(copy);
            }
        }
        let has_rules = self.sheets[dest_sheet]
            .validations
            .iter()
            .any(|rule| rule.ranges.iter().any(|r| r.intersects(&dest)));
        if copies.is_empty() && !has_rules {
            return;
        }
        self.snapshot();
        let sh = &mut self.sheets[dest_sheet];
        remove_from(sh, &[dest]);
        for copy in copies {
            store(sh, copy);
        }
    }

    /// Lay the rules of the block `src` over `dest` like a wallpaper, one
    /// copy of the block having its top-left cell at `origin`: what a
    /// paste repeated over a larger range does (`origin` is the top-left
    /// of `dest`) and what filling does (`dest` contains `src`, `origin`
    /// is the top-left of `src`). Cells without a rule clear the rule of
    /// their copies. Recorded for undo.
    pub fn tile_validation(&mut self, src_sheet: usize, src: Rect, dest_sheet: usize, dest: Rect, origin: (u32, u32)) {
        if src_sheet >= self.sheets.len() || dest_sheet >= self.sheets.len() {
            return;
        }
        // Where the copies of the rows (or columns) `lo..=hi` of a block
        // of `size` fall in `first..=last`, the pattern starting at `start`.
        let spans = |lo: u32, hi: u32, size: u32, start: u32, first: u32, last: u32| -> Vec<(u32, u32)> {
            if lo == 0 && hi + 1 == size {
                return vec![(first, last)];
            }
            let (lo, hi, size) = (lo as i64, hi as i64, size as i64);
            let (start, first, last) = (start as i64, first as i64, last as i64);
            let mut out = Vec::new();
            let mut tile = (first - start - hi).div_euclid(size);
            while start + tile * size + lo <= last && out.len() < MAX_TILED_RANGES {
                let (a, b) = (start + tile * size + lo, start + tile * size + hi);
                if b >= first {
                    out.push((a.max(first) as u32, b.min(last) as u32));
                }
                tile += 1;
            }
            out
        };
        let mut copies = Vec::new();
        for rule in &self.sheets[src_sheet].validations {
            let mut images: Vec<Rect> = Vec::new();
            for piece in rule.ranges.iter().filter_map(|r| r.intersection(&src)) {
                let rows = spans(piece.r0 - src.r0, piece.r1 - src.r0, src.rows(), origin.0, dest.r0, dest.r1);
                let cols = spans(piece.c0 - src.c0, piece.c1 - src.c0, src.cols(), origin.1, dest.c0, dest.c1);
                for (r0, r1) in &rows {
                    images.extend(cols.iter().map(|(c0, c1)| Rect::new(*r0, *c0, *r1, *c1)));
                }
                images.truncate(MAX_TILED_RANGES);
            }
            coalesce(&mut images);
            let Some(first) = images.first() else {
                continue;
            };
            // The cell of the block the first cell of the copy stands for.
            let turn = |at: u32, start: u32, size: u32| (at as i64 - start as i64).rem_euclid(size as i64) as u32;
            let model = (
                src.r0 + turn(first.r0, origin.0, src.rows()),
                src.c0 + turn(first.c0, origin.1, src.cols()),
            );
            let (d_row, d_col) = (first.r0 as i64 - model.0 as i64, first.c0 as i64 - model.1 as i64);
            // The copy says in its cells what the original says in its
            // own: the formulas keep their relative reading.
            let mut copy = rule.clone();
            copy.set_ranges(vec![Rect::cell(model.0, model.1)]);
            copy.map_formulas(|e| e.shifted(d_row, d_col));
            copy.ranges = images;
            copies.push(copy);
        }
        let has_rules = self.sheets[dest_sheet]
            .validations
            .iter()
            .any(|rule| rule.ranges.iter().any(|r| r.intersects(&dest)));
        if copies.is_empty() && !has_rules {
            return;
        }
        self.snapshot();
        let sh = &mut self.sheets[dest_sheet];
        remove_from(sh, &[dest]);
        for copy in copies {
            store(sh, copy);
        }
    }

    /// Take the rules of the block `src` to (`dest_row`, `dest_col`) with
    /// its cells, as cutting and pasting does: the cells the block leaves
    /// keep no rule. Recorded for undo.
    pub fn move_validation(&mut self, src_sheet: usize, src: Rect, dest_sheet: usize, dest_row: u32, dest_col: u32) {
        if src_sheet >= self.sheets.len() || dest_sheet >= self.sheets.len() {
            return;
        }
        let rows = src.rows().min(MAX_ROWS - dest_row);
        let cols = src.cols().min(MAX_COLS - dest_col);
        let dest = Rect::new(dest_row, dest_col, dest_row + rows - 1, dest_col + cols - 1);
        self.tile_validation(src_sheet, src, dest_sheet, dest, (dest_row, dest_col));
        let left = if src_sheet == dest_sheet { subtract(src, &dest) } else { vec![src] };
        self.clear_validation(src_sheet, &left);
    }

    /// What the dialog asks about before giving cells a rule whose boxes
    /// were just filled in (`Validation::set_operands`, the formulas still
    /// reading from `active`): a list named after nothing, or a formula
    /// that gives an error where it stands.
    pub fn rule_formula_warning(&self, sheet: usize, active: (u32, u32), rule: &Validation) -> Option<FormulaWarning> {
        let (row, col) = active;
        let (field1, field2) = operand_fields(rule.kind, rule.operator);
        let boxes = [(field1, &rule.formula1), (field2, &rule.formula2)];
        for (field, formula) in boxes {
            let (Some(field), Some(formula)) = (field, formula.as_deref()) else {
                continue;
            };
            // A list read from cells is judged entry by entry, not as a whole.
            if rule.kind == ValidationKind::List
                && (matches!(formula, Expr::Text(_)) || self.list_reference(sheet, row, col, formula, 0).is_some())
            {
                continue;
            }
            let value = Calc::eval_once(self, sheet, row, col, formula).into_scalar();
            if rule.kind == ValidationKind::List && matches!(formula, Expr::Name(_)) && value.is_error() {
                return Some(FormulaWarning::UnknownName);
            }
            if value.is_error() {
                return Some(FormulaWarning::Error(field));
            }
        }
        None
    }

    // ---- checking --------------------------------------------------------

    /// Check what the user typed in a cell against its rule, before it is
    /// stored. A formula is judged on its result. Nothing is left changed
    /// (the entry is tried in place, then taken back), but the workbook is
    /// recalculated when the rule depends on other formulas. Text that
    /// does not parse is accepted here and refused by `set_cell_text`.
    pub fn check_entry(&mut self, sheet: usize, row: u32, col: u32, text: &str) -> EntryCheck {
        let Some(rule) = self.validation_at(sheet, row, col).cloned() else {
            return EntryCheck::Accepted;
        };
        if rule.kind == ValidationKind::Any || !rule.show_error {
            return EntryCheck::Accepted;
        }
        // Cells formatted as Text keep whatever is typed verbatim.
        let as_text = self.style_at(sheet, row, col).num_fmt.as_ref() == "@" && !text.is_empty();
        let content = if as_text {
            Content::Value(Value::text(text))
        } else {
            match input::parse_input(text, self.locale) {
                Ok(parsed) => parsed.content,
                Err(_) => return EntryCheck::Accepted,
            }
        };
        if self.try_entry(sheet, row, col, content, &rule) {
            EntryCheck::Accepted
        } else {
            EntryCheck::Invalid(rule.alert(self.locale))
        }
    }

    /// Whether `rule` accepts `content` in a cell. The content is put in
    /// the cell for the time of the check, so that formulas of the rule
    /// that look at the cell see it.
    fn try_entry(&mut self, sheet: usize, row: u32, col: u32, content: Content, rule: &Validation) -> bool {
        let deep = [rule.formula1_at(row, col), rule.formula2_at(row, col)]
            .iter()
            .flatten()
            .any(|formula| self.reads_formulas(sheet, (sheet, row, col), formula, 0));
        let value = match &content {
            Content::Empty => Value::Empty,
            Content::Value(value) => value.clone(),
            // A formula never yields a blank.
            Content::Formula(expr) => match Calc::eval_once(self, sheet, row, col, expr).into_scalar() {
                Value::Empty => Value::Number(0.0),
                value => value,
            },
        };
        let style = self.sheets[sheet].style_id_at(row, col);
        let trial = Cell { content, value, style };
        let previous = self.sheets[sheet].cells.insert((row, col), trial);
        if deep {
            self.recalc();
        }
        let accepted = self.rule_accepts(sheet, row, col, rule);
        match previous {
            Some(cell) => self.sheets[sheet].cells.insert((row, col), cell),
            None => self.sheets[sheet].cells.remove(&(row, col)),
        };
        if deep {
            self.recalc();
        }
        accepted
    }

    /// Whether a formula of a rule may read the result of another formula:
    /// then the workbook must be recalculated before the rule can judge an
    /// entry. `skip` is the cell being judged.
    fn reads_formulas(&self, sheet: usize, skip: (usize, u32, u32), expr: &Expr, depth: u32) -> bool {
        let mut found = false;
        expr.walk(&mut |e| match e {
            Expr::Ref(r) => {
                let target = match &r.sheet {
                    Some(name) => self.sheet_index(name),
                    None => Some(sheet),
                };
                let Some(sh) = target.and_then(|i| self.sheets.get(i)) else {
                    return;
                };
                let rect = r.rect();
                let spilled = |sp: &crate::workbook::Spill| {
                    let (rows, cols) = (sp.array.rows as u32, sp.array.cols as u32);
                    Rect::new(sp.row, sp.col, sp.row + rows - 1, sp.col + cols - 1).intersects(&rect)
                };
                found |= sh.spills.iter().any(spilled)
                    || sh
                        .cells
                        .range((rect.r0, 0)..=(rect.r1, u32::MAX))
                        .any(|((row, col), cell)| {
                            *col >= rect.c0
                                && *col <= rect.c1
                                && matches!(cell.content, Content::Formula(_))
                                && (target.unwrap_or(sheet), *row, *col) != skip
                        });
            }
            Expr::Name(name) => {
                if let Some(definition) = self.defined_name(name) {
                    found |= depth >= 8 || self.reads_formulas(sheet, skip, definition, depth + 1);
                }
            }
            // These reach cells their arguments do not name.
            Expr::Func(name, _) => found |= matches!(name.as_ref(), "OFFSET" | "INDIRECT"),
            _ => {}
        });
        found
    }

    /// Whether the value a cell holds now satisfies its rule. Cells
    /// without a rule are valid.
    pub fn cell_is_valid(&self, sheet: usize, row: u32, col: u32) -> bool {
        self.validation_at(sheet, row, col)
            .is_none_or(|rule| self.rule_accepts(sheet, row, col, rule))
    }

    /// The value of a formula of a rule in a cell. Constants and plain
    /// cell references, the usual cases, are read directly.
    fn operand_value(&self, sheet: usize, row: u32, col: u32, expr: &Expr) -> Value {
        match expr {
            Expr::Number(n) => Value::Number(*n),
            Expr::Text(text) => Value::Text(text.clone()),
            Expr::Bool(b) => Value::Bool(*b),
            Expr::Ref(r) if r.sheet.is_none() && r.rect().is_single() => self.value_at(sheet, r.rect().r0, r.rect().c0),
            _ => Calc::eval_once(self, sheet, row, col, expr).into_scalar(),
        }
    }

    fn rule_accepts(&self, sheet: usize, row: u32, col: u32, rule: &Validation) -> bool {
        let value = self.value_at(sheet, row, col);
        if rule.kind == ValidationKind::Any || (value.is_empty() && rule.allow_blank) {
            return true;
        }
        let locale = self.locale;
        let operand = |formula: Option<Expr>| match formula {
            Some(expr) => self.operand_value(sheet, row, col, &expr),
            None => Value::Empty,
        };
        match rule.kind {
            ValidationKind::Any => true,
            ValidationKind::List => self.list_accepts(sheet, row, col, rule, &value),
            ValidationKind::Custom => match operand(rule.formula1_at(row, col)) {
                Value::Empty => rule.allow_blank,
                Value::Bool(b) => b,
                Value::Number(n) => n != 0.0,
                _ => false,
            },
            _ => {
                let entry = match (rule.kind, &value) {
                    (ValidationKind::TextLength, Value::Text(text)) => Some(text.chars().count() as f64),
                    (ValidationKind::TextLength, Value::Number(n)) => {
                        Some(locale.number_to_text(*n).chars().count() as f64)
                    }
                    (ValidationKind::TextLength, Value::Bool(b)) => Some(locale.bool_name(*b).chars().count() as f64),
                    (ValidationKind::TextLength, Value::Empty) => Some(0.0),
                    (ValidationKind::Whole, Value::Number(n)) => (n.fract() == 0.0).then_some(*n),
                    (ValidationKind::Decimal | ValidationKind::Date | ValidationKind::Time, Value::Number(n)) => {
                        Some(*n)
                    }
                    _ => None,
                };
                // Where blanks count, a value of the wrong kind is refused
                // whatever the bounds are.
                if entry.is_none() && !rule.allow_blank {
                    return false;
                }
                let low = operand(rule.formula1_at(row, col));
                let high = if rule.operator.has_two_operands() {
                    operand(rule.formula2_at(row, col))
                } else {
                    Value::Number(0.0)
                };
                // A bound that refers to an empty cell lets anything in.
                if rule.allow_blank && (low.is_empty() || high.is_empty()) {
                    return true;
                }
                let Some(entry) = entry else {
                    return false;
                };
                let number = |bound: &Value| match bound {
                    Value::Empty => Some(0.0),
                    Value::Number(n) => Some(*n),
                    Value::Bool(b) => Some(*b as u8 as f64),
                    Value::Text(text) => input::parse_numeric_text(text, locale),
                    _ => None,
                };
                match (number(&low), number(&high)) {
                    (Some(low), Some(high)) => rule.operator.test(entry, low, high),
                    _ => false,
                }
            }
        }
    }

    /// The cells a list takes its entries from, when its source is a
    /// range, a name for one or a formula giving one.
    fn list_reference(&self, sheet: usize, row: u32, col: u32, source: &Expr, depth: u32) -> Option<Reference> {
        match source {
            Expr::Paren(inner) => self.list_reference(sheet, row, col, inner, depth),
            Expr::Ref(r) => {
                let target = match &r.sheet {
                    Some(name) => self.sheet_index(name)?,
                    None => sheet,
                };
                Some(Reference::new(target, r.rect()))
            }
            Expr::Name(name) if depth < 8 => self.list_reference(sheet, row, col, self.defined_name(name)?, depth + 1),
            Expr::Func(..) => {
                // Where the result of OFFSET, INDIRECT, INDEX… lies is
                // asked of the functions that describe a reference.
                let ask = |function: &str| {
                    let call = Expr::Func(Arc::from(function), vec![source.clone()]);
                    match Calc::eval_once(self, sheet, row, col, &call).into_scalar() {
                        Value::Number(n) if n >= 1.0 => Some(n as u32),
                        _ => None,
                    }
                };
                let (top, left) = (ask("ROW")? - 1, ask("COLUMN")? - 1);
                let rect = Rect::new(top, left, top + ask("ROWS")? - 1, left + ask("COLUMNS")? - 1);
                Some(Reference::new(ask("SHEET")? as usize - 1, rect))
            }
            _ => None,
        }
    }

    /// The entries of a list that is not typed in the dialog: the cells of
    /// its source in reading order, blank ones included, as Excel lists
    /// them.
    fn list_entries(&self, sheet: usize, row: u32, col: u32, source: &Expr) -> Vec<ListItem> {
        if let Some(reference) = self.list_reference(sheet, row, col, source, 0) {
            let Some(sh) = self.sheets.get(reference.sheet) else {
                return Vec::new();
            };
            let rect = self.clamp_to_used(reference.sheet, reference.rect);
            return (rect.r0..=rect.r1)
                .flat_map(|r| (rect.c0..=rect.c1).map(move |c| (r, c)))
                .take(MAX_LIST_ITEMS)
                .map(|(r, c)| ListItem {
                    text: self.display_text(reference.sheet, r, c).text,
                    value: sh.value_at(r, c),
                })
                .collect();
        }
        let item = |value: &Value| ListItem {
            text: format::format_value(value, "General", self.locale).text,
            value: value.clone(),
        };
        match Calc::eval_once(self, sheet, row, col, source) {
            Value::Error(_) => Vec::new(),
            Value::Array(array) => array.iter().take(MAX_LIST_ITEMS).map(item).collect(),
            value => vec![item(&value)],
        }
    }

    fn list_accepts(&self, sheet: usize, row: u32, col: u32, rule: &Validation, value: &Value) -> bool {
        let Some(source) = rule.formula1_at(row, col) else {
            return true;
        };
        if let Expr::Text(list) = &source {
            // A typed list is matched to the letter, case included.
            return typed_items(list).any(|item| match value {
                Value::Text(text) => text.as_ref() == item,
                Value::Empty => false,
                other => typed_item_value(item, self.locale) == *other,
            });
        }
        let entries = self.list_entries(sheet, row, col, &source);
        // Blank cells in a named list let anything in while blanks are
        // ignored; in a plain range they do not.
        if rule.allow_blank && matches!(source, Expr::Name(_)) && entries.iter().any(|e| e.value.is_empty()) {
            return true;
        }
        entries.iter().any(|entry| same_entry(&entry.value, value))
    }

    /// The lines of the drop-down list of a cell; `None` when its rule is
    /// not a list. Whether the arrow shows is `has_validation_dropdown`.
    pub fn validation_list(&self, sheet: usize, row: u32, col: u32) -> Option<Vec<ListItem>> {
        let rule = self
            .validation_at(sheet, row, col)
            .filter(|rule| rule.kind == ValidationKind::List)?;
        let Some(source) = rule.formula1_at(row, col) else {
            return Some(Vec::new());
        };
        Some(match &source {
            Expr::Text(list) => typed_items(list)
                .map(|item| ListItem {
                    text: item.to_string(),
                    value: typed_item_value(item, self.locale),
                })
                .collect(),
            _ => self.list_entries(sheet, row, col, &source),
        })
    }

    /// The cells whose value breaks their rule ("Circle Invalid Data").
    /// Blank cells count where blanks are not ignored.
    pub fn invalid_cells(&self, sheet: usize) -> InvalidCells {
        let mut out = InvalidCells::default();
        let Some(sh) = self.sheets.get(sheet) else {
            return out;
        };
        'rules: for rule in sh.validations.iter().filter(|rule| rule.kind != ValidationKind::Any) {
            for range in &rule.ranges {
                let mut candidates: Vec<(u32, u32)> = Vec::new();
                if rule.allow_blank {
                    // Only cells showing something can be wrong.
                    let typed = sh.cells.range((range.r0, 0)..=(range.r1, u32::MAX));
                    candidates.extend(
                        typed
                            .map(|(key, _)| *key)
                            .filter(|(_, c)| *c >= range.c0 && *c <= range.c1),
                    );
                    for sp in &sh.spills {
                        let (rows, cols) = (sp.array.rows as u32, sp.array.cols as u32);
                        let block = Rect::new(sp.row, sp.col, sp.row + rows - 1, sp.col + cols - 1);
                        if let Some(part) = block.intersection(range) {
                            candidates
                                .extend((part.r0..=part.r1).flat_map(|r| (part.c0..=part.c1).map(move |c| (r, c))));
                        }
                    }
                    candidates.sort_unstable();
                    candidates.dedup();
                }
                let all = (range.r0..=range.r1).flat_map(|r| (range.c0..=range.c1).map(move |c| (r, c)));
                let cells: Box<dyn Iterator<Item = (u32, u32)>> = if rule.allow_blank {
                    Box::new(candidates.into_iter())
                } else {
                    Box::new(all)
                };
                for (r, c) in cells {
                    if !self.rule_accepts(sheet, r, c, rule) {
                        out.cells.push((r, c));
                        if out.cells.len() > MAX_CIRCLES {
                            break 'rules;
                        }
                    }
                }
            }
        }
        out.cells.sort_unstable();
        out.cells.dedup();
        out.truncated = out.cells.len() > MAX_CIRCLES;
        out.cells.truncate(MAX_CIRCLES);
        out
    }
}

/// Nothing derived from the cells is kept: rules are looked up and checked
/// on demand (see `features::Caches`).
#[derive(Debug, Default)]
pub struct Cache;

/// See `features::on_structural_edit`.
pub(crate) fn on_structural_edit(sheet: &mut Sheet, edit: StructuralEdit, edited_sheet: &str) {
    let own = sheet.name.clone();
    let edited = own.eq_ignore_ascii_case(edited_sheet);
    for rule in &mut sheet.validations {
        if edited {
            let anchor = rule.anchor();
            let mut ranges = Vec::new();
            let mut origin = None;
            for (range, was) in rule.ranges.iter().filter_map(|r| adjust_rect(*r, edit)) {
                origin.get_or_insert(was);
                ranges.push(range);
            }
            // The formulas are first rewritten for the cell that becomes
            // the anchor, then follow the edit like the formula of a cell.
            if let (Some(old), Some(new)) = (anchor, origin) {
                let (d_row, d_col) = (new.0 as i64 - old.0 as i64, new.1 as i64 - old.1 as i64);
                rule.map_formulas(|e| e.shifted(d_row, d_col));
            }
            rule.ranges = ranges;
        }
        rule.map_formulas(|e| e.adjusted(edit, edited_sheet, &own));
        if edited {
            // Ranges brought together by a deletion become one.
            let mut joined = rule.ranges.clone();
            coalesce(&mut joined);
            rule.set_ranges(joined);
        }
    }
    sheet.validations.retain(|rule| !rule.ranges.is_empty());
}

/// See `features::on_sheet_renamed`.
pub(crate) fn on_sheet_renamed(sheet: &mut Sheet, old: &str, new: &str) {
    for rule in &mut sheet.validations {
        rule.map_formulas(|e| e.with_sheet_renamed(old, new));
    }
}

/// See `features::on_sheet_duplicated`: rules of the copy that named the
/// original sheet look at the copy.
pub(crate) fn on_sheet_duplicated(wb: &mut Workbook, copy: usize, original: &str) {
    let name = wb.sheets[copy].name.clone();
    for rule in &mut wb.sheets[copy].validations {
        rule.map_formulas(|e| e.with_sheet_renamed(original, &name));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::datetime::date_to_serial;
    use crate::tests::wb;

    fn rect(name: &str) -> Rect {
        Rect::parse(name).unwrap()
    }

    fn cell(name: &str) -> (u32, u32) {
        crate::refs::parse_cell_name(name).unwrap()
    }

    /// A rule built as the dialog builds it, in English.
    fn rule(kind: ValidationKind, operator: Operator, first: &str, second: &str) -> Validation {
        let mut rule = Validation {
            kind,
            operator,
            ..Default::default()
        };
        rule.set_operands(first, second, Locale::en()).unwrap();
        rule
    }

    fn apply(w: &mut Workbook, range: &str, rule: Validation) {
        let range = rect(range);
        assert!(w.set_validation(0, &[range], (range.r0, range.c0), rule));
        w.commit("validation");
    }

    fn accepts(w: &mut Workbook, at: &str, text: &str) -> bool {
        let (r, c) = cell(at);
        w.check_entry(0, r, c, text) == EntryCheck::Accepted
    }

    fn ranges(w: &Workbook) -> Vec<Vec<String>> {
        w.sheets[0]
            .validations
            .iter()
            .map(|v| v.ranges.iter().map(Rect::name).collect())
            .collect()
    }

    #[test]
    fn whole_numbers_and_decimals() {
        let mut w = wb(&[]);
        apply(
            &mut w,
            "A1:A5",
            rule(ValidationKind::Whole, Operator::Between, "1", "10"),
        );
        apply(
            &mut w,
            "B1",
            rule(ValidationKind::Decimal, Operator::GreaterThan, "0.5", ""),
        );
        apply(
            &mut w,
            "C1",
            rule(ValidationKind::Whole, Operator::NotBetween, "-5", "5"),
        );
        assert!(accepts(&mut w, "A1", "1"));
        assert!(accepts(&mut w, "A3", "10"));
        assert!(accepts(&mut w, "A3", "5.0"));
        assert!(!accepts(&mut w, "A1", "0"));
        assert!(!accepts(&mut w, "A1", "11"));
        // 2.5 is not a whole number, text and booleans are not numbers.
        assert!(!accepts(&mut w, "A1", "2.5"));
        assert!(!accepts(&mut w, "A1", "five"));
        assert!(!accepts(&mut w, "A1", "TRUE"));
        assert!(!accepts(&mut w, "A1", "'5"));
        // A date is its serial number, a percentage a fraction.
        assert!(!accepts(&mut w, "A1", "1/2/2024"));
        assert!(!accepts(&mut w, "A1", "50%"));
        // Formulas are judged on their result.
        assert!(accepts(&mut w, "A2", "=2+3"));
        assert!(!accepts(&mut w, "A2", "=2*30"));
        assert!(!accepts(&mut w, "A2", "=1/0"));
        // An empty entry is ignored by default; cells without a rule take anything.
        assert!(accepts(&mut w, "A1", ""));
        assert!(accepts(&mut w, "A6", "999"));
        assert!(accepts(&mut w, "B1", "0.75"));
        assert!(accepts(&mut w, "B1", "60%"));
        assert!(!accepts(&mut w, "B1", "0.5"));
        assert!(accepts(&mut w, "C1", "-6"));
        assert!(!accepts(&mut w, "C1", "5"));
        // Nothing was stored by the checks.
        assert!(w.sheets[0].cells.is_empty());
        assert!(!w.commit("nothing"));
    }

    #[test]
    fn dates_times_and_text_length() {
        let mut w = wb(&[]);
        apply(
            &mut w,
            "A1",
            rule(ValidationKind::Date, Operator::Between, "1/1/2024", "12/31/2024"),
        );
        apply(
            &mut w,
            "B1",
            rule(ValidationKind::Time, Operator::Between, "9:00", "17:00"),
        );
        apply(
            &mut w,
            "C1",
            rule(ValidationKind::TextLength, Operator::LessOrEqual, "5", ""),
        );
        apply(
            &mut w,
            "D1",
            rule(ValidationKind::Date, Operator::GreaterOrEqual, "=TODAY()", ""),
        );
        let rule = w.validation_at(0, 0, 0).unwrap();
        assert_eq!(
            rule.formula1.as_deref(),
            Some(&Expr::Number(date_to_serial(2024, 1, 1).unwrap()))
        );
        assert_eq!(rule.operand1_text(0, 0, Locale::en()), "1/1/2024");
        assert_eq!(
            w.validation_at(0, 0, 1).unwrap().operand2_text(0, 1, Locale::en()),
            "17:00:00"
        );
        assert!(accepts(&mut w, "A1", "6/15/2024"));
        assert!(accepts(&mut w, "A1", "6/15/2024 12:00"));
        assert!(accepts(&mut w, "A1", "45300"));
        assert!(!accepts(&mut w, "A1", "1/1/2025"));
        assert!(!accepts(&mut w, "A1", "soon"));
        assert!(accepts(&mut w, "B1", "9:00"));
        assert!(accepts(&mut w, "B1", "0.5"));
        assert!(!accepts(&mut w, "B1", "8:59"));
        assert!(!accepts(&mut w, "B1", "5:01 PM"));
        assert!(accepts(&mut w, "C1", "hello"));
        assert!(!accepts(&mut w, "C1", "hello!"));
        // The length of a number is that of its text.
        assert!(accepts(&mut w, "C1", "12345"));
        assert!(!accepts(&mut w, "C1", "123456"));
        assert!(!accepts(&mut w, "C1", "=1/0"));
        assert!(accepts(&mut w, "D1", "1/1/2999"));
        assert!(!accepts(&mut w, "D1", "1/1/2000"));
    }

    #[test]
    fn blanks() {
        let mut w = wb(&[("E1", "3")]);
        let strict = |mut rule: Validation| {
            rule.allow_blank = false;
            rule
        };
        apply(
            &mut w,
            "A1",
            strict(rule(ValidationKind::Whole, Operator::Between, "1", "10")),
        );
        apply(
            &mut w,
            "B1",
            strict(rule(ValidationKind::TextLength, Operator::LessThan, "3", "")),
        );
        apply(
            &mut w,
            "C1",
            rule(ValidationKind::Whole, Operator::Between, "=E1", "=E2"),
        );
        apply(
            &mut w,
            "D1",
            strict(rule(ValidationKind::Whole, Operator::Between, "=E2", "=E1")),
        );
        // An empty entry is checked like any other when blanks are not ignored.
        assert!(!accepts(&mut w, "A1", ""));
        assert!(accepts(&mut w, "B1", ""));
        // While a bound is an empty cell, ignoring blanks lets anything in…
        assert!(accepts(&mut w, "C1", "5000"));
        assert!(accepts(&mut w, "C1", "text"));
        // …otherwise the empty cell counts as zero.
        assert!(accepts(&mut w, "D1", "2"));
        assert!(!accepts(&mut w, "D1", "4"));
        w.set_cell_text(0, 1, 4, "8").unwrap();
        w.commit("bound");
        assert!(accepts(&mut w, "C1", "8"));
        assert!(!accepts(&mut w, "C1", "9"));
        assert!(!accepts(&mut w, "C1", "text"));
    }

    #[test]
    fn typed_lists() {
        let mut w = wb(&[]);
        apply(
            &mut w,
            "A1",
            rule(ValidationKind::List, Operator::Between, "Yes, No ,Maybe", ""),
        );
        apply(&mut w, "B1", rule(ValidationKind::List, Operator::Between, "1,2,3", ""));
        let rule = w.validation_at(0, 0, 0).unwrap();
        assert_eq!(rule.formula1.as_deref(), Some(&Expr::Text(Arc::from("Yes,No,Maybe"))));
        assert_eq!(rule.operand1_text(0, 0, Locale::en()), "Yes,No,Maybe");
        assert_eq!(rule.operand1_text(0, 0, Locale::fr()), "Yes;No;Maybe");
        let items = w.validation_list(0, 0, 0).unwrap();
        assert_eq!(
            items.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(),
            ["Yes", "No", "Maybe"]
        );
        assert_eq!(items[0].value, Value::text("Yes"));
        assert!(w.has_validation_dropdown(0, 0, 0));
        assert!(w.validation_list(0, 5, 5).is_none());
        // Typed lists are matched to the letter.
        assert!(accepts(&mut w, "A1", "Yes"));
        assert!(!accepts(&mut w, "A1", "yes"));
        assert!(!accepts(&mut w, "A1", "Perhaps"));
        assert!(accepts(&mut w, "A1", "=\"May\"&\"be\""));
        // Numbers of a typed list are numbers.
        assert_eq!(w.validation_list(0, 0, 1).unwrap()[1].value, Value::Number(2.0));
        assert!(accepts(&mut w, "B1", "2"));
        assert!(accepts(&mut w, "B1", "2.0"));
        assert!(accepts(&mut w, "B1", "'2"));
        assert!(!accepts(&mut w, "B1", "4"));

        // In French the list is typed with semicolons and stored with commas.
        let mut fr = Validation {
            kind: ValidationKind::List,
            ..Default::default()
        };
        fr.set_operands("Oui;Non", "", Locale::fr()).unwrap();
        assert_eq!(fr.formula1.as_deref(), Some(&Expr::Text(Arc::from("Oui,Non"))));
    }

    #[test]
    fn lists_from_cells() {
        let mut w = wb(&[
            ("D1", "Apple"),
            ("D2", "0.5"),
            ("D4", "3/1/2024"),
            ("D5", "=1/0"),
            ("F1", "x"),
            ("F2", "y"),
            ("G1", "=\"D1:D2\""),
        ]);
        w.update_style(0, rect("D2"), &|s| s.num_fmt = Arc::from("0%"));
        w.define_name("Fruit", parse_formula("Sheet1!$D$1:$D$5", Locale::en()).unwrap());
        w.define_name("Letters", parse_formula("Sheet1!$F$1:$F$2", Locale::en()).unwrap());
        w.commit("setup");
        apply(
            &mut w,
            "A1",
            rule(ValidationKind::List, Operator::Between, "=$D$1:$D$5", ""),
        );
        apply(
            &mut w,
            "A2",
            rule(ValidationKind::List, Operator::Between, "=Fruit", ""),
        );
        apply(
            &mut w,
            "A3",
            rule(ValidationKind::List, Operator::Between, "=Letters", ""),
        );
        apply(
            &mut w,
            "A4",
            rule(ValidationKind::List, Operator::Between, "=INDIRECT(G1)", ""),
        );
        apply(
            &mut w,
            "A5",
            rule(ValidationKind::List, Operator::Between, "=OFFSET($F$1,0,0,2,1)", ""),
        );

        // The lines are the cells as displayed, blank ones included; choosing one stores the value.
        let items = w.validation_list(0, 0, 0).unwrap();
        let texts: Vec<&str> = items.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(texts, ["Apple", "50%", "", "3/1/2024", "#DIV/0!"]);
        assert_eq!(items[1].value, Value::Number(0.5));
        assert_eq!(items[2].value, Value::Empty);
        assert_eq!(items[3].value, Value::Number(date_to_serial(2024, 3, 1).unwrap()));
        assert_eq!(w.validation_list(0, 1, 0).unwrap(), items);
        let texts = |w: &Workbook, row: u32| -> Vec<String> {
            w.validation_list(0, row, 0)
                .unwrap()
                .into_iter()
                .map(|i| i.text)
                .collect()
        };
        assert_eq!(texts(&w, 2), ["x", "y"]);
        assert_eq!(texts(&w, 3), ["Apple", "50%"]);
        assert_eq!(texts(&w, 4), ["x", "y"]);

        // Cells are matched by value, text without case.
        assert!(accepts(&mut w, "A1", "apple"));
        assert!(accepts(&mut w, "A1", "50%"));
        assert!(accepts(&mut w, "A1", "0.5"));
        assert!(!accepts(&mut w, "A1", "Pear"));
        assert!(!accepts(&mut w, "A1", "0"));
        // A blank cell in a named list lets anything in while blanks are ignored.
        assert!(accepts(&mut w, "A2", "Pear"));
        assert!(!accepts(&mut w, "A3", "z"));
        assert!(accepts(&mut w, "A4", "APPLE"));
        assert!(!accepts(&mut w, "A4", "3/1/2024"));
        assert!(accepts(&mut w, "A5", "Y"));
        let (r, c) = cell("A2");
        w.sheets[0]
            .validations
            .iter_mut()
            .find(|v| v.contains(r, c))
            .unwrap()
            .allow_blank = false;
        assert!(!accepts(&mut w, "A2", "Pear"));

        // A whole column lists the used part of it.
        apply(
            &mut w,
            "B1",
            rule(ValidationKind::List, Operator::Between, "=$F:$F", ""),
        );
        assert_eq!(w.validation_list(0, 0, 1).unwrap().len(), 5);
        // Lists on another sheet.
        let other = w.add_sheet(1);
        w.set_cell_text(other, 0, 0, "far").unwrap();
        w.commit("sheet");
        apply(
            &mut w,
            "B2",
            rule(ValidationKind::List, Operator::Between, "=Sheet2!$A$1:$A$2", ""),
        );
        assert!(accepts(&mut w, "B2", "Far"));
        assert!(!accepts(&mut w, "B2", "near"));
    }

    #[test]
    fn custom_formulas_see_the_entry() {
        let mut w = wb(&[("A1", "a"), ("A2", "b"), ("C1", "=SUM(B1:B3)"), ("B1", "40")]);
        // No duplicates in A1:A5: the formula is relative to each cell.
        apply(
            &mut w,
            "A1:A5",
            rule(
                ValidationKind::Custom,
                Operator::Between,
                "=COUNTIF($A$1:$A$5,A1)=1",
                "",
            ),
        );
        assert_eq!(
            w.validation_at(0, 2, 0).unwrap().operand1_text(2, 0, Locale::en()),
            "=COUNTIF($A$1:$A$5,A3)=1"
        );
        assert!(accepts(&mut w, "A3", "c"));
        assert!(!accepts(&mut w, "A3", "B"));
        assert!(accepts(&mut w, "A2", "b"));
        assert!(!accepts(&mut w, "A2", "a"));
        // The rule reads a formula that depends on the entry: the total may not pass 100.
        apply(
            &mut w,
            "B1:B3",
            rule(ValidationKind::Custom, Operator::Between, "=$C$1<=100", ""),
        );
        assert!(accepts(&mut w, "B2", "60"));
        assert!(!accepts(&mut w, "B2", "61"));
        assert!(accepts(&mut w, "B3", "=B1+20"));
        assert!(!accepts(&mut w, "B3", "=B1+21"));
        // The trial left no trace.
        assert_eq!(w.value_at(0, 0, 2), Value::Number(40.0));
        assert!(w.cell(0, 1, 1).is_none());
        assert_eq!(w.value_at(0, 1, 0), Value::text("b"));
        // Text and errors are not true; a blank result follows "Ignore blank".
        apply(
            &mut w,
            "E1",
            rule(ValidationKind::Custom, Operator::Between, "=\"yes\"", ""),
        );
        apply(&mut w, "E2", rule(ValidationKind::Custom, Operator::Between, "=F2", ""));
        assert!(!accepts(&mut w, "E1", "1"));
        assert!(accepts(&mut w, "E2", "1"));
        w.sheets[0].validations.last_mut().unwrap().allow_blank = false;
        assert!(!accepts(&mut w, "E2", "1"));
        // Cells formatted as text keep digits as text.
        apply(
            &mut w,
            "G1",
            rule(ValidationKind::Custom, Operator::Between, "=ISNUMBER(G1)", ""),
        );
        assert!(accepts(&mut w, "G1", "12"));
        w.update_style(0, rect("G1"), &|s| s.num_fmt = Arc::from("@"));
        w.commit("text");
        assert!(!accepts(&mut w, "G1", "12"));
    }

    #[test]
    fn alerts() {
        let mut w = wb(&[]);
        apply(&mut w, "A1", rule(ValidationKind::Whole, Operator::Equal, "1", ""));
        let alert = |w: &mut Workbook, text: &str| match w.check_entry(0, 0, 0, text) {
            EntryCheck::Invalid(alert) => alert,
            EntryCheck::Accepted => panic!("{text} was accepted"),
        };
        let stop = alert(&mut w, "2");
        assert_eq!(stop.style, AlertStyle::Stop);
        assert_eq!(stop.title, "");
        assert_eq!(
            stop.message,
            "This value doesn't match the data validation restrictions defined for this cell."
        );
        for (locale, start) in [
            (Locale::fr(), "Cette valeur"),
            (Locale::es(), "Este valor"),
            (Locale::de(), "Dieser Wert"),
            (Locale::it(), "Questo valore"),
        ] {
            w.locale = locale;
            assert!(alert(&mut w, "2").message.starts_with(start));
        }
        let rule = &mut w.sheets[0].validations[0];
        rule.error_style = AlertStyle::Warning;
        assert!(alert(&mut w, "2").message.ends_with("cella.\nContinuare?"));
        let rule = &mut w.sheets[0].validations[0];
        rule.error_title = "Quantity".into();
        rule.error_message = "One only.".into();
        let warning = alert(&mut w, "2");
        assert_eq!(
            (warning.title.as_str(), warning.message.as_str()),
            ("Quantity", "One only.\n\nContinuare?")
        );
        w.sheets[0].validations[0].error_style = AlertStyle::Information;
        assert_eq!(alert(&mut w, "2").message, "One only.");
        // Without an error alert nothing is refused, but the cell is still wrong.
        w.sheets[0].validations[0].show_error = false;
        assert_eq!(w.check_entry(0, 0, 0, "2"), EntryCheck::Accepted);
        w.set_cell_text(0, 0, 0, "2").unwrap();
        w.commit("entry");
        assert!(!w.cell_is_valid(0, 0, 0));
        // An entry that does not parse is left to `set_cell_text`.
        w.sheets[0].validations[0].show_error = true;
        assert_eq!(w.check_entry(0, 0, 0, "=1+"), EntryCheck::Accepted);
    }

    #[test]
    fn input_messages() {
        let mut w = wb(&[]);
        let note = Validation {
            input_title: "Tip".into(),
            input_message: "Any value will do.".into(),
            ..Default::default()
        };
        apply(&mut w, "A1:B2", note);
        assert_eq!(w.input_message(0, 1, 1), Some(("Tip", "Any value will do.")));
        assert_eq!(w.input_message(0, 2, 2), None);
        assert_eq!(w.check_entry(0, 0, 0, "whatever"), EntryCheck::Accepted);
        w.sheets[0].validations[0].show_input = false;
        assert_eq!(w.input_message(0, 1, 1), None);
        // "Any value" without a message is no rule at all.
        apply(&mut w, "A1:B2", Validation::default());
        assert!(w.sheets[0].validations.is_empty());
    }

    #[test]
    fn rules_split_and_join_ranges() {
        let mut w = wb(&[]);
        let whole = || rule(ValidationKind::Whole, Operator::Between, "1", "10");
        apply(&mut w, "B2:E6", whole());
        // A different rule in the middle cuts the first one into four.
        apply(
            &mut w,
            "C3:D4",
            rule(ValidationKind::List, Operator::Between, "a,b", ""),
        );
        assert_eq!(ranges(&w), [vec!["B2:E2", "B5:E6", "B3:B4", "E3:E4"], vec!["C3:D4"]]);
        assert_eq!(w.validation_at(0, 2, 2).unwrap().kind, ValidationKind::List);
        assert_eq!(w.validation_at(0, 2, 1).unwrap().kind, ValidationKind::Whole);
        assert!(w.validation_at(0, 0, 0).is_none());
        // The same rule again fills the hole: one range, one rule.
        apply(&mut w, "C3:D4", whole());
        assert_eq!(ranges(&w), [vec!["B2:E6"]]);
        // Applied elsewhere it joins the rule it equals.
        apply(&mut w, "G1", whole());
        apply(&mut w, "B7:E7", whole());
        assert_eq!(ranges(&w), [vec!["B2:E7", "G1"]]);
        assert_eq!(w.validation_ranges_like(0, 0, 6), [rect("B2:E7"), rect("G1")]);
        // A selection that overlaps itself gives each cell once.
        assert!(w.set_validation(0, &[rect("J1:K2"), rect("K2:L3"), rect("J1")], (0, 9), whole()));
        assert_eq!(ranges(&w), [vec!["B2:E7", "G1", "J1:K2", "K3:L3", "L2"]]);
        assert!(w.clear_validation(0, &[rect("J:L")]));
        w.rollback();
        assert!(w.clear_validation(0, &[rect("A1:C100")]));
        w.commit("clear");
        assert_eq!(ranges(&w), [vec!["D2:E7", "G1"]]);
        assert!(!w.clear_validation(0, &[rect("A1:C100")]));
        w.undo();
        assert_eq!(ranges(&w), [vec!["B2:E7", "G1"]]);
        w.undo();
        w.undo();
        w.undo();
        assert_eq!(ranges(&w).len(), 2);
    }

    #[test]
    fn formulas_follow_the_anchor() {
        let mut w = wb(&[]);
        // Typed with B5 active in the selection B2:B5.
        let greater = rule(ValidationKind::Decimal, Operator::GreaterThan, "=A5", "");
        assert!(w.set_validation(0, &[rect("B2:B5")], cell("B5"), greater.clone()));
        w.commit("rule");
        let stored = &w.sheets[0].validations[0];
        assert_eq!(stored.formula1.as_ref().unwrap().to_text(Locale::en()), "A2");
        assert_eq!(stored.operand1_text(3, 1, Locale::en()), "=A4");
        // The same relative rule elsewhere is the same rule.
        assert!(w.set_validation(
            0,
            &[rect("E9")],
            cell("E9"),
            rule(ValidationKind::Decimal, Operator::GreaterThan, "=D9", "")
        ));
        w.commit("rule");
        assert_eq!(ranges(&w), [vec!["B2:B5", "E9"]]);
        // Cutting the anchor away rewrites the formula for the new one.
        assert!(w.clear_validation(0, &[rect("B2:B3")]));
        w.commit("clear");
        let stored = &w.sheets[0].validations[0];
        assert_eq!(stored.anchor(), Some(cell("B4")));
        assert_eq!(stored.formula1.as_ref().unwrap().to_text(Locale::en()), "A4");
        w.set_cell_text(0, 3, 0, "10").unwrap();
        w.commit("bound");
        assert!(accepts(&mut w, "B4", "11"));
        assert!(!accepts(&mut w, "B4", "10"));
        // The bound of B5 is the empty A5: with blanks ignored anything goes.
        assert!(accepts(&mut w, "B5", "-3"));
    }

    #[test]
    fn selections() {
        let mut w = wb(&[]);
        apply(&mut w, "A1:A3", rule(ValidationKind::Whole, Operator::Equal, "1", ""));
        apply(&mut w, "C1", rule(ValidationKind::List, Operator::Between, "a,b", ""));
        assert_eq!(w.selection_validation(0, &[rect("E1:F9")]), SelectionValidation::None);
        assert!(
            matches!(w.selection_validation(0, &[rect("A1:A2")]), SelectionValidation::Uniform(v) if v.kind == ValidationKind::Whole)
        );
        assert!(matches!(
            w.selection_validation(0, &[rect("A2"), rect("A3")]),
            SelectionValidation::Uniform(_)
        ));
        let partial = w.selection_validation(0, &[rect("A1:B3")]);
        assert!(matches!(partial, SelectionValidation::Partial(_)));
        assert!(
            partial
                .question(Locale::en())
                .unwrap()
                .starts_with("The selection contains some cells without")
        );
        let mixed = w.selection_validation(0, &[rect("A:C")]);
        assert_eq!(mixed, SelectionValidation::Mixed);
        assert!(
            mixed
                .question(Locale::fr())
                .unwrap()
                .starts_with("La sélection contient plus d'un type")
        );
        assert!(SelectionValidation::None.question(Locale::en()).is_none());
    }

    #[test]
    fn structural_edits() {
        let mut w = wb(&[("A1", "5")]);
        apply(
            &mut w,
            "B2:B5",
            rule(ValidationKind::Decimal, Operator::Between, "=$A$1", "=A2+100"),
        );
        let texts = |w: &Workbook| {
            let rule = &w.sheets[0].validations[0];
            let text = |e: &Option<Arc<Expr>>| e.as_ref().unwrap().to_text(Locale::en());
            (
                rule.ranges.iter().map(Rect::name).collect::<Vec<_>>().join(" "),
                text(&rule.formula1),
                text(&rule.formula2),
            )
        };
        // Rows inserted above move the range, inside they widen it.
        w.insert_rows(0, 0, 1);
        w.insert_rows(0, 3, 2);
        w.commit("insert");
        assert_eq!(texts(&w), ("B3:B8".into(), "$A$2".into(), "A3+100".into()));
        // A row inserted right below takes the rule of the row above it; one further down does not.
        w.insert_rows(0, 8, 1);
        w.insert_rows(0, 10, 1);
        w.commit("insert");
        assert_eq!(texts(&w).0, "B3:B9");
        // Deleting the anchor row leaves the rule reading the same in the cells that stay.
        w.delete_rows(0, 2, 2);
        w.commit("delete");
        assert_eq!(texts(&w), ("B3:B7".into(), "$A$2".into(), "A3+100".into()));
        w.insert_cols(0, 0, 2);
        w.commit("columns");
        assert_eq!(texts(&w), ("D3:D7".into(), "$C$2".into(), "C3+100".into()));
        w.delete_cols(0, 3, 1);
        w.commit("columns");
        assert!(w.sheets[0].validations.is_empty());
        w.undo();
        assert_eq!(texts(&w).0, "D3:D7");
        // Edits of another sheet only touch the formulas that point there.
        let other = w.add_sheet(1);
        w.commit("sheet");
        assert!(w.set_validation(
            other,
            &[rect("A1")],
            (0, 0),
            rule(ValidationKind::List, Operator::Between, "=Sheet1!$C$2:$C$4", "")
        ));
        w.commit("rule");
        w.insert_rows(0, 0, 3);
        w.commit("insert");
        let source = |w: &Workbook| {
            w.sheets[1].validations[0]
                .formula1
                .as_ref()
                .unwrap()
                .to_text(Locale::en())
        };
        assert_eq!(source(&w), "Sheet1!$C$5:$C$7");
        assert_eq!(w.sheets[1].validations[0].ranges, [rect("A1")]);
        assert!(w.rename_sheet(0, "Data"));
        w.commit("rename");
        assert_eq!(source(&w), "Data!$C$5:$C$7");
    }

    #[test]
    fn duplicated_sheets_validate_their_own_cells() {
        let mut w = wb(&[("D1", "a"), ("D2", "b")]);
        apply(
            &mut w,
            "A1",
            rule(ValidationKind::List, Operator::Between, "=Sheet1!$D$1:$D$2", ""),
        );
        apply(
            &mut w,
            "A2",
            rule(ValidationKind::List, Operator::Between, "=$D$1:$D$2", ""),
        );
        let copy = w.duplicate_sheet(0).unwrap();
        w.commit("copy");
        let source = |row: usize| {
            w.sheets[copy].validations[row]
                .formula1
                .as_ref()
                .unwrap()
                .to_text(Locale::en())
        };
        assert_eq!(source(0), "'Sheet1 (2)'!$D$1:$D$2");
        assert_eq!(source(1), "$D$1:$D$2");
        assert_eq!(
            w.sheets[0].validations[0]
                .formula1
                .as_ref()
                .unwrap()
                .to_text(Locale::en()),
            "Sheet1!$D$1:$D$2"
        );
    }

    #[test]
    fn circles() {
        let mut w = wb(&[
            ("A1", "5"),
            ("A2", "50"),
            ("A3", "x"),
            ("B1", "=A2*2"),
            ("C1", "={1;200}"),
        ]);
        apply(
            &mut w,
            "A1:C100",
            rule(ValidationKind::Whole, Operator::Between, "1", "10"),
        );
        let found = w.invalid_cells(0);
        assert_eq!(found.cells, [cell("B1"), cell("A2"), cell("C2"), cell("A3")]);
        assert!(!found.truncated && found.warning(Locale::en()).is_none());
        assert!(w.cell_is_valid(0, 0, 0) && !w.cell_is_valid(0, 1, 0) && w.cell_is_valid(0, 20, 20));
        // Blank cells are wrong where blanks are not ignored, and Excel stops at 255 circles.
        w.sheets[0].validations[0].allow_blank = false;
        let found = w.invalid_cells(0);
        assert_eq!(found.cells.len(), MAX_CIRCLES);
        assert!(found.truncated);
        assert!(found.warning(Locale::de()).unwrap().contains("255"));
        assert_eq!(found.cells[0], cell("B1"));
    }

    #[test]
    fn pasting_copies_rules() {
        let mut w = wb(&[]);
        apply(
            &mut w,
            "A1:A2",
            rule(ValidationKind::Decimal, Operator::GreaterThan, "=B1", ""),
        );
        apply(&mut w, "D4", rule(ValidationKind::Whole, Operator::Equal, "1", ""));
        // A1:A3 onto D3:D5: two cells with the rule, one without.
        w.copy_validation(0, rect("A1:A3"), 0, 2, 3);
        w.commit("paste");
        assert_eq!(ranges(&w), [vec!["A1:A2", "D3:D4"]]);
        assert_eq!(
            w.validation_at(0, 3, 3).unwrap().operand1_text(3, 3, Locale::en()),
            "=E4"
        );
        assert!(w.validation_at(0, 4, 3).is_none());
        w.undo();
        assert_eq!(ranges(&w), [vec!["A1:A2"], vec!["D4"]]);
    }

    #[test]
    fn filling_extends_rules() {
        let operand = |w: &Workbook, at: &str| {
            let (r, c) = cell(at);
            w.validation_at(0, r, c).map(|v| v.operand1_text(r, c, Locale::en()))
        };
        // One cell filled down: the rule grows, its formula read from each cell.
        let mut w = wb(&[]);
        apply(
            &mut w,
            "B2",
            rule(ValidationKind::Decimal, Operator::GreaterThan, "=A2", ""),
        );
        apply(&mut w, "B5", rule(ValidationKind::Whole, Operator::Equal, "1", ""));
        w.tile_validation(0, rect("B2"), 0, rect("B2:B6"), (1, 1));
        w.commit("fill");
        assert_eq!(ranges(&w), [vec!["B2:B6"]]);
        assert_eq!(operand(&w, "B6").as_deref(), Some("=A6"));
        w.undo();
        assert_eq!(ranges(&w), [vec!["B2"], vec!["B5"]]);

        // Part of a rule filled beyond it: one rule still.
        let mut w = wb(&[]);
        apply(&mut w, "A1:A4", rule(ValidationKind::List, Operator::Between, "a,b", ""));
        w.tile_validation(0, rect("A4"), 0, rect("A4:A9"), (3, 0));
        w.commit("fill");
        assert_eq!(ranges(&w), [vec!["A1:A9"]]);

        // A pattern of two rows, one with a rule, filled down and up:
        // every other row, cut where the range ends.
        let mut w = wb(&[]);
        apply(
            &mut w,
            "C5",
            rule(ValidationKind::Whole, Operator::LessThan, "=$A$1+D5", ""),
        );
        apply(&mut w, "C8", rule(ValidationKind::TextLength, Operator::Equal, "3", ""));
        w.tile_validation(0, rect("C5:C6"), 0, rect("C2:C9"), (4, 2));
        w.commit("fill");
        assert_eq!(ranges(&w), [vec!["C3", "C5", "C7", "C9"]]);
        assert_eq!(operand(&w, "C3").as_deref(), Some("=$A$1+D3"));
        assert_eq!(operand(&w, "C9").as_deref(), Some("=$A$1+D9"));
        assert_eq!(operand(&w, "C8"), None);

        // Across: a row of two cells, the second without a rule.
        let mut w = wb(&[]);
        apply(&mut w, "A1", rule(ValidationKind::Whole, Operator::Equal, "1", ""));
        apply(&mut w, "B1:E1", rule(ValidationKind::Whole, Operator::Equal, "2", ""));
        w.tile_validation(0, rect("A1:B1"), 0, rect("A1:E1"), (0, 0));
        w.commit("fill");
        assert_eq!(ranges(&w), [vec!["A1", "C1", "E1"], vec!["B1", "D1"]]);
    }

    #[test]
    fn pasting_repeats_and_cutting_moves() {
        let mut w = wb(&[]);
        apply(
            &mut w,
            "A1",
            rule(ValidationKind::Decimal, Operator::GreaterThan, "=B1", ""),
        );
        // A1:A2 pasted over D1:E4: the block repeats four times.
        w.tile_validation(0, rect("A1:A2"), 0, rect("D1:E4"), (0, 3));
        w.commit("paste");
        assert_eq!(ranges(&w), [vec!["A1", "D1:E1", "D3:E3"]]);
        assert_eq!(
            w.validation_at(0, 2, 4).unwrap().operand1_text(2, 4, Locale::en()),
            "=F3"
        );
        // Onto another sheet.
        let other = w.add_sheet(1);
        w.tile_validation(0, rect("A1"), other, rect("C3"), (2, 2));
        w.commit("paste");
        let pasted = w.validation_at(other, 2, 2).unwrap();
        assert_eq!(pasted.ranges, [rect("C3")]);
        assert_eq!(pasted.operand1_text(2, 2, Locale::en()), "=D3");

        // Cutting A1:A2 to A2:A3: the rule goes with its cell.
        let mut w = wb(&[]);
        apply(&mut w, "A1", rule(ValidationKind::Whole, Operator::Equal, "1", ""));
        apply(&mut w, "A3", rule(ValidationKind::Whole, Operator::Equal, "3", ""));
        w.move_validation(0, rect("A1:A2"), 0, 1, 0);
        w.commit("cut");
        assert_eq!(ranges(&w), [vec!["A2"]]);
        assert_eq!(w.validation_at(0, 1, 0).unwrap().operand1_text(1, 0, Locale::en()), "1");
        w.undo();
        assert_eq!(ranges(&w), [vec!["A1"], vec!["A3"]]);
        // To another sheet: nothing stays behind.
        let other = w.add_sheet(1);
        w.move_validation(0, rect("A1:A3"), other, 0, 0);
        w.commit("cut");
        assert!(w.sheets[0].validations.is_empty());
        assert_eq!(w.sheets[other].validations.len(), 2);
    }

    #[test]
    fn formulas_that_fail_are_asked_about() {
        let mut w = wb(&[("A1", "0"), ("C1", "x"), ("C2", "=1/0")]);
        w.define_name("Fruits", crate::parser::parse_formula("Sheet1!$C$1:$C$2", Locale::en()).unwrap());
        w.commit("names");
        let warning = |w: &Workbook, kind, first: &str| {
            w.rule_formula_warning(0, (4, 4), &rule(kind, Operator::GreaterThan, first, ""))
        };
        assert_eq!(warning(&w, ValidationKind::Whole, "5"), None);
        assert_eq!(warning(&w, ValidationKind::Whole, "=A1+1"), None);
        assert_eq!(
            warning(&w, ValidationKind::Whole, "=1/A1"),
            Some(FormulaWarning::Error(Field::Minimum))
        );
        assert_eq!(
            warning(&w, ValidationKind::Custom, "=E5/A1>0"),
            Some(FormulaWarning::Error(Field::Formula))
        );
        // A list of cells may hold errors; a name must exist.
        assert_eq!(warning(&w, ValidationKind::List, "=$C$1:$C$2"), None);
        assert_eq!(warning(&w, ValidationKind::List, "=Fruits"), None);
        assert_eq!(warning(&w, ValidationKind::List, "a,b"), None);
        assert_eq!(
            warning(&w, ValidationKind::List, "=Nowhere"),
            Some(FormulaWarning::UnknownName)
        );
        let second = w.rule_formula_warning(
            0,
            (0, 1),
            &rule(ValidationKind::Decimal, Operator::Between, "1", "=1/A1"),
        );
        assert_eq!(second, Some(FormulaWarning::Error(Field::Maximum)));
        assert_eq!(
            FormulaWarning::Error(Field::Source).message(Locale::en()),
            "The Source currently evaluates to an error. Do you want to continue?"
        );
        assert_eq!(
            FormulaWarning::Error(Field::Minimum).message(Locale::fr()),
            "Le minimum est reconnu comme erroné. Voulez-vous continuer ?"
        );
    }

    #[test]
    fn dialog_boxes() {
        let en = Locale::en();
        let fr = Locale::fr();
        assert_eq!(
            operand_fields(ValidationKind::Date, Operator::Between),
            (Some(Field::StartDate), Some(Field::EndDate))
        );
        assert_eq!(
            operand_fields(ValidationKind::TextLength, Operator::Equal),
            (Some(Field::Length), None)
        );
        assert_eq!(
            operand_fields(ValidationKind::Whole, Operator::LessThan),
            (Some(Field::Maximum), None)
        );
        assert_eq!(operand_fields(ValidationKind::Any, Operator::Between), (None, None));
        let build = |kind, operator, first: &str, second: &str, locale: &Locale| {
            let mut rule = Validation {
                kind,
                operator,
                ..Default::default()
            };
            rule.set_operands(first, second, locale)
                .map(|_| rule)
                .map_err(|e| e.message(locale))
        };
        let error = |kind, operator, first: &str, second: &str| build(kind, operator, first, second, en).unwrap_err();
        use Operator::*;
        use ValidationKind::*;
        assert_eq!(
            error(Whole, Between, "", ""),
            "You must enter both a Minimum and a Maximum."
        );
        assert_eq!(error(Whole, Between, "1", ""), "You must enter a Maximum.");
        assert_eq!(error(List, Between, " ", ""), "You must enter a Source.");
        assert_eq!(error(Custom, Between, "", ""), "You must enter a Formula.");
        assert_eq!(
            error(Whole, Between, "10", "1"),
            "The Maximum must be greater than or equal to the Minimum."
        );
        assert_eq!(
            error(Whole, Equal, "1.5", ""),
            "Decimal values cannot be used for Whole number conditions."
        );
        assert_eq!(
            error(TextLength, LessThan, "-1", ""),
            "Negative values cannot be used for Text length conditions."
        );
        assert!(error(Decimal, GreaterThan, "abc", "").starts_with("The Minimum must be a numeric value"));
        assert_eq!(
            error(Date, Between, "1/1/2024", "someday"),
            "The date you entered for the End Date is invalid."
        );
        assert_eq!(
            error(Time, Equal, "noon", ""),
            "The time you entered for the Time is invalid."
        );
        assert!(error(List, Between, "=$A$1:$B$5", "").starts_with("The list source must be a delimited list"));
        assert!(matches!(
            Validation {
                kind: Custom,
                ..Default::default()
            }
            .set_operands("=1+", "", en),
            Err(RuleError::Formula(_))
        ));
        assert_eq!(
            build(Date, Between, "", "", fr).unwrap_err(),
            "Tapez une date de Début et aussi une date de Fin."
        );
        assert_eq!(
            build(Decimal, Between, "5", "1,5", fr).unwrap_err(),
            "Le maximum doit être supérieur ou égal à Le minimum."
        );

        // What is typed and what is shown again.
        let decimal = build(Decimal, Between, "-1,5", "=SOMME(A1:A3)", fr).unwrap();
        assert_eq!(decimal.formula1.as_deref(), Some(&Expr::Number(-1.5)));
        assert_eq!(decimal.operand1_text(0, 0, fr), "-1,5");
        assert_eq!(decimal.operand2_text(0, 0, fr), "=SOMME(A1:A3)");
        assert_eq!(decimal.operand2_text(0, 0, en), "=SUM(A1:A3)");
        // A reference or a function typed without its "=".
        let bare = build(Whole, GreaterOrEqual, "A1*2", "", en).unwrap();
        assert_eq!(bare.operand1_text(0, 0, en), "=A1*2");
        let date = build(Date, Equal, "12/03/2025", "", fr).unwrap();
        assert_eq!(
            date.formula1.as_deref(),
            Some(&Expr::Number(date_to_serial(2025, 3, 12).unwrap()))
        );
        assert_eq!(date.operand1_text(0, 0, fr), "12/03/2025");
        assert_eq!(date.operand1_text(0, 0, en), "3/12/2025");
        // A list source keeps single rows and columns, however they are written.
        assert!(build(List, Between, "=$A$1:$A$9", "", en).is_ok());
        assert!(build(List, Between, "=Names", "", en).is_ok());
        // Operators that take no second value drop the one left in the box.
        assert!(build(Whole, Equal, "3", "99", en).unwrap().formula2.is_none());
        assert_eq!(build(Any, Between, "1", "2", en).unwrap().formula1, None);
    }

    #[test]
    fn file_names() {
        for kind in ValidationKind::ALL {
            assert_eq!(ValidationKind::from_ooxml_name(kind.ooxml_name()), Some(kind));
            assert!(!kind.label(Locale::it()).is_empty());
        }
        for operator in Operator::ALL {
            assert_eq!(Operator::from_ooxml_name(operator.ooxml_name()), Some(operator));
        }
        for style in AlertStyle::ALL {
            assert_eq!(AlertStyle::from_ooxml_name(style.ooxml_name()), Some(style));
        }
        for mode in ImeMode::ALL {
            assert_eq!(ImeMode::from_ooxml_name(mode.ooxml_name()), Some(mode));
        }
        assert_eq!(
            ValidationKind::from_ooxml_name("textLength"),
            Some(ValidationKind::TextLength)
        );
        assert_eq!(
            Operator::from_ooxml_name("greaterThanOrEqual"),
            Some(Operator::GreaterOrEqual)
        );
        assert_eq!(Operator::LessThan.label(Locale::fr()), "inférieure à");
        assert_eq!(AlertStyle::Warning.label(Locale::de()), "Warnung");
        assert_eq!(ValidationKind::from_ooxml_name("bogus"), None);
        let (new, read) = (Validation::default(), Validation::file_default());
        assert!(new.allow_blank && new.show_input && new.show_error && new.in_cell_dropdown);
        assert!(!read.allow_blank && !read.show_input && !read.show_error && read.in_cell_dropdown);
    }
}
