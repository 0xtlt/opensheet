//! The words Excel writes into a PivotTable ("Row Labels", "Grand Total",
//! "Sum of Sales", "Qtr1", ...), in the five languages. The wording is the
//! one of Excel for Microsoft 365 in each language.

use super::{Aggregation, GroupBy};
use crate::datetime::{serial_to_hms, serial_to_ymd};
use crate::format;
use crate::locale::Locale;

/// Index of the locale's language in the tables below (en, fr, es, de, it).
fn lang(locale: &Locale) -> usize {
    match locale.id {
        "fr" => 1,
        "es" => 2,
        "de" => 3,
        "it" => 4,
        _ => 0,
    }
}

/// A caption Excel generates in or around a PivotTable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caption {
    /// Header of the row labels in compact form.
    RowLabels,
    /// Header of the column labels in compact form.
    ColumnLabels,
    GrandTotal,
    /// The item standing for empty source cells.
    Blank,
    /// A filter field showing every item.
    All,
    /// A filter field with several items selected.
    MultipleItems,
    /// The "Σ Values" field.
    Values,
    /// The word in "East Total", and the label of the only row of a
    /// report without row fields.
    Total,
    /// Prefix of a manual group ("Group1").
    Group,
    /// Prefix of a quarter ("Qtr1").
    Quarter,
    /// Prefix of the default name of a PivotTable ("PivotTable1").
    PivotTable,
}

/// The caption in the workbook's language.
pub fn caption(c: Caption, locale: &Locale) -> &'static str {
    let t: [&str; 5] = match c {
        Caption::RowLabels => [
            "Row Labels",
            "Étiquettes de lignes",
            "Etiquetas de fila",
            "Zeilenbeschriftungen",
            "Etichette di riga",
        ],
        Caption::ColumnLabels => [
            "Column Labels",
            "Étiquettes de colonnes",
            "Etiquetas de columna",
            "Spaltenbeschriftungen",
            "Etichette di colonna",
        ],
        Caption::GrandTotal => [
            "Grand Total",
            "Total général",
            "Total general",
            "Gesamtergebnis",
            "Totale complessivo",
        ],
        Caption::Blank => ["(blank)", "(vide)", "(en blanco)", "(Leer)", "(vuoto)"],
        Caption::All => ["(All)", "(Tous)", "(Todas)", "(Alle)", "(Tutto)"],
        Caption::MultipleItems => [
            "(Multiple Items)",
            "(Plusieurs éléments)",
            "(Varios elementos)",
            "(Mehrere Elemente)",
            "(più elementi)",
        ],
        Caption::Values => ["Values", "Valeurs", "Valores", "Werte", "Valori"],
        Caption::Total => ["Total", "Total", "Total", "Ergebnis", "Totale"],
        Caption::Group => ["Group", "Groupe", "Grupo", "Gruppe", "Gruppo"],
        Caption::Quarter => ["Qtr", "Trimestre", "Trim.", "Qrtl", "Trim"],
        Caption::PivotTable => [
            "PivotTable",
            "Tableau croisé dynamique",
            "TablaDinámica",
            "PivotTable",
            "Tabella pivot",
        ],
    };
    t[lang(locale)]
}

impl Aggregation {
    /// The name of the function in the "Summarize value field by" list.
    pub fn label(self, locale: &Locale) -> &'static str {
        let t: [&str; 5] = match self {
            Aggregation::Sum => ["Sum", "Somme", "Suma", "Summe", "Somma"],
            Aggregation::Count => ["Count", "Nombre", "Recuento", "Anzahl", "Conteggio"],
            Aggregation::Average => ["Average", "Moyenne", "Promedio", "Mittelwert", "Media"],
            Aggregation::Max => ["Max", "Max.", "Máx.", "Max.", "Max"],
            Aggregation::Min => ["Min", "Min.", "Mín.", "Min.", "Min"],
            Aggregation::Product => ["Product", "Produit", "Producto", "Produkt", "Prodotto"],
            Aggregation::CountNums => [
                "Count Numbers",
                "Chiffres",
                "Contar números",
                "Anzahl Zahlen",
                "Conta numeri",
            ],
            Aggregation::StdDev => ["StdDev", "Écartype", "Desvest", "STABW", "Dev. standard"],
            Aggregation::StdDevp => [
                "StdDevp",
                "Écartypep",
                "Desvestp",
                "Standardabweichung (Grundgesamtheit)",
                "Dev. standard pop.",
            ],
            Aggregation::Var => ["Var", "Var", "Var", "Var", "Varianza"],
            Aggregation::Varp => [
                "Varp",
                "Varp",
                "Varp",
                "Varianz (Grundgesamtheit)",
                "Varianza pop.",
            ],
        };
        t[lang(locale)]
    }

    /// The word used in the name of a value field: as `label`, except that
    /// "Count Numbers" names its field "Count of ..." like "Count".
    fn word(self, locale: &Locale) -> &'static str {
        match self {
            Aggregation::CountNums => Aggregation::Count.label(locale),
            f => f.label(locale),
        }
    }
}

