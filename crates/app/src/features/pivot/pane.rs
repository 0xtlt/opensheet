//! The PivotTable Fields pane: the check-list of the fields, the four
//! areas, dragging between them, the menu of an area entry, and "Defer
//! Layout Update".

use super::logic::{self, AreaEntry, DragSource, PaneOp, Target};
use super::{PivotUi, active_pivot, entry, menus, open_menu_at, perform};
use crate::app::{ExcelApp, MenuEntry};
use crate::features::Cx;
use crate::i18n::t;
use crate::text_input::TextInput;
use crate::theme::{self, c};
use crate::widgets::{self, Button, Checkbox, TextField, Tri, check_mark, icon, label};
use gpui::{
    AnyElement, Context, DragMoveEvent, Entity, FontWeight, MouseButton, ScrollHandle,
    SharedString, WeakEntity, Window, div, prelude::*, px,
};
use opensheet_engine::pivot::{Area, PivotTable};

const AREAS: [Area; 4] = [Area::Filters, Area::Columns, Area::Rows, Area::Values];
/// Height of an area box and of one entry in it.
const AREA_H: f32 = 92.0;
const ENTRY_H: f32 = 24.0;

fn area_index(area: Area) -> usize {
    AREAS.iter().position(|a| *a == area).unwrap_or(0)
}

/// What is dragged in the pane.
#[derive(Clone)]
struct Dragged {
    source: DragSource,
    label: SharedString,
}

/// The name that follows the pointer during a drag.
struct DragChip(SharedString);

impl Render for DragChip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .h(px(ENTRY_H))
            .px(px(8.))
            .flex()
            .items_center()
            .bg(c(0xFFFFFF))
            .border_1()
            .border_color(c(theme::ACCENT))
            .rounded(px(2.))
            .shadow_md()
            .text_size(px(12.))
            .text_color(c(theme::TEXT))
            .child(self.0.clone())
    }
}

/// A field of the check-list.
struct FieldRow {
    field: usize,
    caption: SharedString,
    used: bool,
    filtered: bool,
}

/// What the pane shows of a PivotTable: gathered before drawing, so that
/// the listeners own what they need.
struct Shown {
    id: u32,
    fields: Vec<FieldRow>,
    areas: [Vec<AreaEntry>; 4],
    defer: bool,
    /// Changes wait for "Update".
    pending: bool,
}

impl Shown {
    fn of(app: &ExcelApp) -> Option<Shown> {
        let ui = app.ext::<PivotUi>();
        let active = active_pivot(app)?;
        // While layout updates are deferred, the pane shows the definition
        // being prepared.
        let pending = ui
            .and_then(|ui| ui.pending.as_ref())
            .filter(|p| p.id == active.id);
        let pivot = pending.unwrap_or(active);
        let locale = app.wb.locale;
        Some(Shown {
            id: pivot.id,
            fields: (0..pivot.fields.len())
                .map(|field| FieldRow {
                    field,
                    caption: pivot.field_caption(field).to_string().into(),
                    used: pivot.is_used(field),
                    filtered: pivot.is_filtered(field),
                })
                .collect(),
            areas: AREAS.map(|area| logic::area_entries(pivot, area, locale)),
            defer: ui.is_some_and(|ui| ui.defer),
            pending: pending.is_some(),
        })
    }
}

/// The definition the pane works on: the one being prepared while layout
/// updates are deferred, else the PivotTable itself.
fn shown_pivot(app: &ExcelApp, id: u32) -> Option<&PivotTable> {
    app.ext::<PivotUi>()
        .and_then(|ui| ui.pending.as_ref())
        .filter(|p| p.id == id)
        .or_else(|| app.wb.pivot(id).map(|(_, p)| p))
}

