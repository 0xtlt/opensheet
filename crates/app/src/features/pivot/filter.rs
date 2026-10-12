//! The drop-down of a field (on "Row Labels", "Column Labels", a field
//! header or a report filter cell) and the dialogs it leads to: Sort,
//! Label Filter, Value Filter and Top 10.

use super::logic::{self, LabelTest, ValueTest};
use super::{PivotUi, actions, just_closed, update};
use crate::app::ExcelApp;
use crate::features::Cx;
use crate::i18n::t;
use crate::shell::Modal;
use crate::text_input::TextInput;
use crate::theme::{self, c};
use crate::widgets::{
    self, Button, CheckItem, CheckList, Checkbox, ListBox, NumberField, RadioGroup, Select, TextField, dialog, icon,
    label, ok_cancel,
};
use gpui::{
    AnyElement, Bounds, Context, Entity, Focusable, MouseButton, Pixels, Point, SharedString, WeakEntity, Window, canvas,
    div, prelude::*, px,
};
use opensheet_engine::pivot::{Area, FilterKind, PivotTable, SortOrder, Top10By};
use opensheet_engine::{Locale, Rect};
use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

/// Width of the drop-down and of its submenus.
const WIDTH: f32 = 264.0;
const SUB_WIDTH: f32 = 232.0;
const ROW: f32 = 26.0;
const SEPARATOR: f32 = 9.0;

/// The submenus of the drop-down.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sub {
    Label,
    Value,
}

/// What the drop-down shows of one field.
struct FieldState {
    field: usize,
    caption: SharedString,
    /// A report filter: one item is picked, unless "Select Multiple Items".
    page: bool,
    /// The items of the field, in the order of the list, and their boxes.
    items: Vec<u32>,
    checks: Vec<CheckItem>,
    /// The item a report filter shows (`None`: all of them).
    page_item: Option<u32>,
    multi: bool,
    filtered: bool,
    sort: SortOrder,
    label_test: Option<LabelTest>,
    /// The field has a value filter; `Some(true)` when it is a Top 10.
    value_filter: Option<Option<ValueTest>>,
    /// How the sort entries are worded: 0 text, 1 numbers, 2 dates.
    kind: u8,
    has_values: bool,
}

impl FieldState {
    fn of(pivot: &PivotTable, field: usize, locale: &Locale) -> FieldState {
        let items = pivot.items_in_order(field);
        let hidden = |item: u32| {
            pivot.fields[field]
                .items
                .iter()
                .any(|it| it.x == item && it.hidden)
        };
        let page = pivot.axis_of(field) == Some(Area::Filters);
        let page_item = pivot.page_fields.iter().find(|pf| pf.field == field).and_then(|pf| pf.item);
        let checks: Vec<CheckItem> = items
            .iter()
            .map(|item| CheckItem::new(pivot.item_text(field, *item, locale), !hidden(*item)))
            .collect();
        let source = pivot.cache.database_field(field);
        let grouped = pivot.cache.fields.get(field).is_some_and(|f| f.group.is_some());
        let kind = if grouped || !pivot.cache.is_numeric(source) {
            0
        } else if pivot.cache.is_date(source) {
            2
        } else {
            1
        };
        FieldState {
            field,
            caption: pivot.field_caption(field).to_string().into(),
            page,
            multi: page && checks.iter().any(|item| !item.checked),
            items,
            checks,
            page_item,
            filtered: pivot.is_filtered(field),
            sort: pivot.fields[field].sort.order,
            label_test: logic::label_filter(pivot, field).map(|filter| LabelTest::of(filter).0),
            value_filter: logic::value_filter(pivot, field).map(|kind| match kind {
                FilterKind::Value { test, .. } => Some(ValueTest::of(test).0),
                _ => None,
            }),
            kind,
            has_values: !pivot.data_fields.is_empty(),
        }
    }
}

/// The drop-down hanging from a cell of a PivotTable.
pub(super) struct DropDown {
    app: WeakEntity<ExcelApp>,
    pivot: u32,
    /// The fields the cell serves ("Row Labels" serves every row field of
    /// its column), with their captions.
    fields: Vec<(usize, SharedString)>,
    state: FieldState,
    search: Entity<TextInput>,
    sub: Option<Sub>,
    /// Where the drop-down hangs from (the corner of its cell).
    anchor: Point<Pixels>,
    /// Where the drop-down was painted, to choose the side of a submenu.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Set when a press lands outside the drop-down (which closes it).
    closed: Rc<Cell<Option<Instant>>>,
}

