//! The Conditional Formatting Rules Manager: the rules of the selection,
//! of the sheet or of another sheet in priority order, with what each one
//! does, where it applies and whether it stops the ones below it.
//!
//! The manager edits a copy of the rules of every sheet; OK and Apply
//! write the copies back as one undo step.

use super::draft::{Draft, Graded, Kind, set_icons};
use super::rule_dialog::{self, Setup};
use super::{draw, end_preview, format_sample, undo_label, words};
use crate::app::ExcelApp;
use crate::features::Cx;
use crate::i18n::t;
use crate::shell::{Alert, Modal, RefStyle};
use crate::text_input::TextInput;
use crate::theme::{self, c};
use crate::widgets::{Button, Checkbox, RefEdit, ScrollArea, Select, dialog, labeled};
use gpui::{AnyElement, Context, Entity, FontWeight, SharedString, Window, div, prelude::*, px};
use opensheet_engine::condfmt::{Bar, CondFormat, Rule, parse_ranges, ranges_text};
use opensheet_engine::style::StylePatch;
use opensheet_engine::{Locale, Rect};
use std::rc::Rc;

/// What the manager needs from the application, gathered before it opens.
struct Opening {
    locale: &'static Locale,
    current: usize,
    names: Vec<String>,
    rules: Vec<Vec<CondFormat>>,
    /// The selected ranges and the active cell of every sheet.
    selections: Vec<(Vec<Rect>, (u32, u32))>,
}

pub fn open(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    end_preview(app, cx);
    let setup = Opening {
        locale: app.wb.locale,
        current: app.sheet,
        names: app.wb.sheets.iter().map(|sheet| sheet.name.clone()).collect(),
        rules: app.wb.sheets.iter().map(|sheet| sheet.cond_formats.clone()).collect(),
        selections: (0..app.wb.sheets.len())
            .map(|sheet| match app.views.get(sheet) {
                Some(view) => (view.sel.ranges.clone(), view.sel.active),
                None => (vec![Rect::cell(0, 0)], (0, 0)),
            })
            .collect(),
    };
    app.open_modal(window, cx, move |modal, _, cx| Manager::new(modal, setup, cx));
}

/// The rules of `rules` that touch one of `selection`, as positions in
/// the list (what "Current Selection" shows).
fn rules_in(rules: &[CondFormat], selection: &[Rect]) -> Vec<usize> {
    (0..rules.len())
        .filter(|i| selection.iter().any(|rect| rules[*i].intersects(rect)))
        .collect()
}

/// Where the rule at `from` goes when it is moved one place up or down
/// among the rules on show (`shown`, positions in the whole list): the
/// place of its neighbour there. `None` at either end.
fn move_target(shown: &[usize], from: usize, up: bool) -> Option<usize> {
    let at = shown.iter().position(|i| *i == from)?;
    match up {
        true => at.checked_sub(1).map(|above| shown[above]),
        false => shown.get(at + 1).copied(),
    }
}

/// One line of the list: a rule and the box of its "Applies to".
struct Row {
    /// Position of the rule among the rules of its sheet.
    rule: usize,
    applies: Entity<TextInput>,
}

struct Manager {
    modal: Modal,
    locale: &'static Locale,
    current: usize,
    names: Vec<String>,
    /// The rules of every sheet, as edited here.
    rules: Vec<Vec<CondFormat>>,
    selections: Vec<(Vec<Rect>, (u32, u32))>,
    /// Position in "Show formatting rules for": the selection, the
    /// current sheet, then the other sheets in order.
    scope: usize,
    /// The rule selected in the list (a position among the rules of the
    /// sheet on show).
    selected: Option<usize>,
    rows: Vec<Row>,
}

impl Manager {
    fn new(modal: Modal, setup: Opening, cx: &mut Context<Self>) -> Self {
        let mut manager = Manager {
            modal,
            locale: setup.locale,
            current: setup.current,
            names: setup.names,
            rules: setup.rules,
            selections: setup.selections,
            scope: 0,
            selected: None,
            rows: Vec::new(),
        };
        manager.rebuild(cx);
        manager
    }

