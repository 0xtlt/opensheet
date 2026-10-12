//! The two dialogs of the conditions submenu of a filter drop-down: Custom
//! AutoFilter (one or two comparisons) and Top 10 AutoFilter.

use super::{APP_NAME, apply_criteria, column_title};
use crate::app::ExcelApp;
use crate::features::Cx;
use crate::i18n::t;
use crate::shell::{Alert, Modal};
use crate::text_input::TextInput;
use crate::widgets::{self, ComboBox, NumberField, RadioGroup, Select, dialog, label, ok_cancel};
use gpui::{Context, Entity, Focusable, SharedString, Window, div, prelude::*, px};
use opensheet_engine::Locale;
use opensheet_engine::datetime::date_to_serial;
use opensheet_engine::filter::{
    ColumnKind, Comparison, Condition, Criteria, CustomFilter, FilterDropdown, FilterScope, Top10,
};
use opensheet_engine::format;

// ---------------------------------------------------------- custom autofilter

/// The comparisons the dialog offers, in the order of its lists, with the
/// wording for the kind of column (dates are "after", not "greater than").
pub(super) fn comparisons(kind: ColumnKind) -> Vec<(Comparison, &'static str)> {
    let date = kind == ColumnKind::Date;
    vec![
        (Comparison::Equals, t("equals", "est égal à")),
        (Comparison::NotEquals, t("does not equal", "est différent de")),
        (
            Comparison::Greater,
            if date { t("is after", "postérieur au") } else { t("is greater than", "est supérieur à") },
        ),
        (
            Comparison::GreaterOrEqual,
            if date {
                t("is after or equal to", "postérieur ou égal au")
            } else {
                t("is greater than or equal to", "est supérieur ou égal à")
            },
        ),
        (
            Comparison::Less,
            if date { t("is before", "antérieur au") } else { t("is less than", "est inférieur à") },
        ),
        (
            Comparison::LessOrEqual,
            if date {
                t("is before or equal to", "antérieur ou égal au")
            } else {
                t("is less than or equal to", "est inférieur ou égal à")
            },
        ),
        (Comparison::BeginsWith, t("begins with", "commence par")),
        (Comparison::NotBeginsWith, t("does not begin with", "ne commence pas par")),
        (Comparison::EndsWith, t("ends with", "se termine par")),
        (Comparison::NotEndsWith, t("does not end with", "ne se termine pas par")),
        (Comparison::Contains, t("contains", "contient")),
        (Comparison::NotContains, t("does not contain", "ne contient pas")),
    ]
}

/// What the dialog holds: two comparisons (`None`: the blank choice of
/// the list, the line is not used), their operands as typed, And / Or.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Fields {
    pub comparisons: [Option<Comparison>; 2],
    pub operands: [String; 2],
    pub and: bool,
}

impl Fields {
    /// How the dialog opens: on the comparisons of the entry that opened
    /// it ("Begins With..."), else on the column's custom filter, else on
    /// "equals".
    pub(super) fn opening(
        criteria: Option<&Criteria>,
        preset: Option<(Comparison, Option<Comparison>)>,
        as_date: bool,
        locale: &Locale,
    ) -> Fields {
        if let Some((first, second)) = preset {
            return Fields {
                comparisons: [Some(first), second],
                operands: Default::default(),
                and: true,
            };
        }
        let Some(Criteria::Custom(custom)) = criteria else {
            return Fields {
                comparisons: [Some(Comparison::Equals), None],
                operands: Default::default(),
                and: true,
            };
        };
        let second = custom.second.as_ref();
        Fields {
            comparisons: [Some(custom.first.comparison().0), second.map(|c| c.comparison().0)],
            operands: [
                custom.first.operand_text(locale, as_date),
                second.map(|c| c.operand_text(locale, as_date)).unwrap_or_default(),
            ],
            and: second.is_none() || custom.and,
        }
    }

    /// The criterion OK gives the column: the lines whose comparison is
    /// chosen; `None` (no filter) when neither is. A line left without an
    /// operand only counts when it can mean "is blank" / "is not blank".
    pub(super) fn criteria(&self, locale: &Locale) -> Option<Criteria> {
        let mut conditions = self.comparisons.iter().zip(&self.operands).filter_map(|(comparison, operand)| {
            let (comparison, operand) = ((*comparison)?, operand.trim());
            let about_blanks = matches!(comparison, Comparison::Equals | Comparison::NotEquals);
            (about_blanks || !operand.is_empty()).then(|| Condition::new(comparison, operand, locale))
        });
        let first = conditions.next()?;
        Some(Criteria::Custom(match conditions.next() {
            Some(second) => CustomFilter {
                first,
                second: Some(second),
                and: self.and,
            },
            None => CustomFilter::one(first),
        }))
    }
}