/// Open the drop-down of the button in a cell, or close it when it is the
/// one that was open.
pub(super) fn toggle_drop_down(app: &mut ExcelApp, id: u32, row: u32, col: u32, window: &mut Window, cx: &mut Cx) {
    let ui = app.ext_mut::<PivotUi>();
    let closed = ui.filter_closed.clone();
    let same = ui.open_filter == Some((row, col));
    ui.open_filter = Some((row, col));
    if just_closed(&closed) && same {
        app.ext_mut::<PivotUi>().open_filter = None;
        return;
    }
    let locale = app.wb.locale;
    let Some((_, pivot)) = app.wb.pivot(id) else {
        return;
    };
    let Some(cell) = pivot.cell(row, col) else {
        return;
    };
    let fields: Vec<(usize, SharedString)> = cell
        .fields
        .iter()
        .map(|field| (*field, pivot.field_caption(*field).to_string().into()))
        .collect();
    let Some(first) = fields.first().map(|(field, _)| *field) else {
        return;
    };
    let state = FieldState::of(pivot, first, locale);
    let weak = cx.weak_entity();
    let anchor = app.cell_anchor(Rect::cell(row, col));
    let view = cx.new(|cx| {
        let search = widgets::input(cx, "");
        search.update(cx, |field, _| field.placeholder = t("Search", "Rechercher").into());
        DropDown {
            app: weak,
            pivot: id,
            fields,
            state,
            search,
            sub: None,
            anchor,
            bounds: Rc::default(),
            closed,
        }
    });
    // The keyboard goes to the search box, as typing narrows the list.
    let focus = view.read(cx).search.read(cx).focus_handle(cx);
    window.focus(&focus);
    app.open_popup_view(anchor, view, cx);
}

impl DropDown {
    /// Close the drop-down, then do something to the application.
    fn act(&self, window: &mut Window, cx: &mut Context<Self>, f: impl FnOnce(&mut ExcelApp, &mut Window, &mut Cx)) {
        self.app
            .update(cx, |app, cx| {
                app.close_popup(window, cx);
                f(app, window, cx);
            })
            .ok();
    }

    /// Indices of the items the search box leaves in the list; `None`
    /// without a search.
    fn shown(&self, cx: &Context<Self>) -> Option<Vec<usize>> {
        let needle = self.search.read(cx).text().trim().to_lowercase();
        (!needle.is_empty()).then(|| {
            self.state
                .checks
                .iter()
                .enumerate()
                .filter(|(_, item)| item.label.to_lowercase().contains(&needle))
                .map(|(i, _)| i)
                .collect()
        })
    }

    fn can_apply(&self, cx: &Context<Self>) -> bool {
        if self.state.page && !self.state.multi {
            return true;
        }
        let checked: Vec<bool> = self.state.checks.iter().map(|item| item.checked).collect();
        !logic::items_to_show(&self.state.items, &checked, self.shown(cx).as_deref()).is_empty()
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        if !self.can_apply(cx) {
            return;
        }
        let (id, field) = (self.pivot, self.state.field);
        let label = t("Filter", "Filtrer");
        if self.state.page && !self.state.multi {
            let item = self.state.page_item;
            self.act(window, cx, move |app, window, cx| {
                update(app, id, label, window, cx, move |p| p.set_page_item(field, item));
            });
            return;
        }
        let checked: Vec<bool> = self.state.checks.iter().map(|item| item.checked).collect();
        let visible = logic::items_to_show(&self.state.items, &checked, self.shown(cx).as_deref());
        let all = visible.len() == self.state.items.len();
        let page = self.state.page;
        self.act(window, cx, move |app, window, cx| {
            update(app, id, label, window, cx, move |p| {
                if page && all {
                    p.set_page_item(field, None);
                } else {
                    p.set_visible_items(field, &visible);
                }
            });
        });
    }