/// Make a change of the pane: at once, or on the definition put aside
/// while "Defer Layout Update" is ticked.
fn apply(app: &mut ExcelApp, id: u32, op: PaneOp, window: &mut Window, cx: &mut Cx) {
    let locale = app.wb.locale;
    if app.ext::<PivotUi>().is_some_and(|ui| ui.defer) {
        let Some(mut pending) = shown_pivot(app, id).cloned() else {
            return;
        };
        logic::apply_op(&mut pending, op, locale);
        app.ext_mut::<PivotUi>().pending = Some(pending);
        cx.notify();
        return;
    }
    perform(
        app,
        t("PivotTable Fields", "Champs de tableau croisé dynamique"),
        window,
        cx,
        move |app, overwrite| logic::run_op(&mut app.wb, id, op, overwrite),
    );
}

/// The menu of the entry at `index` of an area: moving it inside the area
/// and to another one, removing it, its settings. Shared by the pane and
/// by the value field buttons of a PivotChart. `pending`: changes wait
/// for "Update", and the settings with them.
pub(super) fn entry_menu_entries(
    app: &ExcelApp,
    id: u32,
    area: Area,
    index: usize,
    pending: bool,
) -> Vec<MenuEntry> {
    let locale = app.wb.locale;
    let Some(pivot) = shown_pivot(app, id) else {
        return Vec::new();
    };
    let moves = logic::entry_moves(pivot, area, index, locale);
    let count = |area: Area| logic::area_entries(pivot, area, locale).len();
    let last = count(area).saturating_sub(1);
    let go = |label: &'static str, to: Area, position: usize, possible: bool| {
        let op = PaneOp::Move {
            from: area,
            index,
            to,
            position,
        };
        entry(label, move |app, window, cx| apply(app, id, op, window, cx)).disabled(!possible)
    };
    let target = match area {
        Area::Values => Some(Target::Data(index)),
        _ => logic::area_entries(pivot, area, locale)
            .get(index)
            .and_then(|e| e.field)
            .map(Target::Field),
    };
    let settings = if moves.value {
        t(
            "Value Field Settings...",
            "Paramètres des champs de valeurs...",
        )
    } else {
        t("Field Settings...", "Paramètres de champ...")
    };
    vec![
        go(
            t("Move Up", "Monter"),
            area,
            index.saturating_sub(1),
            moves.up,
        ),
        go(t("Move Down", "Descendre"), area, index + 1, moves.down),
        go(
            t("Move to Beginning", "Déplacer au début"),
            area,
            0,
            moves.up,
        ),
        go(
            t("Move to End", "Déplacer à la fin"),
            area,
            last,
            moves.down,
        ),
        MenuEntry::separator(),
        go(
            t(
                "Move to Report Filter",
                "Déplacer dans la zone Filtre du rapport",
            ),
            Area::Filters,
            count(Area::Filters),
            moves.to_filters,
        )
        .icon("filter"),
        go(
            t(
                "Move to Row Labels",
                "Déplacer dans la zone Étiquettes de lignes",
            ),
            Area::Rows,
            count(Area::Rows),
            moves.to_rows,
        )
        .icon("pivot-rows"),
        go(
            t(
                "Move to Column Labels",
                "Déplacer dans la zone Étiquettes de colonnes",
            ),
            Area::Columns,
            count(Area::Columns),
            moves.to_columns,
        )
        .icon("pivot-columns"),
        go(
            t("Move to Values", "Déplacer dans la zone Valeurs"),
            Area::Values,
            count(Area::Values),
            moves.to_values,
        )
        .icon("pivot-sigma"),
        MenuEntry::separator(),
        entry(
            t("Remove Field", "Supprimer le champ"),
            move |app, window, cx| apply(app, id, PaneOp::Remove { area, index }, window, cx),
        )
        .icon("close"),
        MenuEntry::separator(),
        // The settings are those of the report as written: not
        // while changes wait for "Update".
        entry(settings, move |app, window, cx| {
            if let Some(target) = target {
                menus::open_target_settings(app, id, target, window, cx);
            }
        })
        .icon("format-cells")
        .disabled(moves.sigma || pending || target.is_none()),
    ]
}

/// "Update": write the report of the definition that was put aside.
fn update_deferred(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some(pending) = app.ext::<PivotUi>().and_then(|ui| ui.pending.clone()) else {
        return;
    };
    let id = pending.id;
    perform(
        app,
        t("PivotTable Fields", "Champs de tableau croisé dynamique"),
        window,
        cx,
        move |app, overwrite| {
            app.wb
                .pivot_update(id, overwrite, |p| logic::adopt_layout(p, &pending))
        },
    );
}

