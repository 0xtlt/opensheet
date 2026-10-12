//! The small dialogs of the Highlight Cells Rules and Top/Bottom Rules
//! menus ("Greater Than", "Top 10 Items", ...): one or two values, the
//! format to give the cells, and the rule shown on the sheet while the
//! dialog is open.

use super::{add_rule, end_preview, preview, words};
use crate::app::ExcelApp;
use crate::features::{Cx, format_cells};
use crate::i18n::{t, t_ctx};
use crate::shell::{Alert, Modal, RefStyle};
use crate::text_input::TextInput;
use crate::widgets::{self, NumberField, RefEdit, Select, dialog, label, ok_cancel};
use gpui::{Context, Entity, SharedString, Window, div, prelude::*, px};
use opensheet_engine::condfmt::{
    CondFormat, HighlightFormat, Rule, TimePeriod, number_span, parse_operand, parse_text_operand,
};
use opensheet_engine::style::StylePatch;
use opensheet_engine::{Locale, Rect, Value, Workbook};

/// The dialogs, named after their menu entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    GreaterThan,
    LessThan,
    Between,
    EqualTo,
    TextContains,
    DateOccurring,
    DuplicateValues,
    TopItems,
    TopPercent,
    BottomItems,
    BottomPercent,
    AboveAverage,
    BelowAverage,
}

impl Kind {
    fn title(self) -> &'static str {
        match self {
            Kind::GreaterThan => t("Greater Than", "Supérieur à"),
            Kind::LessThan => t("Less Than", "Inférieur à"),
            Kind::Between => t("Between", "Entre"),
            Kind::EqualTo => t("Equal To", "Égal à"),
            Kind::TextContains => t("Text That Contains", "Texte qui contient"),
            Kind::DateOccurring => t("A Date Occurring", "Une date se produisant"),
            Kind::DuplicateValues => t("Duplicate Values", "Valeurs en double"),
            Kind::TopItems => t("Top 10 Items", "10 valeurs les plus élevées"),
            Kind::TopPercent => t("Top 10%", "10% des valeurs les plus élevées"),
            Kind::BottomItems => t("Bottom 10 Items", "10 valeurs les moins élevées"),
            Kind::BottomPercent => t("Bottom 10%", "10 % des valeurs les moins élevées"),
            Kind::AboveAverage => t_ctx("rule", "Above Average", "Valeurs supérieures à la moyenne"),
            Kind::BelowAverage => t_ctx("rule", "Below Average", "Valeurs inférieures à la moyenne"),
        }
    }

    /// The line over the controls.
    fn prompt(self) -> &'static str {
        match self {
            Kind::GreaterThan => t(
                "Format cells that are GREATER THAN:",
                "Mettre en forme les cellules dont le contenu est supérieur à :",
            ),
            Kind::LessThan => t(
                "Format cells that are LESS THAN:",
                "Mettre en forme les cellules dont le contenu est inférieur à :",
            ),
            Kind::Between => t("Format cells that are BETWEEN:", "Mettre en forme les cellules ENTRE :"),
            Kind::EqualTo => t(
                "Format cells that are EQUAL TO:",
                "Mettre en forme les cellules dont le contenu est égal à :",
            ),
            Kind::TextContains => t(
                "Format cells that contain the text:",
                "Mettre en forme les cellules contenant le texte :",
            ),
            Kind::DateOccurring => t(
                "Format cells that contain a date occurring:",
                "Mettre en forme les cellules dont la date correspond à :",
            ),
            Kind::DuplicateValues => t(
                "Format cells that contain:",
                "Mettre en forme les cellules contenant des valeurs :",
            ),
            Kind::TopItems | Kind::TopPercent => t(
                "Format cells that rank in the TOP:",
                "Mettre en forme les N valeurs les plus élevées :",
            ),
            Kind::BottomItems | Kind::BottomPercent => t(
                "Format cells that rank in the BOTTOM:",
                "Mettre en forme les N valeurs les moins élevées :",
            ),
            Kind::AboveAverage => t(
                "Format cells that are ABOVE AVERAGE:",
                "Appliquer la mise en forme suivante à la plage sélectionnée de cellules figurant au-dessus de la moyenne :",
            ),
            Kind::BelowAverage => t(
                "Format cells that are BELOW AVERAGE:",
                "Appliquer la mise en forme suivante à la plage sélectionnée de cellules figurant en dessous de la moyenne :",
            ),
        }
    }

    /// The dialog asks for a count or a percentage with a spinner.
    fn ranked(self) -> bool {
        matches!(self, Kind::TopItems | Kind::TopPercent | Kind::BottomItems | Kind::BottomPercent)
    }

    fn percent(self) -> bool {
        matches!(self, Kind::TopPercent | Kind::BottomPercent)
    }
}