    fn cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.act(window, cx, |_, _, _| {});
    }

    fn sort(&mut self, order: SortOrder, window: &mut Window, cx: &mut Context<Self>) {
        let (id, field) = (self.pivot, self.state.field);
        self.act(window, cx, move |app, window, cx| actions::sort(app, id, field, order, None, window, cx));
    }

    /// Show another of the fields the cell serves.
    fn pick_field(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some((field, _)) = self.fields.get(index).cloned() else {
            return;
        };
        let state = self.app.upgrade().and_then(|app| {
            let app = app.read(cx);
            app.wb.pivot(self.pivot).map(|(_, pivot)| FieldState::of(pivot, field, app.wb.locale))
        });
        if let Some(state) = state {
            self.state = state;
            self.search.update(cx, |input, cx| input.set_text("", cx));
            cx.notify();
        }
    }

    /// One line of the menu part. `sub`: the submenu it opens.
    #[allow(clippy::too_many_arguments)]
    fn menu_row(
        &self,
        id: &'static str,
        icon_name: Option<&'static str>,
        text: SharedString,
        enabled: bool,
        marked: bool,
        sub: Option<Sub>,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        let fg = if enabled { theme::TEXT } else { theme::TEXT_DISABLED };
        let open = sub.is_some() && self.sub == sub;
        div()
            .id(id)
            .h(px(ROW))
            .mx(px(4.))
            .px(px(6.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(4.))
            .text_color(c(fg))
            .when(open, |d| d.bg(c(theme::HOVER)))
            .when(enabled, |d| d.hover(|s| s.bg(c(theme::HOVER))))
            .child(
                div()
                    .size(px(18.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(3.))
                    .when(marked, |d| d.bg(c(theme::TOGGLED)))
                    .when_some(icon_name, |d, name| d.child(icon(name, 14.0, fg))),
            )
            .child(div().flex_1().truncate().child(text))
            .when(sub.is_some(), |d| d.child(icon("chevron-right", 10.0, fg)))
            // Resting on a line opens its submenu and closes the other one.
            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                let wanted = sub.filter(|_| enabled);
                if this.sub != wanted {
                    this.sub = wanted;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, window, cx| {
                if !enabled {
                    return;
                }
                match sub {
                    // A click opens a submenu as resting on its line does.
                    Some(_) => {
                        this.sub = sub;
                        cx.notify();
                    }
                    None => on_click(this, window, cx),
                }
            }))
            .into_any_element()
    }

    /// One line of a submenu. Submenus hang outside the drop-down, so a
    /// press on them closes it before anything else hears of the press:
    /// the line acts on the press itself and through the application, not
    /// through a listener of this view (which may be gone by then).
    fn sub_row(
        &self,
        id: (&'static str, usize),
        text: &'static str,
        enabled: bool,
        ticked: bool,
        on_press: impl Fn(&mut ExcelApp, &mut Window, &mut Cx) + 'static,
    ) -> AnyElement {
        let fg = if enabled { theme::TEXT } else { theme::TEXT_DISABLED };
        let app = self.app.clone();
        div()
            .id(id)
            .h(px(ROW))
            .mx(px(4.))
            .px(px(6.))
            .flex()
            .items_center()
            .gap(px(8.))
            .rounded(px(4.))
            .text_color(c(fg))
            .when(enabled, |d| d.hover(|s| s.bg(c(theme::HOVER))))
            .child(
                div()
                    .w(px(18.))
                    .flex_none()
                    .flex()
                    .justify_center()
                    .when(ticked, |d| d.child(icon("check", 14.0, theme::ACCENT))),
            )
            .child(div().flex_1().truncate().child(text))
            .when(enabled, |d| {
                d.on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    app.update(cx, |app, cx| {
                        app.close_popup(window, cx);
                        on_press(app, window, cx);
                    })
                    .ok();
                    cx.stop_propagation();
                })
            })
            .into_any_element()
    }

    fn separator() -> AnyElement {
        div()
            .h(px(1.))
            .my(px((SEPARATOR - 1.0) / 2.0))
            .mx(px(8.))
            .bg(c(theme::BORDER))
            .into_any_element()
    }

    /// The submenu that is open, placed beside the line it belongs to.
    fn submenu(&self, sub: Sub, top: f32, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let (id, field) = (self.pivot, self.state.field);
        let mut rows: Vec<AnyElement> = Vec::new();
        match sub {
            Sub::Label => {
                rows.push(self.sub_row(
                    ("label-clear", 0),
                    t("Clear Filter", "Effacer le filtre"),
                    self.state.label_test.is_some(),
                    false,
                    move |app, window, cx| {
                        update(app, id, t("Clear Filter", "Effacer le filtre"), window, cx, move |p| {
                            p.set_label_filter(field, None)
                        });
                    },
                ));
                for (g, tests) in LabelTest::MENU.iter().enumerate() {
                    rows.push(Self::separator());
                    for (k, test) in tests.iter().enumerate() {
                        let test = *test;
                        rows.push(self.sub_row(
                            ("label-test", g * 10 + k),
                            test.menu_label(),
                            true,
                            self.state.label_test == Some(test),
                            move |app, window, cx| open_label_filter(app, id, field, Some(test), window, cx),
                        ));
                    }
                }
            }
            Sub::Value => {
                rows.push(self.sub_row(
                    ("value-clear", 0),
                    t("Clear Filter", "Effacer le filtre"),
                    self.state.value_filter.is_some(),
                    false,
                    move |app, window, cx| {
                        update(app, id, t("Clear Filter", "Effacer le filtre"), window, cx, move |p| {
                            p.set_value_filter(field, None)
                        });
                    },
                ));
                rows.push(Self::separator());
                for (k, test) in ValueTest::ALL.iter().enumerate() {
                    let test = *test;
                    if matches!(test, ValueTest::GreaterThan | ValueTest::Between) {
                        rows.push(Self::separator());
                    }
                    rows.push(self.sub_row(
                        ("value-test", k),
                        test.menu_label(),
                        true,
                        self.state.value_filter == Some(Some(test)),
                        move |app, window, cx| open_value_filter(app, id, field, Some(test), window, cx),
                    ));
                }
                rows.push(Self::separator());
                rows.push(self.sub_row(
                    ("value-top", 0),
                    t("Top 10...", "10 premiers..."),
                    true,
                    self.state.value_filter == Some(None),
                    move |app, window, cx| open_top10(app, id, field, window, cx),
                ));
            }
        }
        // A press in the submenu that runs no entry (a line, a greyed
        // entry) has closed the drop-down like any press outside it: put
        // it back as it was.
        let (app, view, anchor) = (self.app.clone(), cx.entity(), self.anchor);
        let keep_open = move |_: &gpui::MouseDownEvent, _: &mut Window, cx: &mut gpui::App| {
            app.update(cx, |app, cx| {
                if app.popup.is_none() {
                    app.open_popup_view(anchor, view.clone(), cx);
                }
            })
            .ok();
        };
        // On the right when there is room, else on the left.
        let bounds = self.bounds.get();
        let room = f32::from(window.viewport_size().width) - f32::from(bounds.right());
        let panel = div()
            .id("submenu")
            .absolute()
            .top(px((top - 5.0).max(0.0)))
            .w(px(SUB_WIDTH))
            .occlude()
            .py(px(4.))
            .bg(c(0xFFFFFF))
            .border_1()
            .border_color(c(theme::BORDER_STRONG))
            .rounded(px(6.))
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, keep_open)
            .children(rows);
        if room >= SUB_WIDTH + 4.0 || f32::from(bounds.left()) < SUB_WIDTH {
            panel.left(px(WIDTH - 2.0)).into_any_element()
        } else {
            panel.left(px(-SUB_WIDTH + 2.0)).into_any_element()
        }
    }

    /// The list of a report filter that picks one item: "(All)", then the
    /// items the search leaves.
    fn page_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let shown = self.shown(cx);
        let rows: Vec<Option<usize>> = std::iter::once(None)
            .filter(|_| shown.is_none())
            .chain(
                (0..self.state.items.len())
                    .filter(|i| shown.as_ref().is_none_or(|s| s.contains(i)))
                    .map(Some),
            )
            .collect();
        let labels: Vec<SharedString> = rows
            .iter()
            .map(|row| match row {
                None => t("(All)", "(Tous)").into(),
                Some(i) => self.state.checks[*i].label.clone(),
            })
            .collect();
        let current = self.state.page_item.and_then(|item| self.state.items.iter().position(|x| *x == item));
        let selected = rows.iter().position(|row| *row == current);
        let pick = Rc::new(rows);
        let activate = pick.clone();
        ListBox::new("page-items", labels, selected)
            .height(widgets::ROW_H * 9.0)
            .on_change(cx.listener(move |this, ix: &usize, _, cx| {
                this.state.page_item = pick.get(*ix).copied().flatten().map(|i| this.state.items[i]);
                cx.notify();
            }))
            .on_activate(cx.listener(move |this, ix: &usize, window, cx| {
                this.state.page_item = activate.get(*ix).copied().flatten().map(|i| this.state.items[i]);
                this.ok(&(), window, cx);
            }))
            .into_any_element()
    }
}