pub(super) struct FieldsPane {
    app: WeakEntity<ExcelApp>,
    search: Entity<TextInput>,
    /// Where the field being dragged would land: the area and the entry it
    /// would go before.
    drop_at: Option<(Area, usize)>,
    scroll: [ScrollHandle; 4],
}

impl FieldsPane {
    pub(super) fn new(app: WeakEntity<ExcelApp>, cx: &mut Context<Self>) -> Self {
        let search = widgets::input(cx, "");
        search.update(cx, |field, _| {
            field.placeholder = t("Search", "Rechercher").into()
        });
        FieldsPane {
            app,
            search,
            drop_at: None,
            scroll: std::array::from_fn(|_| ScrollHandle::new()),
        }
    }

    fn run(&self, id: u32, op: PaneOp, window: &mut Window, cx: &mut Context<Self>) {
        self.app
            .update(cx, |app, cx| apply(app, id, op, window, cx))
            .ok();
    }

    /// Something dragged was dropped on an area (`None`: beside them).
    fn dropped(
        &mut self,
        id: u32,
        source: DragSource,
        area: Option<Area>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let slot = self.drop_at.take();
        self.app
            .update(cx, |app, cx| {
                let locale = app.wb.locale;
                let Some(pivot) = shown_pivot(app, id) else {
                    return;
                };
                let target = area.map(|area| {
                    let end = logic::area_entries(pivot, area, locale).len();
                    (
                        area,
                        slot.filter(|(a, _)| *a == area)
                            .map_or(end, |(_, at)| at.min(end)),
                    )
                });
                if let Some(op) = logic::drop_op(pivot, source, target, locale) {
                    apply(app, id, op, window, cx);
                }
            })
            .ok();
        cx.notify();
    }

    /// The menu of an entry of an area.
    fn entry_menu(
        &self,
        shown: &Shown,
        area: Area,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (id, pending) = (shown.id, shown.pending);
        let position = window.mouse_position();
        self.app
            .update(cx, |app, cx| {
                let entries = entry_menu_entries(app, id, area, index, pending);
                open_menu_at(app, position, entries, cx);
            })
            .ok();
    }