/// The values a dialog opens with. Excel proposes the middle of the
/// numbers selected for "Greater Than" and "Less Than", the points three
/// and seven tenths of the way through them for "Between", and what the
/// active cell holds for "Equal To" and "Text That Contains".
pub fn defaults(kind: Kind, wb: &Workbook, sheet: usize, ranges: &[Rect], active: (u32, u32)) -> (String, String) {
    let locale = wb.locale;
    let span = || number_span(wb, sheet, ranges);
    let number = |n: f64| locale.number_to_text(n);
    match kind {
        Kind::GreaterThan | Kind::LessThan => (span().map(|(low, high)| number((low + high) / 2.0)).unwrap_or_default(), String::new()),
        Kind::Between => match span() {
            Some((low, high)) => (number(low + (high - low) * 0.3), number(low + (high - low) * 0.7)),
            None => (String::new(), String::new()),
        },
        Kind::EqualTo | Kind::TextContains => {
            let text = match wb.value_at(sheet, active.0, active.1) {
                Value::Number(n) => number(n),
                Value::Text(text) => text.to_string(),
                Value::Bool(b) => locale.bool_name(b).to_string(),
                _ => String::new(),
            };
            (text, String::new())
        }
        kind if kind.ranked() => ("10".to_string(), String::new()),
        _ => (String::new(), String::new()),
    }
}

fn invalid_value() -> &'static str {
    t(
        "The value you entered is not a valid number, date, time, or string.",
        "La valeur entrée n'est pas un nombre, une date, une heure ou une chaîne valide.",
    )
}

/// The rule a dialog stands for: `first` and `second` are its value
/// boxes, `choice` the position of its list (period, duplicate / unique).
/// The error is the message to show.
pub fn rule_of(
    kind: Kind,
    first: &str,
    second: &str,
    choice: usize,
    locale: &Locale,
    active: (u32, u32),
    anchor: (u32, u32),
) -> Result<Rule, &'static str> {
    let operand = |text: &str| {
        if text.trim().is_empty() {
            return Err(invalid_value());
        }
        parse_operand(text, locale, active, anchor).map_err(|_| t("Enter a valid formula.", "Entrez une formule valide."))
    };
    let rank = |limit: u32, message: &'static str| match first.trim().parse::<u32>() {
        Ok(n) if (1..=limit).contains(&n) => Ok(n),
        _ => Err(message),
    };
    let items = || {
        rank(1000, t("Enter a whole number between 1 and 1000.", "Entrez un nombre entier entre 1 et 1 000."))
    };
    let percent = || {
        rank(100, t("You must enter a number between 1 and 100.", "Vous devez entrer un nombre entre 1 et 100."))
    };
    Ok(match kind {
        Kind::GreaterThan => Rule::greater_than(operand(first)?),
        Kind::LessThan => Rule::less_than(operand(first)?),
        Kind::EqualTo => Rule::equal_to(operand(first)?),
        Kind::Between => Rule::between(operand(first)?, operand(second)?),
        Kind::TextContains => {
            if first.is_empty() {
                return Err(invalid_value());
            }
            Rule::text_contains(
                parse_text_operand(first, locale, active, anchor)
                    .map_err(|_| t("Enter a valid formula.", "Entrez une formule valide."))?,
            )
        }
        Kind::DateOccurring => Rule::date_occurring(TimePeriod::ALL[choice.min(TimePeriod::ALL.len() - 1)]),
        Kind::DuplicateValues if choice == 1 => Rule::UniqueValues,
        Kind::DuplicateValues => Rule::DuplicateValues,
        Kind::TopItems => Rule::top_items(items()?),
        Kind::TopPercent => Rule::top_percent(percent()?),
        Kind::BottomItems => Rule::bottom_items(items()?),
        Kind::BottomPercent => Rule::bottom_percent(percent()?),
        Kind::AboveAverage => Rule::above_average(),
        Kind::BelowAverage => Rule::below_average(),
    })
}