impl Render for DropDown {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = &self.state;
        let mut top: Vec<AnyElement> = Vec::new();
        // Offset of the next line from the top of the drop-down, to hang
        // the submenus at the right height.
        let mut y = 4.0;
        let mut sub_top = [0.0f32; 2];

        if self.fields.len() > 1 {
            let captions: Vec<SharedString> = self.fields.iter().map(|(_, caption)| caption.clone()).collect();
            let current = self.fields.iter().position(|(field, _)| *field == state.field);
            top.push(
                div()
                    .h(px(ROW + 22.0))
                    .px(px(10.))
                    .flex()
                    .flex_col()
                    .justify_center()
                    .gap(px(3.))
                    .child(label(t("Select field:", "Sélectionner le champ :")))
                    .child(
                        Select::new("field", captions, current)
                            .width(WIDTH - 20.0)
                            .on_change(cx.listener(|this, ix: &usize, _, cx| this.pick_field(*ix, cx))),
                    )
                    .into_any_element(),
            );
            y += ROW + 22.0;
        }

        if !state.page {
            let (ascending, descending) = match state.kind {
                1 => (
                    t("Sort Smallest to Largest", "Trier du plus petit au plus grand"),
                    t("Sort Largest to Smallest", "Trier du plus grand au plus petit"),
                ),
                2 => (
                    t("Sort Oldest to Newest", "Trier du plus ancien au plus récent"),
                    t("Sort Newest to Oldest", "Trier du plus récent au plus ancien"),
                ),
                _ => (t("Sort A to Z", "Trier de A à Z"), t("Sort Z to A", "Trier de Z à A")),
            };
            top.push(self.menu_row(
                "sort-asc",
                Some("sort-asc"),
                ascending.into(),
                true,
                state.sort == SortOrder::Ascending,
                None,
                cx,
                |this, window, cx| this.sort(SortOrder::Ascending, window, cx),
            ));
            top.push(self.menu_row(
                "sort-desc",
                Some("sort-desc"),
                descending.into(),
                true,
                state.sort == SortOrder::Descending,
                None,
                cx,
                |this, window, cx| this.sort(SortOrder::Descending, window, cx),
            ));
            top.push(self.menu_row(
                "sort-more",
                None,
                t("More Sort Options...", "Autres options de tri...").into(),
                true,
                false,
                None,
                cx,
                |this, window, cx| {
                    let (id, field) = (this.pivot, this.state.field);
                    this.act(window, cx, move |app, window, cx| open_sort(app, id, field, window, cx))
                },
            ));
            top.push(Self::separator());
            y += 3.0 * ROW + SEPARATOR;
        }