    /// The sheet whose rules are on show.
    fn sheet(&self) -> usize {
        match self.scope {
            0 | 1 => self.current,
            other => (0..self.names.len())
                .filter(|sheet| *sheet != self.current)
                .nth(other - 2)
                .unwrap_or(self.current),
        }
    }

    /// The rules on show, as positions among the rules of their sheet.
    fn shown(&self) -> Vec<usize> {
        let sheet = self.sheet();
        match self.scope {
            0 => rules_in(&self.rules[sheet], &self.selections[sheet].0),
            _ => (0..self.rules[sheet].len()).collect(),
        }
    }

    /// Make the lines of the list again, after the rules or the scope
    /// changed.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let sheet = self.sheet();
        let style = RefStyle {
            absolute: true,
            sheet: false,
            equals: true,
        };
        self.rows = self
            .shown()
            .into_iter()
            .map(|rule| {
                let text = ranges_text(&self.rules[sheet][rule].ranges, self.locale);
                let applies = self.modal.ref_input(style, text, cx);
                // Typing a range or picking it on the sheet moves the rule
                // as soon as what the box holds is a range.
                cx.observe(&applies, move |this, input, cx| {
                    let sheet = this.sheet();
                    let ranges = parse_ranges(input.read(cx).text());
                    if let (Some(ranges), Some(cf)) = (ranges, this.rules[sheet].get_mut(rule)) {
                        if cf.ranges != ranges {
                            cf.set_ranges(ranges);
                        }
                    }
                })
                .detach();
                Row { rule, applies }
            })
            .collect();
        if self.selected.is_some_and(|selected| self.rows.iter().all(|row| row.rule != selected)) {
            self.selected = None;
        }
        cx.notify();
    }

    fn selected_rule(&self) -> Option<&CondFormat> {
        self.rules[self.sheet()].get(self.selected?)
    }

    fn open_rule_dialog(&mut self, setup: Setup, window: &mut Window, cx: &mut Context<Self>) {
        self.modal.update_app(cx, |app, cx| rule_dialog::open(app, setup, window, cx));
    }

    fn new_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sheet = self.sheet();
        let (ranges, active) = self.selections[sheet].clone();
        let anchor = ranges.first().map(|r| (r.r0, r.c0)).unwrap_or(active);
        let on_ok = cx.listener(move |this, (rule, format): &(Rule, StylePatch), _, cx| {
            let cf = CondFormat::new(ranges.clone(), rule.clone()).with_format(format.clone());
            this.rules[sheet].insert(0, cf);
            this.selected = Some(0);
            this.rebuild(cx);
        });
        let setup = Setup {
            title: rule_dialog::new_title(),
            draft: Draft::new(Kind::Values, Graded::TwoColor),
            locale: self.locale,
            active,
            anchor,
            on_ok: Rc::new(on_ok),
        };
        self.open_rule_dialog(setup, window, cx);
    }

    fn edit_rule(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sheet = self.sheet();
        let (Some(index), Some(cf)) = (self.selected, self.selected_rule()) else {
            return;
        };
        let active = self.selections[sheet].1;
        let on_ok = cx.listener(move |this, (rule, format): &(Rule, StylePatch), _, cx| {
            if let Some(cf) = this.rules[sheet].get_mut(index) {
                cf.rule = rule.clone();
                cf.format = format.clone();
                // Only rules that test a condition can stop the others.
                cf.stop_if_true &= !cf.rule.is_graded();
            }
            this.rebuild(cx);
        });
        let setup = Setup {
            title: rule_dialog::edit_title(),
            draft: Draft::of(cf, self.locale, active),
            locale: self.locale,
            active,
            anchor: cf.anchor(),
            on_ok: Rc::new(on_ok),
        };
        self.open_rule_dialog(setup, window, cx);
    }

    fn delete_rule(&mut self, cx: &mut Context<Self>) {
        let sheet = self.sheet();
        let Some(index) = self.selected.filter(|index| *index < self.rules[sheet].len()) else {
            return;
        };
        // The selection goes to the rule that takes the place.
        let place = self.rows.iter().position(|row| row.rule == index).unwrap_or(0);
        self.rules[sheet].remove(index);
        self.selected = None;
        self.rebuild(cx);
        self.selected = self.rows.get(place).or(self.rows.last()).map(|row| row.rule);
    }

    fn duplicate_rule(&mut self, cx: &mut Context<Self>) {
        let sheet = self.sheet();
        let Some(index) = self.selected.filter(|index| *index < self.rules[sheet].len()) else {
            return;
        };
        let copy = self.rules[sheet][index].clone();
        self.rules[sheet].insert(index + 1, copy);
        self.selected = Some(index + 1);
        self.rebuild(cx);
    }

    fn move_rule(&mut self, up: bool, cx: &mut Context<Self>) {
        let sheet = self.sheet();
        let Some(from) = self.selected else {
            return;
        };
        let Some(to) = move_target(&self.shown(), from, up) else {
            return;
        };
        let cf = self.rules[sheet].remove(from);
        self.rules[sheet].insert(to, cf);
        self.selected = Some(to);
        self.rebuild(cx);
    }

    /// Write the rules back; false (and a message) when an "Applies to"
    /// box does not hold a range.
    fn apply(&mut self, cx: &mut Context<Self>) -> bool {
        if self.rows.iter().any(|row| parse_ranges(row.applies.read(cx).text()).is_none()) {
            let message = t("Reference is not valid.", "Référence non valide.");
            self.modal
                .update_app(cx, |app, cx| app.alert(Alert::new("OpenSheet", message).warning(), cx));
            return false;
        }
        let rules = self.rules.clone();
        self.modal.update_app(cx, |app, cx| {
            let mut changed = false;
            for (sheet, rules) in rules.into_iter().enumerate() {
                changed |= app.wb.set_cond_formats(sheet, rules);
            }
            if changed {
                app.commit(undo_label(), cx);
            }
        });
        true
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        if self.apply(cx) {
            self.modal.close(window, cx);
        }
    }

    /// What a rule does, in small: its format, its colours, its bar or
    /// its icons.
    fn format_cell(cf: &CondFormat) -> AnyElement {
        match &cf.rule {
            Rule::ColorScale(scale) => {
                let colors: Vec<_> = scale.stops.iter().map(|stop| stop.color).collect();
                div()
                    .border_1()
                    .border_color(c(theme::BORDER_STRONG))
                    .child(draw::picture(128.0, 18.0, move |bounds, out| draw::scale_strip(&colors, bounds, out)))
                    .into_any_element()
            }
            Rule::DataBar(bar) => {
                let sample = Bar {
                    start: 0.0,
                    end: 0.75,
                    fill: bar.fill,
                    gradient: bar.gradient,
                    border: bar.border,
                    negative: false,
                    axis: None,
                    axis_color: bar.axis_color,
                };
                draw::picture(130.0, 20.0, move |bounds, out| {
                    let (x, y) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
                    draw::bar(&sample, (x, y, x + 130.0, y + 21.0), 1.0, out);
                })
                .into_any_element()
            }
            Rule::IconSet(icons) => {
                // The icons the bands end up with, highest band first.
                let shown = Draft::of(cf, Locale::en(), cf.anchor()).icons;
                let plain = shown == set_icons(icons.set);
                if plain {
                    return draw::set_picture(icons.set, 16.0, 3.0).into_any_element();
                }
                div()
                    .flex()
                    .gap(px(3.))
                    .children(shown.into_iter().rev().map(|icon| match icon {
                        Some((set, index)) if (index as usize) < set.len() => {
                            draw::icon_picture(set.icons()[index as usize], 16.0).into_any_element()
                        }
                        _ => div().size(px(16.)).into_any_element(),
                    }))
                    .into_any_element()
            }
            _ => format_sample(&cf.format, 130.0, 22.0),
        }
    }
}