/// Position of "Custom Format..." in the list of formats.
const CUSTOM: usize = HighlightFormat::ALL.len();

/// What the dialog needs from the application, gathered before it opens.
struct Setup {
    kind: Kind,
    locale: &'static Locale,
    active: (u32, u32),
    ranges: Vec<Rect>,
    first: String,
    second: String,
}

pub fn open(app: &mut ExcelApp, kind: Kind, window: &mut Window, cx: &mut Cx) {
    let ranges = app.sel().ranges.clone();
    let active = app.active();
    let (first, second) = defaults(kind, &app.wb, app.sheet, &ranges, active);
    let setup = Setup {
        kind,
        locale: app.wb.locale,
        active,
        ranges,
        first,
        second,
    };
    let view = app.open_modal(window, cx, move |modal, _, cx| Dialog::new(modal, setup, cx));
    // The rule shows on the sheet from the start, and leaves it with the
    // dialog however that is closed.
    cx.observe_release(&view, |app, _, cx| end_preview(app, cx)).detach();
    let first = view.read(cx).format_rule(cx);
    view.update(cx, |dialog, _| dialog.shown = first.clone());
    preview(app, first, cx);
}

struct Dialog {
    modal: Modal,
    kind: Kind,
    locale: &'static Locale,
    active: (u32, u32),
    ranges: Vec<Rect>,
    first: Entity<TextInput>,
    second: Entity<TextInput>,
    /// Position of the list of periods, or of "Duplicate / Unique".
    choice: usize,
    /// Position of the list of formats.
    format: usize,
    custom: StylePatch,
    /// The rule the sheet currently shows.
    shown: Option<CondFormat>,
}

impl Dialog {
    fn new(modal: Modal, setup: Setup, cx: &mut Context<Self>) -> Self {
        // A cell picked on the sheet is written as `=$A$1`.
        let style = RefStyle {
            absolute: true,
            sheet: false,
            equals: true,
        };
        let first = if setup.kind.ranked() {
            let limit = if setup.kind.percent() { 100.0 } else { 1000.0 };
            widgets::number_input(cx, Some(10.0), 1.0, limit, 1.0)
        } else {
            modal.ref_input(style, setup.first, cx)
        };
        let second = modal.ref_input(style, setup.second, cx);
        // Typing, stepping and picking cells all end here.
        for input in [&first, &second] {
            cx.observe(input, |this, _, cx| this.refresh_preview(cx)).detach();
        }
        Dialog {
            modal,
            kind: setup.kind,
            locale: setup.locale,
            active: setup.active,
            ranges: setup.ranges,
            first,
            second,
            choice: 0,
            format: 0,
            custom: StylePatch::default(),
            shown: None,
        }
    }

    fn anchor(&self) -> (u32, u32) {
        self.ranges.first().map(|r| (r.r0, r.c0)).unwrap_or(self.active)
    }

    fn rule(&self, cx: &gpui::App) -> Result<Rule, &'static str> {
        rule_of(
            self.kind,
            self.first.read(cx).text(),
            self.second.read(cx).text(),
            self.choice,
            self.locale,
            self.active,
            self.anchor(),
        )
    }

    fn patch(&self) -> StylePatch {
        match HighlightFormat::ALL.get(self.format) {
            Some(format) => format.patch(),
            None => self.custom.clone(),
        }
    }

    /// The rule with its format on the selected cells; `None` while the
    /// values do not make a rule.
    fn format_rule(&self, cx: &gpui::App) -> Option<CondFormat> {
        let rule = self.rule(cx).ok()?;
        Some(CondFormat::new(self.ranges.clone(), rule).with_format(self.patch()))
    }

    fn refresh_preview(&mut self, cx: &mut Context<Self>) {
        let rule = self.format_rule(cx);
        if rule != self.shown {
            self.shown = rule.clone();
            self.modal.update_app(cx, |app, cx| preview(app, rule, cx));
        }
    }

    fn pick_format(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index < CUSTOM {
            self.format = index;
            self.refresh_preview(cx);
            cx.notify();
            return;
        }
        // "Custom Format...": the list only changes once a format is chosen.
        let start = self.custom.clone();
        let chosen = cx.listener(|this, format: &StylePatch, _, cx| {
            this.custom = format.clone();
            this.format = CUSTOM;
            this.refresh_preview(cx);
            cx.notify();
        });
        self.modal
            .update_app(cx, |app, cx| format_cells::open_partial(app, start, window, cx, chosen));
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        match self.rule(cx) {
            Ok(rule) => {
                let format = self.patch();
                self.modal.update_app(cx, |app, cx| add_rule(app, rule, format, cx));
                self.modal.close(window, cx);
            }
            Err(message) => {
                self.modal
                    .update_app(cx, |app, cx| app.alert(Alert::new("OpenSheet", message).warning(), cx));
            }
        }
    }
}