        let clear = t("Clear Filter From \"{}\"", "Effacer le filtre de « {} »").replace("{}", &state.caption);
        top.push(self.menu_row(
            "clear",
            Some("filter"),
            clear.into(),
            state.filtered,
            false,
            None,
            cx,
            |this, window, cx| {
                let (id, field) = (this.pivot, this.state.field);
                this.act(window, cx, move |app, window, cx| {
                    actions::clear_field_filters(app, id, field, window, cx)
                })
            },
        ));
        y += ROW;
        if !state.page {
            sub_top[0] = y;
            top.push(self.menu_row(
                "label-filters",
                None,
                t("Label Filters", "Filtres s'appliquant aux étiquettes").into(),
                true,
                state.label_test.is_some(),
                Some(Sub::Label),
                cx,
                |_, _, _| {},
            ));
            y += ROW;
            sub_top[1] = y;
            top.push(self.menu_row(
                "value-filters",
                None,
                t("Value Filters", "Filtres s'appliquant aux valeurs").into(),
                state.has_values,
                state.value_filter.is_some(),
                Some(Sub::Value),
                cx,
                |_, _, _| {},
            ));
        }

        let list = if state.page && !state.multi {
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(div().h(px(widgets::CONTROL_H)).flex().child(
                    TextField::new(&self.search).trailing(div().pr(px(4.)).child(icon("search", 12.0, theme::TEXT_MUTED))),
                ))
                .child(div().flex().child(self.page_list(cx)))
                .into_any_element()
        } else {
            div()
                .flex()
                .child(
                    CheckList::new("items", state.checks.clone())
                        .search(&self.search)
                        .height(widgets::ROW_H * 9.0)
                        .on_toggle(cx.listener(|this, (ix, on): &(usize, bool), _, cx| {
                            this.state.checks[*ix].checked = *on;
                            cx.notify();
                        }))
                        .on_toggle_all(cx.listener(|this, (shown, on): &(Vec<usize>, bool), _, cx| {
                            for ix in shown {
                                this.state.checks[*ix].checked = *on;
                            }
                            cx.notify();
                        })),
                )
                .into_any_element()
        };

        let multiple = state.page.then(|| {
            Checkbox::new(
                "multiple",
                t("Select Multiple Items", "Sélectionner plusieurs éléments"),
                state.multi,
            )
            .on_change(cx.listener(|this, on: &bool, _, cx| {
                this.state.multi = *on;
                if *on {
                    // The boxes start from the item that was picked.
                    let picked = this.state.page_item;
                    let items = this.state.items.clone();
                    for (item, check) in items.iter().zip(&mut this.state.checks) {
                        check.checked = picked.is_none_or(|x| x == *item);
                    }
                } else {
                    this.state.page_item = None;
                }
                cx.notify();
            }))
        });

        let submenu = self.sub.map(|sub| {
            let top = sub_top[if sub == Sub::Label { 0 } else { 1 }];
            self.submenu(sub, top, window, cx)
        });
        let can_apply = self.can_apply(cx);
        let bounds = self.bounds.clone();
        let closed = self.closed.clone();
        div()
            .id("pivot-drop-down")
            .relative()
            .w(px(WIDTH))
            .py(px(4.))
            .text_size(px(12.5))
            // A press outside closes the drop-down; when it lands on the
            // button it hangs from, that button must not open it again.
            .on_mouse_down_out(move |_, _, _| closed.set(Some(Instant::now())))
            .on_action(cx.listener(|this, _: &widgets::Accept, window, cx| this.ok(&(), window, cx)))
            .on_action(cx.listener(|this, _: &crate::text_input::Enter, window, cx| this.ok(&(), window, cx)))
            .child(
                canvas(move |painted, _, _| bounds.set(painted), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .children(top)
            .child(div().px(px(10.)).pt(px(6.)).child(list))
            .children(multiple.map(|check| div().px(px(10.)).pt(px(6.)).child(check)))
            .child(
                div()
                    .px(px(10.))
                    .pt(px(8.))
                    .pb(px(4.))
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        Button::new("ok", t("OK", "OK"))
                            .primary()
                            .disabled(!can_apply)
                            .on_click(cx.listener(Self::ok)),
                    )
                    .child(
                        Button::new("cancel", t("Cancel", "Annuler"))
                            .on_click(cx.listener(|this, _, window, cx| this.cancel(window, cx))),
                    ),
            )
            .children(submenu)
    }
}