    /// The menu of a field of the list (right click).
    fn field_menu(&self, id: u32, field: usize, window: &mut Window, cx: &mut Context<Self>) {
        let position = window.mouse_position();
        let add = move |label: &'static str, area: Area| {
            let op = PaneOp::Add {
                field,
                area,
                position: None,
            };
            entry(label, move |app, window, cx| apply(app, id, op, window, cx))
        };
        let entries = vec![
            add(
                t(
                    "Add to Report Filter",
                    "Ajouter à la zone Filtre du rapport",
                ),
                Area::Filters,
            )
            .icon("filter"),
            add(
                t("Add to Row Labels", "Ajouter aux étiquettes de lignes"),
                Area::Rows,
            )
            .icon("pivot-rows"),
            add(
                t("Add to Column Labels", "Ajouter aux étiquettes de colonnes"),
                Area::Columns,
            )
            .icon("pivot-columns"),
            add(
                t("Add to Values", "Ajouter à la zone Valeurs"),
                Area::Values,
            )
            .icon("pivot-sigma"),
        ];
        self.app
            .update(cx, |app, cx| open_menu_at(app, position, entries, cx))
            .ok();
    }

    fn field_row(&self, id: u32, row: &FieldRow, cx: &mut Context<Self>) -> AnyElement {
        let (field, used) = (row.field, row.used);
        let dragged = Dragged {
            source: DragSource::List(field),
            label: row.caption.clone(),
        };
        div()
            .id(("field", field))
            .h(px(22.))
            .px(px(6.))
            .flex()
            .items_center()
            .gap(px(6.))
            .hover(|s| s.bg(c(0xF5F5F5)))
            .child(check_mark(Tri::from(used), false))
            .child(
                div()
                    .flex_1()
                    .truncate()
                    .when(used, |d| d.font_weight(FontWeight::SEMIBOLD))
                    .child(row.caption.clone()),
            )
            .when(row.filtered, |d| {
                d.child(icon("filter", 11.0, theme::TEXT_MUTED))
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                let op = if used {
                    PaneOp::Uncheck(field)
                } else {
                    PaneOp::Check(field)
                };
                this.run(id, op, window, cx);
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, window, cx| this.field_menu(id, field, window, cx)),
            )
            .on_drag(dragged, |dragged, _, _, cx| {
                cx.new(|_| DragChip(dragged.label.clone()))
            })
            .into_any_element()
    }

    fn area_box(&self, shown: &Shown, area: Area, cx: &mut Context<Self>) -> AnyElement {
        let at = area_index(area);
        let entries = &shown.areas[at];
        let id = shown.id;
        let count = entries.len();
        let mark = self
            .drop_at
            .filter(|(a, _)| *a == area)
            .map(|(_, slot)| slot.min(count));
        let scroll = self.scroll[at].clone();
        let (name, icon_name, tag) = match area {
            Area::Filters => (t("Filters", "Filtres"), "filter", "area-filters"),
            Area::Columns => (t("Columns", "Colonnes"), "pivot-columns", "area-columns"),
            Area::Rows => (t("Rows", "Lignes"), "pivot-rows", "area-rows"),
            Area::Values => (t("Values", "Valeurs"), "pivot-sigma", "area-values"),
        };
        let mut chips: Vec<AnyElement> = Vec::with_capacity(count);
        for (index, e) in entries.iter().enumerate() {
            let dragged = Dragged {
                source: DragSource::Area(area, index),
                label: e.label.clone().into(),
            };
            chips.push(
                div()
                    .id((tag, index))
                    .h(px(ENTRY_H - 2.0))
                    .mb(px(2.))
                    .pl(px(6.))
                    .pr(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .bg(c(0xF3F3F3))
                    .border_1()
                    .border_color(c(theme::BORDER))
                    .rounded(px(2.))
                    .hover(|s| s.bg(c(theme::HOVER)))
                    .child(div().flex_1().truncate().child(e.label.clone()))
                    .child(icon("chevron-down", 9.0, theme::TEXT_MUTED))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let shown = this.app.upgrade().and_then(|app| Shown::of(app.read(cx)));
                        if let Some(shown) = shown {
                            this.entry_menu(&shown, area, index, window, cx);
                        }
                    }))
                    .on_drag(dragged, |dragged, _, _, cx| {
                        cx.new(|_| DragChip(dragged.label.clone()))
                    })
                    .into_any_element(),
            );
        }
        // The line showing where the dragged field would go.
        let insertion = mark.map(|slot| {
            div()
                .absolute()
                .left(px(3.))
                .right(px(3.))
                .top(px(2.0 + slot as f32 * ENTRY_H - 1.0))
                .h(px(2.))
                .bg(c(theme::ACCENT))
        });
        div()
            .flex_1()
            .min_w(px(0.))
            .flex()
            .flex_col()
            .gap(px(3.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .child(icon(icon_name, 13.0, theme::TEXT_MUTED))
                    .child(label(name)),
            )
            .child(
                div()
                    .id(tag)
                    .relative()
                    .h(px(AREA_H))
                    .p(px(3.))
                    .overflow_y_scroll()
                    .track_scroll(&scroll)
                    .bg(c(0xFFFFFF))
                    .border_1()
                    .border_color(c(if mark.is_some() {
                        theme::ACCENT
                    } else {
                        0xABABAB
                    }))
                    .rounded(px(3.))
                    .children(chips)
                    .children(insertion)
                    .on_drag_move::<Dragged>(cx.listener(
                        move |this, ev: &DragMoveEvent<Dragged>, _, cx| {
                            let position = ev.event.position;
                            let wanted = if ev.bounds.contains(&position) {
                                // Before the entry whose middle the pointer is above.
                                let y = f32::from(position.y - ev.bounds.top())
                                    - 3.0
                                    - f32::from(scroll.offset().y);
                                Some((area, logic::drop_slot(y, ENTRY_H, count)))
                            } else {
                                this.drop_at.filter(|(a, _)| *a != area)
                            };
                            if this.drop_at != wanted {
                                this.drop_at = wanted;
                                cx.notify();
                            }
                        },
                    ))
                    .on_drop(cx.listener(move |this, dragged: &Dragged, window, cx| {
                        this.dropped(id, dragged.source, Some(area), window, cx);
                    })),
            )
            .into_any_element()
    }
}