/// The name Excel gives a value field: "Sum of Sales", "Somme de Ventes".
pub fn data_field_name(function: Aggregation, field: &str, locale: &Locale) -> String {
    let of = [" of ", " de ", " de ", " von ", " di "][lang(locale)];
    format!("{}{of}{field}", function.word(locale))
}

/// Label of a subtotal: "East Total" ("Total East" in French and Spanish).
pub(crate) fn subtotal_label(item: &str, word: &str, locale: &Locale) -> String {
    match lang(locale) {
        1 | 2 => format!("{word} {item}"),
        _ => format!("{item} {word}"),
    }
}

/// Label of the grand total of one value field when there are several:
/// "Total Sum of Sales".
pub(crate) fn grand_total_of(data_name: &str, locale: &Locale) -> String {
    match lang(locale) {
        3 => format!("Gesamt: {data_name}"),
        4 => format!("{data_name} totale"),
        _ => format!("Total {data_name}"),
    }
}

impl GroupBy {
    /// The name of the grouping in the Grouping dialog, also used to name
    /// the fields date grouping creates ("Years").
    pub fn label(self, locale: &Locale) -> &'static str {
        let t: [&str; 5] = match self {
            GroupBy::Range => ["Group", "Groupe", "Grupo", "Gruppe", "Gruppo"],
            GroupBy::Seconds => ["Seconds", "Secondes", "Segundos", "Sekunden", "Secondi"],
            GroupBy::Minutes => ["Minutes", "Minutes", "Minutos", "Minuten", "Minuti"],
            GroupBy::Hours => ["Hours", "Heures", "Horas", "Stunden", "Ore"],
            GroupBy::Days => ["Days", "Jours", "Días", "Tage", "Giorni"],
            GroupBy::Months => ["Months", "Mois", "Meses", "Monate", "Mesi"],
            GroupBy::Quarters => [
                "Quarters",
                "Trimestres",
                "Trimestres",
                "Quartale",
                "Trimestri",
            ],
            GroupBy::Years => ["Years", "Années", "Años", "Jahre", "Anni"],
        };
        t[lang(locale)]
    }
}

/// Name of a field created by date grouping: "Years (Date)".
pub(crate) fn date_group_field_name(by: GroupBy, base: &str, locale: &Locale) -> String {
    format!("{} ({base})", by.label(locale))
}

/// A date as it appears in the bounds of a date group ("<1/1/2024").
pub(crate) fn short_date(serial: f64, locale: &Locale) -> String {
    format::format_number(serial.floor(), locale.short_date_format, locale).text
}

/// The items of a date grouping other than by years or by several days, in
/// order: "Jan".."Dec", "Qtr1".."Qtr4", "1-Jan".."31-Dec", the hours, the
/// minutes or the seconds.
pub(crate) fn date_group_items(by: GroupBy, locale: &Locale) -> Vec<String> {
    let l = lang(locale);
    match by {
        GroupBy::Months => locale.months_short.iter().map(|m| m.to_string()).collect(),
        GroupBy::Quarters => (1..=4)
            .map(|q| format!("{}{q}", caption(Caption::Quarter, locale)))
            .collect(),
        GroupBy::Days => {
            let mut out = Vec::with_capacity(366);
            for (m, name) in locale.months_short.iter().enumerate() {
                // A leap year, so that 29 February has its item.
                for d in 1..=crate::datetime::days_in_month(2024, m as u32 + 1) {
                    out.push(match l {
                        0 => format!("{d}-{name}"),
                        3 => format!("{d:02}.{name}"),
                        _ => format!("{d:02}-{name}"),
                    });
                }
            }
            out
        }
        GroupBy::Hours => (0..24u32)
            .map(|h| {
                if l == 0 {
                    format!(
                        "{} {}",
                        if h % 12 == 0 { 12 } else { h % 12 },
                        if h < 12 { "AM" } else { "PM" }
                    )
                } else {
                    format!("{h:02}")
                }
            })
            .collect(),
        GroupBy::Minutes | GroupBy::Seconds => (0..60).map(|n| format!(":{n:02}")).collect(),
        GroupBy::Years | GroupBy::Range => Vec::new(),
    }
}

/// Position of a date in the list `date_group_items` returns.
pub(crate) fn date_group_index(by: GroupBy, serial: f64) -> usize {
    let (_, m, d) = serial_to_ymd(serial);
    let (h, min, s) = serial_to_hms(serial);
    match by {
        GroupBy::Months => (m.max(1) - 1) as usize,
        GroupBy::Quarters => ((m.max(1) - 1) / 3) as usize,
        GroupBy::Days => {
            let before: u32 = (1..m)
                .map(|k| crate::datetime::days_in_month(2024, k))
                .sum();
            (before + d.max(1) - 1) as usize
        }
        GroupBy::Hours => h as usize,
        GroupBy::Minutes => min as usize,
        GroupBy::Seconds => s as usize,
        GroupBy::Years | GroupBy::Range => 0,
    }
}