// ----------------------------------------------------------------- the dialogs

fn field_title(pattern: &'static str, caption: &str) -> SharedString {
    pattern.replace("{}", caption).into()
}

/// "More Sort Options": manual order, or by label or by a value field.
pub(super) fn open_sort(app: &mut ExcelApp, id: u32, field: usize, window: &mut Window, cx: &mut Cx) {
    let Some((_, pivot)) = app.wb.pivot(id) else {
        return;
    };
    let caption = pivot.field_caption(field).to_string();
    let sort = pivot.fields[field].sort;
    let mut keys: Vec<SharedString> = vec![caption.clone().into()];
    keys.extend(pivot.data_fields.iter().map(|d| SharedString::from(d.name.clone())));
    app.open_modal(window, cx, move |modal, _, _| SortDialog {
        modal,
        pivot: id,
        field,
        caption,
        order: sort.order,
        key: sort.by_value.map_or(0, |d| d + 1),
        keys,
    });
}

struct SortDialog {
    modal: Modal,
    pivot: u32,
    field: usize,
    caption: String,
    order: SortOrder,
    /// 0: the labels of the field; then the value fields.
    key: usize,
    keys: Vec<SharedString>,
}

impl SortDialog {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let (id, field, order) = (self.pivot, self.field, self.order);
        let by_value = (order != SortOrder::Manual && self.key > 0).then(|| self.key - 1);
        self.modal.close(window, cx);
        self.modal
            .update_app(cx, |app, cx| actions::sort(app, id, field, order, by_value, window, cx));
    }

    fn option(&self, id: &'static str, text: &'static str, order: SortOrder, cx: &mut Context<Self>) -> RadioGroup {
        RadioGroup::new(id, vec![text.into()], (self.order == order).then_some(0)).on_change(cx.listener(
            move |this, _: &usize, _, cx| {
                this.order = order;
                cx.notify();
            },
        ))
    }

    fn keys(&self, id: &'static str, order: SortOrder, cx: &mut Context<Self>) -> AnyElement {
        div()
            .pl(px(22.))
            .child(
                Select::new(id, self.keys.clone(), Some(self.key))
                    .width(260.0)
                    .disabled(self.order != order)
                    .on_change(cx.listener(|this, ix: &usize, _, cx| {
                        this.key = *ix;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }
}

impl Render for SortDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let by_value = (self.key > 0).then(|| self.keys[self.key].to_string());
        let summary = logic::sort_summary(&self.caption, self.order, by_value.as_deref());
        dialog(&self.modal, field_title(t("Sort ({})", "Trier ({})"), &self.caption))
            .width(340.0)
            .on_ok(cx.listener(Self::ok))
            .child(
                widgets::GroupBox::new(t("Sort options", "Options de tri"))
                    .child(self.option(
                        "manual",
                        t(
                            "Manual (you can drag items to rearrange them)",
                            "Manuel (vous pouvez faire glisser des éléments pour les réorganiser)",
                        ),
                        SortOrder::Manual,
                        cx,
                    ))
                    .child(self.option(
                        "ascending",
                        t("Ascending (A to Z) by:", "Croissant (de A à Z) par :"),
                        SortOrder::Ascending,
                        cx,
                    ))
                    .child(self.keys("ascending-key", SortOrder::Ascending, cx))
                    .child(self.option(
                        "descending",
                        t("Descending (Z to A) by:", "Décroissant (de Z à A) par :"),
                        SortOrder::Descending,
                        cx,
                    ))
                    .child(self.keys("descending-key", SortOrder::Descending, cx)),
            )
            .child(
                widgets::GroupBox::new(t("Summary", "Synthèse"))
                    .child(div().min_h(px(34.)).line_height(px(17.)).child(summary)),
            )
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

/// The Label Filter dialog, opened on `test` or on the filter the field
/// has.
pub(super) fn open_label_filter(
    app: &mut ExcelApp,
    id: u32,
    field: usize,
    test: Option<LabelTest>,
    window: &mut Window,
    cx: &mut Cx,
) {
    let Some((_, pivot)) = app.wb.pivot(id) else {
        return;
    };
    let caption = pivot.field_caption(field).to_string();
    let current = logic::label_filter(pivot, field).map(LabelTest::of);
    let (test, a, b) = match (test, current) {
        (Some(test), Some((had, a, b))) if had == test => (test, a, b),
        (Some(test), _) => (test, String::new(), String::new()),
        (None, Some(current)) => current,
        (None, None) => (LabelTest::Equals, String::new(), String::new()),
    };
    app.open_modal(window, cx, move |modal, _, cx| LabelFilterDialog {
        modal,
        pivot: id,
        field,
        caption,
        test,
        first: widgets::input(cx, a),
        second: widgets::input(cx, b),
    });
}

struct LabelFilterDialog {
    modal: Modal,
    pivot: u32,
    field: usize,
    caption: String,
    test: LabelTest,
    first: Entity<TextInput>,
    second: Entity<TextInput>,
}

impl LabelFilterDialog {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let (id, field) = (self.pivot, self.field);
        let filter = self.test.build(self.first.read(cx).text(), self.second.read(cx).text());
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            update(app, id, t("Label Filter", "Filtre s'appliquant aux étiquettes"), window, cx, move |p| {
                p.set_label_filter(field, Some(filter.clone()))
            })
        });
    }
}