impl Render for FieldsPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !cx.has_active_drag() {
            self.drop_at = None;
        }
        let shown = self.app.upgrade().and_then(|app| Shown::of(app.read(cx)));
        let Some(shown) = shown else {
            // The active cell left the PivotTable: the pane is about to go.
            return div().into_any_element();
        };
        let id = shown.id;
        let needle = self.search.read(cx).text().trim().to_lowercase();
        let rows: Vec<AnyElement> = shown
            .fields
            .iter()
            .filter(|row| needle.is_empty() || row.caption.to_lowercase().contains(&needle))
            .map(|row| self.field_row(id, row, cx))
            .collect();
        let areas: Vec<AnyElement> = AREAS
            .iter()
            .map(|area| self.area_box(&shown, *area, cx))
            .collect();
        let mut areas = areas.into_iter();
        let mut pair = || {
            div()
                .flex()
                .gap(px(10.))
                .children(areas.next())
                .children(areas.next())
        };
        let (top, bottom) = (pair(), pair());
        let defer = shown.defer;
        div()
            .id("pivot-fields")
            .size_full()
            .px(px(12.))
            .pb(px(10.))
            .flex()
            .flex_col()
            .gap(px(7.))
            .text_size(px(12.))
            // Dropped beside the areas, an entry leaves the report.
            .on_drop(cx.listener(move |this, dragged: &Dragged, window, cx| {
                this.dropped(id, dragged.source, None, window, cx);
            }))
            .child(label(t(
                "Choose fields to add to report:",
                "Choisissez les champs à inclure dans le rapport :",
            )))
            .child(
                div().h(px(widgets::CONTROL_H)).flex().child(
                    TextField::new(&self.search).trailing(div().pr(px(4.)).child(icon(
                        "search",
                        12.0,
                        theme::TEXT_MUTED,
                    ))),
                ),
            )
            .child(
                div()
                    .id("pivot-field-list")
                    .flex_1()
                    .min_h(px(66.))
                    .overflow_y_scroll()
                    .bg(c(0xFFFFFF))
                    .border_1()
                    .border_color(c(0xABABAB))
                    .rounded(px(3.))
                    .py(px(2.))
                    .children(rows),
            )
            .child(div().line_height(px(16.)).child(t(
                "Drag fields between areas below:",
                "Faites glisser les champs dans les zones voulues ci-dessous :",
            )))
            .child(top)
            .child(bottom)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    // A long label (French) is cut short rather than
                    // push the button out of the pane.
                    .child(
                        div().flex_1().min_w(px(0.)).overflow_hidden().child(
                            Checkbox::new(
                                "defer",
                                t(
                                    "Defer Layout Update",
                                    "Différer la mise à jour de la disposition",
                                ),
                                defer,
                            )
                            .on_change(cx.listener(
                                |this, on: &bool, window, cx| {
                                    let on = *on;
                                    this.app
                                        .update(cx, |app, cx| {
                                            app.ext_mut::<PivotUi>().defer = on;
                                            // What was put aside is written
                                            // when deferring stops.
                                            if !on {
                                                update_deferred(app, window, cx);
                                            }
                                            cx.notify();
                                        })
                                        .ok();
                                },
                            )),
                        ),
                    )
                    .child(
                        Button::new("update", t("Update", "Mettre à jour"))
                            .disabled(!(defer && shown.pending))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.app
                                    .update(cx, |app, cx| update_deferred(app, window, cx))
                                    .ok();
                            })),
                    ),
            )
            .into_any_element()
    }
}