/// What the value boxes of the dialog suggest: the values of the column,
/// its dates written as short dates.
pub(super) fn suggestions(data: &FilterDropdown, locale: &Locale) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for year in data.dates.iter().rev() {
        for month in &year.months {
            for day in &month.days {
                if let Some(serial) = date_to_serial(year.year as i64, month.month as i64, day.day as i64) {
                    out.push(format::format_number(serial, locale.short_date_format, locale).text);
                }
            }
        }
    }
    out.extend(data.items.iter().map(|item| item.text.clone()));
    out
}

struct CustomDialog {
    modal: Modal,
    sheet: usize,
    scope: FilterScope,
    col_id: u32,
    title: SharedString,
    kind: ColumnKind,
    locale: &'static Locale,
    comparisons: [Option<Comparison>; 2],
    and: bool,
    operands: [Entity<TextInput>; 2],
    suggestions: Vec<SharedString>,
    /// The dialog has not put the keyboard in its first value box yet.
    opening: bool,
}

/// Open the Custom AutoFilter dialog for a column. `preset`: the
/// comparisons of the menu entry that asked for it.
pub(super) fn open(
    app: &mut ExcelApp,
    scope: FilterScope,
    col_id: u32,
    preset: Option<(Comparison, Option<Comparison>)>,
    window: &mut Window,
    cx: &mut Cx,
) {
    let sheet = app.sheet;
    let locale = app.wb.locale;
    let Some(data) = app.wb.filter_dropdown(sheet, scope, col_id) else {
        return;
    };
    let Some(first_col) = app.wb.auto_filter(sheet, scope).map(|filter| filter.range.c0) else {
        return;
    };
    let kind = data.kind;
    let fields = Fields::opening(data.criteria.as_ref(), preset, kind == ColumnKind::Date, locale);
    let title: SharedString = column_title(&data.header, first_col + col_id).into();
    let suggestions: Vec<SharedString> = suggestions(&data, locale).into_iter().map(SharedString::from).collect();
    app.open_modal(window, cx, move |modal, _, cx| {
        let [first, second] = fields.operands;
        CustomDialog {
            modal,
            sheet,
            scope,
            col_id,
            title,
            kind,
            locale,
            comparisons: fields.comparisons,
            and: fields.and,
            operands: [widgets::input(cx, first), widgets::input(cx, second)],
            suggestions,
            opening: true,
        }
    });
}

impl CustomDialog {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let fields = Fields {
            comparisons: self.comparisons,
            operands: [
                self.operands[0].read(cx).text().to_string(),
                self.operands[1].read(cx).text().to_string(),
            ],
            and: self.and,
        };
        let criteria = fields.criteria(self.locale);
        let (sheet, scope, col_id) = (self.sheet, self.scope, self.col_id);
        self.modal.close(window, cx);
        self.modal
            .update_app(cx, |app, cx| apply_criteria(app, sheet, scope, col_id, criteria, cx));
    }

    /// One line of the dialog: a comparison and its operand.
    fn line(&self, index: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let choices = comparisons(self.kind);
        // The list starts with a blank line: "this comparison is not used".
        let mut names: Vec<SharedString> = vec![SharedString::default()];
        names.extend(choices.iter().map(|(_, name)| SharedString::from(*name)));
        let selected = match self.comparisons[index] {
            Some(comparison) => choices.iter().position(|(c, _)| *c == comparison).map(|at| at + 1),
            None => Some(0),
        };
        div()
            .flex()
            .gap(px(10.))
            .child(
                Select::new(("custom-comparison", index), names, selected)
                    .width(200.0)
                    .on_change(cx.listener(move |this, at: &usize, _, cx| {
                        this.comparisons[index] = at.checked_sub(1).map(|at| comparisons(this.kind)[at].0);
                        cx.notify();
                    })),
            )
            .child(ComboBox::new(("custom-operand", index), &self.operands[index], self.suggestions.clone()).width(220.0))
    }
}