impl Render for LabelFilterDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tests: Vec<SharedString> = LabelTest::ALL.iter().map(|test| test.label().into()).collect();
        let selected = LabelTest::ALL.iter().position(|test| *test == self.test);
        dialog(
            &self.modal,
            field_title(t("Label Filter ({})", "Filtre s'appliquant aux étiquettes ({})"), &self.caption),
        )
        .width(if self.test.two() { 560.0 } else { 470.0 })
        .on_ok(cx.listener(Self::ok))
        .child(label(t(
            "Show items for which the label",
            "Afficher les éléments pour lesquels l’étiquette",
        )))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    Select::new("test", tests, selected)
                        .width(200.0)
                        .on_change(cx.listener(|this, ix: &usize, _, cx| {
                            this.test = LabelTest::ALL[*ix];
                            cx.notify();
                        })),
                )
                .child(TextField::new(&self.first))
                .when(self.test.two(), |d| {
                    d.child(label(t("and", "et"))).child(TextField::new(&self.second))
                }),
        )
        .child(
            div()
                .text_color(c(theme::TEXT_MUTED))
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

/// The value fields of a PivotTable, for the lists of the dialogs.
fn value_names(pivot: &PivotTable) -> Vec<SharedString> {
    pivot.data_fields.iter().map(|d| SharedString::from(d.name.clone())).collect()
}

/// The Value Filter dialog, opened on `test` or on the filter the field
/// has.
pub(super) fn open_value_filter(
    app: &mut ExcelApp,
    id: u32,
    field: usize,
    test: Option<ValueTest>,
    window: &mut Window,
    cx: &mut Cx,
) {
    let locale = app.wb.locale;
    let Some((_, pivot)) = app.wb.pivot(id) else {
        return;
    };
    if pivot.data_fields.is_empty() {
        return;
    }
    let caption = pivot.field_caption(field).to_string();
    let values = value_names(pivot);
    let current = match logic::value_filter(pivot, field) {
        Some(FilterKind::Value { data_field, test }) => Some((*data_field, ValueTest::of(test))),
        _ => None,
    };
    let (data, test, a, b) = match (test, current) {
        (Some(test), Some((data, (had, a, b)))) if had == test => (data, test, Some(a), b),
        (Some(test), current) => (current.map_or(0, |(data, _)| data), test, None, None),
        (None, Some((data, (test, a, b)))) => (data, test, Some(a), b),
        (None, None) => (0, ValueTest::Equals, None, None),
    };
    let text = move |n: Option<f64>| n.map(|n| logic::number_text(n, locale)).unwrap_or_default();
    app.open_modal(window, cx, move |modal, _, cx| ValueFilterDialog {
        modal,
        pivot: id,
        field,
        caption,
        locale,
        values,
        data,
        test,
        first: widgets::input(cx, text(a)),
        second: widgets::input(cx, text(b)),
    });
}

struct ValueFilterDialog {
    modal: Modal,
    pivot: u32,
    field: usize,
    caption: String,
    locale: &'static Locale,
    values: Vec<SharedString>,
    data: usize,
    test: ValueTest,
    first: Entity<TextInput>,
    second: Entity<TextInput>,
}

impl ValueFilterDialog {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let (id, field, data, test) = (self.pivot, self.field, self.data, self.test);
        let a = logic::parse_number(self.first.read(cx).text(), self.locale);
        let b = logic::parse_number(self.second.read(cx).text(), self.locale);
        let (Some(a), Some(b)) = (a, b.or(Some(0.0).filter(|_| !test.two()))) else {
            self.modal.update_app(cx, |app, cx| {
                actions::warn(app, t("Value must be a number.", "La valeur doit être un nombre."), cx)
            });
            return;
        };
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            update(app, id, t("Value Filter", "Filtre s'appliquant aux valeurs"), window, cx, move |p| {
                p.set_value_filter(
                    field,
                    Some(FilterKind::Value {
                        data_field: data,
                        test: test.build(a, b),
                    }),
                )
            })
        });
    }
}