const COL_FORMAT: f32 = 150.0;
const COL_APPLIES: f32 = 210.0;
const COL_STOP: f32 = 110.0;
const LINE: f32 = 34.0;

impl Render for Manager {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sheet = self.sheet();
        let active = self.selections[sheet].1;
        let scopes: Vec<SharedString> = [t("Current Selection", "Sélection actuelle"), t("This Worksheet", "Cette feuille de calcul")]
            .into_iter()
            .map(SharedString::from)
            .chain(
                self.names
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| *index != self.current)
                    .map(|(_, name)| format!("{}{name}", t("Sheet: ", "Feuille : ")).into()),
            )
            .collect();
        let has = self.selected_rule().is_some();
        let shown = self.shown();
        let can_move = |up: bool| self.selected.is_some_and(|from| move_target(&shown, from, up).is_some());

        let buttons = div()
            .flex()
            .gap(px(8.))
            .child(
                Button::new("new", t("New Rule...", "Nouvelle règle..."))
                    .on_click(cx.listener(|this, _, window, cx| this.new_rule(window, cx))),
            )
            .child(
                Button::new("edit", t("Edit Rule...", "Modifier la règle..."))
                    .disabled(!has)
                    .on_click(cx.listener(|this, _, window, cx| this.edit_rule(window, cx))),
            )
            .child(
                Button::new("delete", t("Delete Rule", "Supprimer la règle"))
                    .disabled(!has)
                    .on_click(cx.listener(|this, _, _, cx| this.delete_rule(cx))),
            )
            .child(
                Button::new("duplicate", t("Duplicate Rule", "Dupliquer la règle"))
                    .disabled(!has)
                    .on_click(cx.listener(|this, _, _, cx| this.duplicate_rule(cx))),
            )
            .child(
                Button::new("up", t("Move Up", "Monter"))
                    .disabled(!can_move(true))
                    .on_click(cx.listener(|this, _, _, cx| this.move_rule(true, cx))),
            )
            .child(
                Button::new("down", t("Move Down", "Descendre"))
                    .disabled(!can_move(false))
                    .on_click(cx.listener(|this, _, _, cx| this.move_rule(false, cx))),
            );

        let cell = |width: Option<f32>| {
            let cell = div().px(px(8.)).flex().items_center().overflow_hidden();
            match width {
                Some(width) => cell.w(px(width)).flex_none(),
                None => cell.flex_1().min_w(px(0.)),
            }
        };
        let header = div()
            .h(px(26.))
            .flex()
            .bg(c(theme::CHROME_BG))
            .border_b_1()
            .border_color(c(theme::BORDER))
            .font_weight(FontWeight::SEMIBOLD)
            .child(cell(None).child(t("Rule (applied in order shown)", "Règle (appliquée dans l'ordre indiqué)")))
            .child(cell(Some(COL_FORMAT)).child(t("Format", "Format")))
            .child(cell(Some(COL_APPLIES)).child(t("Applies to", "S'applique à")))
            .child(cell(Some(COL_STOP)).child(t("Stop If True", "Interrompre si Vrai")));

        let lines: Vec<AnyElement> = self
            .rows
            .iter()
            .filter_map(|row| {
                let index = row.rule;
                let cf = self.rules[sheet].get(index)?;
                let on = self.selected == Some(index);
                Some(
                    div()
                        .id(("rule", index))
                        .h(px(LINE))
                        .flex()
                        .border_b_1()
                        .border_color(c(theme::BORDER))
                        .when(on, |d| d.bg(c(theme::ACCENT_LIGHT)))
                        .when(!on, |d| d.hover(|s| s.bg(c(theme::HOVER))))
                        .child(cell(None).child(div().truncate().child(words::describe(cf, self.locale, active))))
                        .child(cell(Some(COL_FORMAT)).child(Self::format_cell(cf)))
                        .child(cell(Some(COL_APPLIES)).child(RefEdit::new(&row.applies, &self.modal).width(COL_APPLIES - 16.0)))
                        .child(
                            cell(Some(COL_STOP)).justify_center().child(
                                Checkbox::new(("stop", index), "", cf.stop_if_true)
                                    // Scales, bars and icons never stop anything.
                                    .disabled(cf.rule.is_graded())
                                    .on_change(cx.listener(move |this, stop: &bool, _, cx| {
                                        let sheet = this.sheet();
                                        if let Some(cf) = this.rules[sheet].get_mut(index) {
                                            cf.stop_if_true = *stop;
                                        }
                                        cx.notify();
                                    })),
                            ),
                        )
                        .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, window, cx| {
                            this.selected = Some(index);
                            cx.notify();
                            if ev.click_count() >= 2 {
                                this.edit_rule(window, cx);
                            }
                        }))
                        .into_any_element(),
                )
            })
            .collect();

        dialog(
            &self.modal,
            t(
                "Conditional Formatting Rules Manager",
                "Gestionnaire des règles de mise en forme conditionnelle",
            ),
        )
        .width(820.0)
        .on_ok(cx.listener(Self::ok))
        .child(labeled(
            t("Show formatting rules for:", "Afficher les règles de mise en forme pour :"),
            Select::new("scope", scopes, Some(self.scope))
                .width(240.0)
                .on_change(cx.listener(|this, ix: &usize, _, cx| {
                    this.scope = *ix;
                    this.selected = None;
                    this.rebuild(cx);
                })),
        ))
        .child(buttons)
        .child(
            div()
                .bg(c(0xFFFFFF))
                .border_1()
                .border_color(c(theme::BORDER_STRONG))
                .rounded(px(3.))
                .child(header)
                .child(ScrollArea::new("rules", LINE * 6.0).children(lines)),
        )
        .footer(
            div()
                .flex()
                .gap(px(8.))
                .child(Button::new("dialog-ok", t("OK", "OK")).primary().on_click(cx.listener(Self::ok)))
                .child(Button::new("dialog-close", t("Close", "Fermer")).on_click({
                    let modal = self.modal.clone();
                    move |_, window, cx| modal.close(window, cx)
                }))
                .child(Button::new("dialog-apply", t("Apply", "Appliquer")).on_click(cx.listener(|this, _, _, cx| {
                    this.apply(cx);
                }))),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::Expr;

    fn rule_on(range: &str) -> CondFormat {
        CondFormat::new(vec![Rect::parse(range).unwrap()], Rule::greater_than(Expr::Number(1.0)))
    }

    #[test]
    fn the_selection_shows_the_rules_that_touch_it() {
        let rules = [rule_on("A1:A5"), rule_on("C1:C5"), rule_on("A4:D4")];
        assert_eq!(rules_in(&rules, &[Rect::parse("A1").unwrap()]), [0]);
        assert_eq!(rules_in(&rules, &[Rect::parse("B4:C4").unwrap()]), [1, 2]);
        assert_eq!(rules_in(&rules, &[Rect::parse("F9").unwrap(), Rect::parse("A4").unwrap()]), [0, 2]);
        assert!(rules_in(&rules, &[Rect::parse("F9").unwrap()]).is_empty());
    }

    #[test]
    fn a_rule_moves_past_its_neighbour_on_show() {
        // Rules 1, 4 and 6 of the sheet are on show.
        let shown = [1, 4, 6];
        assert_eq!(move_target(&shown, 4, true), Some(1));
        assert_eq!(move_target(&shown, 4, false), Some(6));
        assert_eq!(move_target(&shown, 1, true), None);
        assert_eq!(move_target(&shown, 6, false), None);
        assert_eq!(move_target(&shown, 3, true), None);
    }

    #[test]
    fn moving_keeps_the_other_rules_in_order() {
        let mut rules: Vec<u32> = (0..7).collect();
        let to = move_target(&[1, 4, 6], 4, true).unwrap();
        let moved = rules.remove(4);
        rules.insert(to, moved);
        assert_eq!(rules, [0, 4, 1, 2, 3, 5, 6]);
    }
}