impl Render for CustomDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.opening) {
            // The comparison is chosen already: typing goes to its value,
            // as in Excel. (A dialog starts on its first control; this
            // runs right after.)
            cx.on_next_frame(window, |this, window, cx| {
                let field = this.operands[0].clone();
                window.focus(&field.read(cx).focus_handle(cx));
                field.update(cx, |field, cx| field.select_all_text(cx));
            });
        }
        let joints: Vec<SharedString> = vec![t("And", "Et").into(), t("Or", "Ou").into()];
        let first = self.line(0, cx);
        let second = self.line(1, cx);
        dialog(&self.modal, t("Custom AutoFilter", "Filtre automatique personnalisé"))
            .width(480.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(t("Show rows where:", "Afficher les lignes dans lesquelles\u{a0}:")))
            .child(
                div()
                    .pl(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(label(self.title.clone()))
                    .child(first)
                    .child(
                        div().pl(px(36.)).child(
                            RadioGroup::new("custom-joint", joints, Some(if self.and { 0 } else { 1 }))
                                .horizontal()
                                .on_change(cx.listener(|this, at: &usize, _, cx| {
                                    this.and = *at == 0;
                                    cx.notify();
                                })),
                        ),
                    )
                    .child(second),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(label(t(
                        "Use ? to represent any single character",
                        "Utilisez ? pour représenter un caractère",
                    )))
                    .child(label(t(
                        "Use * to represent any series of characters",
                        "Utilisez * pour représenter une série de caractères",
                    ))),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// --------------------------------------------------------------------- top 10

/// The bounds of the count of the Top 10 dialog.
const TOP_RANGE: (f64, f64) = (1.0, 500.0);

/// The criterion of the Top 10 dialog, when its count is a whole number
/// Excel accepts.
pub(super) fn top10_criteria(top: bool, percent: bool, count: Option<f64>) -> Option<Criteria> {
    let count = count.filter(|n| n.fract() == 0.0 && (TOP_RANGE.0..=TOP_RANGE.1).contains(n))?;
    Some(Criteria::Top10(Top10::new(top, percent, count)))
}

struct Top10Dialog {
    modal: Modal,
    sheet: usize,
    scope: FilterScope,
    col_id: u32,
    top: bool,
    percent: bool,
    count: Entity<TextInput>,
}

/// Open the Top 10 AutoFilter dialog for a column, on its current top /
/// bottom filter if it has one.
pub(super) fn open_top10(app: &mut ExcelApp, scope: FilterScope, col_id: u32, window: &mut Window, cx: &mut Cx) {
    let sheet = app.sheet;
    let current = app
        .wb
        .auto_filter(sheet, scope)
        .and_then(|filter| match filter.columns.get(&col_id) {
            Some(Criteria::Top10(top)) => Some(*top),
            _ => None,
        })
        .unwrap_or(Top10::new(true, false, 10.0));
    app.open_modal(window, cx, move |modal, _, cx| Top10Dialog {
        modal,
        sheet,
        scope,
        col_id,
        top: current.top,
        percent: current.percent,
        count: widgets::number_input(cx, Some(current.value), TOP_RANGE.0, TOP_RANGE.1, 1.0),
    });
}

impl Top10Dialog {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let Some(criteria) = top10_criteria(self.top, self.percent, self.count.read(cx).number()) else {
            let message = t(
                "The number must be between 1 and 500. Try again by entering a number in this range.",
                "Le nombre doit être compris entre 1 et 500. Réessayez en tapant un nombre compris entre ces valeurs.",
            );
            self.modal
                .update_app(cx, |app, cx| app.alert(Alert::new(APP_NAME, message).warning(), cx));
            return;
        };
        let (sheet, scope, col_id) = (self.sheet, self.scope, self.col_id);
        self.modal.close(window, cx);
        self.modal
            .update_app(cx, |app, cx| apply_criteria(app, sheet, scope, col_id, Some(criteria), cx));
    }
}

impl Render for Top10Dialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ends: Vec<SharedString> = vec![crate::i18n::t_ctx("top10", "Top", "Haut").into(), crate::i18n::t_ctx("top10", "Bottom", "Bas").into()];
        let units: Vec<SharedString> = vec![t("Items", "Éléments").into(), t("Percent", "Pourcentage").into()];
        dialog(&self.modal, t("Top 10 AutoFilter", "Filtre automatique des 10 premiers"))
            .width(360.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(t("Show", "Afficher")))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        Select::new("top-end", ends, Some(if self.top { 0 } else { 1 }))
                            .width(96.0)
                            .on_change(cx.listener(|this, at: &usize, _, cx| {
                                this.top = *at == 0;
                                cx.notify();
                            })),
                    )
                    .child(NumberField::new("top-count", &self.count).width(72.0))
                    .child(
                        Select::new("top-unit", units, Some(if self.percent { 1 } else { 0 }))
                            .width(130.0)
                            .on_change(cx.listener(|this, at: &usize, _, cx| {
                                this.percent = *at == 1;
                                cx.notify();
                            })),
                    ),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}