impl Render for ValueFilterDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tests: Vec<SharedString> = ValueTest::ALL.iter().map(|test| test.label().into()).collect();
        let selected = ValueTest::ALL.iter().position(|test| *test == self.test);
        dialog(
            &self.modal,
            field_title(t("Value Filter ({})", "Filtre s'appliquant aux valeurs ({})"), &self.caption),
        )
        .width(if self.test.two() { 660.0 } else { 570.0 })
        .on_ok(cx.listener(Self::ok))
        .child(label(t("Show items for which", "Afficher les éléments pour lesquels")))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    Select::new("value", self.values.clone(), Some(self.data))
                        .width(170.0)
                        .on_change(cx.listener(|this, ix: &usize, _, cx| {
                            this.data = *ix;
                            cx.notify();
                        })),
                )
                .child(
                    Select::new("test", tests, selected)
                        .width(200.0)
                        .on_change(cx.listener(|this, ix: &usize, _, cx| {
                            this.test = ValueTest::ALL[*ix];
                            cx.notify();
                        })),
                )
                .child(TextField::new(&self.first))
                .when(self.test.two(), |d| {
                    d.child(label(t("and", "et"))).child(TextField::new(&self.second))
                }),
        )
        .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

/// The Top 10 Filter dialog.
pub(super) fn open_top10(app: &mut ExcelApp, id: u32, field: usize, window: &mut Window, cx: &mut Cx) {
    let Some((_, pivot)) = app.wb.pivot(id) else {
        return;
    };
    if pivot.data_fields.is_empty() {
        return;
    }
    let caption = pivot.field_caption(field).to_string();
    let values = value_names(pivot);
    let (data, top, count, by) = match logic::value_filter(pivot, field) {
        Some(FilterKind::Top10 {
            data_field,
            top,
            count,
            by,
        }) => (*data_field, *top, *count, *by),
        _ => (0, true, 10.0, Top10By::Items),
    };
    app.open_modal(window, cx, move |modal, _, cx| Top10Dialog {
        modal,
        pivot: id,
        field,
        caption,
        values,
        data,
        top,
        count: widgets::number_input(cx, Some(count), 1.0, 1e12, 1.0),
        by,
    });
}

struct Top10Dialog {
    modal: Modal,
    pivot: u32,
    field: usize,
    caption: String,
    values: Vec<SharedString>,
    data: usize,
    top: bool,
    count: Entity<TextInput>,
    by: Top10By,
}

const TOP10_BY: [Top10By; 3] = [Top10By::Items, Top10By::Percent, Top10By::Sum];

impl Top10Dialog {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let (id, field, data, top, by) = (self.pivot, self.field, self.data, self.top, self.by);
        let Some(count) = self.count.read(cx).number().filter(|n| *n > 0.0) else {
            self.modal.update_app(cx, |app, cx| {
                actions::warn(app, t("Value must be a number.", "La valeur doit être un nombre."), cx)
            });
            return;
        };
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            update(app, id, t("Top 10 Filter", "Filtre des 10 premiers"), window, cx, move |p| {
                p.set_value_filter(
                    field,
                    Some(FilterKind::Top10 {
                        data_field: data,
                        top,
                        count,
                        by,
                    }),
                )
            })
        });
    }
}

impl Render for Top10Dialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let ends: Vec<SharedString> = vec![t("Top", "Haut").into(), t("Bottom", "Bas").into()];
        let kinds: Vec<SharedString> = TOP10_BY.iter().map(|by| logic::top10_by_label(*by).into()).collect();
        dialog(
            &self.modal,
            field_title(t("Top 10 Filter ({})", "Filtre des 10 premiers ({})"), &self.caption),
        )
        .width(560.0)
        .on_ok(cx.listener(Self::ok))
        .child(label(t("Show", "Afficher")))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    Select::new("end", ends, Some(if self.top { 0 } else { 1 }))
                        .width(90.0)
                        .on_change(cx.listener(|this, ix: &usize, _, cx| {
                            this.top = *ix == 0;
                            cx.notify();
                        })),
                )
                .child(NumberField::new("count", &self.count).width(80.0))
                .child(
                    Select::new("kind", kinds, TOP10_BY.iter().position(|by| *by == self.by))
                        .width(120.0)
                        .on_change(cx.listener(|this, ix: &usize, _, cx| {
                            this.by = TOP10_BY[*ix];
                            cx.notify();
                        })),
                )
                .child(label(t("by", "par")))
                .child(
                    Select::new("value", self.values.clone(), Some(self.data))
                        .width(170.0)
                        .on_change(cx.listener(|this, ix: &usize, _, cx| {
                            this.data = *ix;
                            cx.notify();
                        })),
                ),
        )
        .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}