impl Render for Dialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let formats: Vec<SharedString> = HighlightFormat::ALL
            .iter()
            .map(|format| words::highlight_name(*format).into())
            .chain(std::iter::once(words::custom_format_name().into()))
            .collect();
        let format = Select::new("format", formats, Some(self.format))
            .width(320.0)
            .on_change(cx.listener(|this, ix: &usize, window, cx| this.pick_format(*ix, window, cx)));
        let with = t("with", "avec");
        let choice = |options: Vec<SharedString>, width: f32, this: &Self, cx: &mut Context<Self>| {
            Select::new("choice", options, Some(this.choice))
                .width(width)
                .on_change(cx.listener(|this, ix: &usize, _, cx| {
                    this.choice = *ix;
                    this.refresh_preview(cx);
                    cx.notify();
                }))
        };

        let mut row = div().flex().items_center().gap(px(8.));
        row = match self.kind {
            Kind::GreaterThan | Kind::LessThan | Kind::EqualTo | Kind::TextContains => row
                .child(RefEdit::new(&self.first, &self.modal).width(200.0))
                .child(label(with)),
            Kind::Between => row
                .child(RefEdit::new(&self.first, &self.modal).width(130.0))
                .child(label(t("and", "et")))
                .child(RefEdit::new(&self.second, &self.modal).width(130.0))
                .child(label(with)),
            Kind::DateOccurring => {
                let periods = TimePeriod::ALL.iter().map(|p| words::period_name(*p).into()).collect();
                row.child(choice(periods, 200.0, self, cx)).child(label(with))
            }
            Kind::DuplicateValues => {
                let kinds = vec![t_ctx("values", "Duplicate", "en double").into(), t("Unique", "uniques").into()];
                row.child(choice(kinds, 110.0, self, cx)).child(label(t("values with", "avec")))
            }
            Kind::TopItems | Kind::BottomItems => row.child(NumberField::new("rank", &self.first).width(70.0)).child(label(with)),
            Kind::TopPercent | Kind::BottomPercent => row
                .child(NumberField::new("rank", &self.first).width(70.0))
                .child(label(t("% with", "% avec"))),
            Kind::AboveAverage | Kind::BelowAverage => row.child(label(t("for the selected range with", "avec"))),
        };

        dialog(&self.modal, self.kind.title())
            .width(if self.kind == Kind::Between { 690.0 } else { 600.0 })
            .on_ok(cx.listener(Self::ok))
            .child(div().line_height(px(17.)).child(self.kind.prompt()))
            .child(row.child(format))
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::Expr;
    use opensheet_engine::condfmt::CellOp;

    fn book(values: &[&str]) -> Workbook {
        let mut wb = Workbook::new(Locale::en());
        for (row, value) in values.iter().enumerate() {
            let _ = wb.set_cell_text(0, row as u32, 0, value);
        }
        wb.commit("test");
        wb
    }

    fn column(rows: u32) -> Vec<Rect> {
        vec![Rect::new(0, 0, rows - 1, 0)]
    }

    #[test]
    fn greater_than_proposes_the_middle_of_the_numbers() {
        let wb = book(&["1", "2", "3", "4", "100"]);
        assert_eq!(defaults(Kind::GreaterThan, &wb, 0, &column(5), (0, 0)), ("50.5".to_string(), String::new()));
        assert_eq!(defaults(Kind::LessThan, &wb, 0, &column(5), (0, 0)).0, "50.5");
    }

    #[test]
    fn between_proposes_three_and_seven_tenths() {
        let wb = book(&["1", "10", "4"]);
        assert_eq!(defaults(Kind::Between, &wb, 0, &column(3), (0, 0)), ("3.7".to_string(), "7.3".to_string()));
    }

    #[test]
    fn equal_to_and_text_propose_the_active_cell() {
        let wb = book(&["apple", "7", "pear"]);
        assert_eq!(defaults(Kind::TextContains, &wb, 0, &column(3), (0, 0)).0, "apple");
        assert_eq!(defaults(Kind::EqualTo, &wb, 0, &column(3), (1, 0)).0, "7");
        // Nothing to propose from cells without numbers.
        let words = book(&["a", "b"]);
        assert_eq!(defaults(Kind::GreaterThan, &words, 0, &column(2), (0, 0)), (String::new(), String::new()));
        assert_eq!(defaults(Kind::TopItems, &words, 0, &column(2), (0, 0)).0, "10");
    }

    fn make(kind: Kind, first: &str, second: &str, choice: usize) -> Result<Rule, &'static str> {
        rule_of(kind, first, second, choice, Locale::en(), (0, 0), (0, 0))
    }

    #[test]
    fn the_value_dialogs_make_cell_rules() {
        assert_eq!(make(Kind::GreaterThan, "5", "", 0), Ok(Rule::greater_than(Expr::Number(5.0))));
        assert_eq!(
            make(Kind::Between, "1", "abc", 0),
            Ok(Rule::between(Expr::Number(1.0), Expr::Text("abc".into())))
        );
        assert!(matches!(
            make(Kind::EqualTo, "=$B$1", "", 0),
            Ok(Rule::CellIs { op: CellOp::Equal, first: Expr::Ref(_), .. })
        ));
        assert!(make(Kind::LessThan, "  ", "", 0).is_err());
        assert!(make(Kind::Between, "1", "", 0).is_err());
        assert!(make(Kind::GreaterThan, "=SUM(", "", 0).is_err());
    }

    #[test]
    fn a_picked_cell_is_stored_for_the_anchor() {
        // Typed for the active cell B3, stored for the anchor B1: the row
        // of a relative reference moves with it.
        let rule = rule_of(Kind::GreaterThan, "=A3", "", 0, Locale::en(), (2, 1), (0, 1)).unwrap();
        let Rule::CellIs { first, .. } = rule else {
            panic!("a cell rule");
        };
        assert_eq!(first.to_text(Locale::en()), "A1");
    }

    #[test]
    fn the_list_dialogs_follow_their_choice() {
        assert_eq!(make(Kind::DateOccurring, "", "", 0), Ok(Rule::date_occurring(TimePeriod::Yesterday)));
        assert_eq!(make(Kind::DateOccurring, "", "", 9), Ok(Rule::date_occurring(TimePeriod::NextMonth)));
        assert_eq!(make(Kind::DuplicateValues, "", "", 0), Ok(Rule::DuplicateValues));
        assert_eq!(make(Kind::DuplicateValues, "", "", 1), Ok(Rule::UniqueValues));
        assert_eq!(make(Kind::TextContains, "ab", "", 0), Ok(Rule::text_contains(Expr::Text("ab".into()))));
        assert!(make(Kind::TextContains, "", "", 0).is_err());
        assert_eq!(make(Kind::AboveAverage, "", "", 0), Ok(Rule::above_average()));
    }

    #[test]
    fn ranks_stay_within_excels_limits() {
        assert_eq!(make(Kind::TopItems, "10", "", 0), Ok(Rule::top_items(10)));
        assert_eq!(make(Kind::BottomPercent, " 25 ", "", 0), Ok(Rule::bottom_percent(25)));
        assert!(make(Kind::TopItems, "0", "", 0).is_err());
        assert!(make(Kind::TopItems, "1001", "", 0).is_err());
        assert!(make(Kind::TopPercent, "101", "", 0).is_err());
        assert!(make(Kind::BottomItems, "2.5", "", 0).is_err());
    }
}
