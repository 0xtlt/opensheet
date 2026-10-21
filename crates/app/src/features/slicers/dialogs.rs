//! The dialogs of slicers and timelines: Insert Slicers, Insert
//! Timelines, Slicer Settings, Report Connections (the PivotTables one
//! filter drives) and Filter Connections (the filters driving one
//! PivotTable).

use super::{
    Slicers, TIMELINE, insert_slicers, insert_timelines, selected_slicer, selected_timeline,
};
use crate::app::ExcelApp;
use crate::features::{Cx, Intent, object_id, pivot};
use crate::i18n::{t, t_ctx};
use crate::shell::{Alert, Modal};
use crate::text_input::TextInput;
use crate::theme::{self, c};
use crate::widgets::{
    self, Checkbox, GroupBox, RadioGroup, ScrollArea, TextField, dialog, label, labeled,
};
use gpui::{AnyElement, Context, Entity, SharedString, Window, div, prelude::*, px};
use opensheet_engine::slicer::{SlicerError, SlicerSource};

/// What new slicers filter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    /// A PivotTable, by id.
    Pivot(u32),
    /// A table, by id.
    Table(u32),
}

/// A list of check boxes: the rows of every dialog here.
fn check_rows(
    id: &'static str,
    rows: Vec<(SharedString, Option<SharedString>, bool)>,
    height: f32,
    on_toggle: impl Fn(usize, bool, &mut Window, &mut gpui::App) + Clone + 'static,
) -> AnyElement {
    let list = rows
        .into_iter()
        .enumerate()
        .map(|(k, (name, detail, checked))| {
            let on_toggle = on_toggle.clone();
            div()
                .h(px(widgets::ROW_H + 2.0))
                .px(px(6.))
                .flex()
                .items_center()
                .child(
                    div().flex_1().child(
                        Checkbox::new((id, k), name, checked)
                            .on_change(move |on, window, cx| on_toggle(k, *on, window, cx)),
                    ),
                )
                .children(detail.map(|text| {
                    div()
                        .w(px(140.))
                        .flex_none()
                        .text_color(c(theme::TEXT_MUTED))
                        .child(text)
                }))
        });
    div()
        .border_1()
        .border_color(c(theme::BORDER_STRONG))
        .bg(c(0xFFFFFF))
        .child(ScrollArea::new(id, height).child(div().py(px(2.)).flex().flex_col().children(list)))
        .into_any_element()
}

// ----------------------------------------------------------- insert slicers

/// The fields a slicer can be made for: (index, caption).
pub(crate) fn slicer_fields(app: &ExcelApp) -> Option<(Source, Vec<(usize, String)>)> {
    let (row, col) = app.active();
    if let Some(p) = pivot::target_pivot(app) {
        let fields = (0..p.fields.len())
            .map(|f| (f, p.field_caption(f).to_string()))
            .collect();
        return Some((Source::Pivot(p.id), fields));
    }
    let table = app.wb.table_at(app.sheet, row, col)?;
    let fields = table
        .columns
        .iter()
        .enumerate()
        .map(|(k, column)| (k, column.name.clone()))
        .collect();
    Some((Source::Table(table.id), fields))
}

/// The date fields of the PivotTable of the active cell, for a timeline.
pub(crate) fn timeline_fields(app: &ExcelApp) -> Option<(u32, Vec<(usize, String)>)> {
    let p = pivot::target_pivot(app)?;
    let fields = (0..p.fields.len())
        .filter(|f| p.cache.fields[*f].is_database() && p.cache.is_date(*f))
        .map(|f| (f, p.field_caption(f).to_string()))
        .collect();
    Some((p.id, fields))
}

/// "Insert Slicers" and "Insert Timelines": tick the fields to make one
/// for.
pub(super) struct InsertDialog {
    modal: Modal,
    /// `None`: timelines for the PivotTable `pivot`.
    source: Option<Source>,
    pivot: u32,
    pub(super) fields: Vec<(usize, SharedString, bool)>,
}

pub(super) fn open_insert_slicers(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some((source, fields)) = slicer_fields(app) else {
        return;
    };
    if !app.may(Intent::EditObject(None), cx) {
        return;
    }
    app.open_modal(window, cx, move |modal, _, _| InsertDialog {
        modal,
        source: Some(source),
        pivot: 0,
        fields: fields
            .into_iter()
            .map(|(f, name)| (f, name.into(), false))
            .collect(),
    });
}

pub(super) fn open_insert_timelines(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some((pivot, fields)) = timeline_fields(app) else {
        return;
    };
    if fields.is_empty() {
        let text = t(
            "We can't create a Timeline for this report because it doesn't have a field formatted as Date.",
            "Désolé... Nous ne pouvons pas créer de chronologie pour ce rapport, car ce dernier ne comporte pas de champ au format Date.",
        );
        app.alert(Alert::new("OpenSheet", text).warning(), cx);
        return;
    }
    if !app.may(Intent::EditObject(None), cx) {
        return;
    }
    app.open_modal(window, cx, move |modal, _, _| InsertDialog {
        modal,
        source: None,
        pivot,
        fields: fields
            .into_iter()
            .map(|(f, name)| (f, name.into(), false))
            .collect(),
    });
}

impl InsertDialog {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let chosen: Vec<usize> = self
            .fields
            .iter()
            .filter(|(_, _, on)| *on)
            .map(|(f, ..)| *f)
            .collect();
        let (source, pivot) = (self.source, self.pivot);
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| match source {
            Some(source) => insert_slicers(app, source, &chosen, cx),
            None => insert_timelines(app, pivot, &chosen, cx),
        });
    }
}

impl Render for InsertDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = match self.source {
            Some(_) => t("Insert Slicers", "Insérer des segments"),
            None => t("Insert Timelines", "Insérer une chronologie"),
        };
        let rows = self
            .fields
            .iter()
            .map(|(_, name, on)| (name.clone(), None, *on))
            .collect();
        let this = cx.weak_entity();
        dialog(&self.modal, title)
            .width(280.0)
            .on_ok(cx.listener(Self::ok))
            .child(check_rows(
                "fields",
                rows,
                widgets::ROW_H * 11.0,
                move |k, on, _, cx| {
                    this.update(cx, |this, cx| {
                        this.fields[k].2 = on;
                        cx.notify();
                    })
                    .ok();
                },
            ))
            .footer(widgets::ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// ---------------------------------------------------------- slicer settings

/// The Slicer Settings dialog.
pub(super) struct SettingsDialog {
    modal: Modal,
    slicer: u32,
    source_name: SharedString,
    cache_name: SharedString,
    pub(super) name: Entity<TextInput>,
    pub(super) caption: Entity<TextInput>,
    pub(super) show_header: bool,
    pub(super) descending: bool,
    custom_lists: bool,
    pub(super) hide_no_data: bool,
    indicate_no_data: bool,
    no_data_last: bool,
    show_deleted: bool,
}

pub(super) fn open_settings(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some(slicer) = selected_slicer(app).cloned() else {
        return;
    };
    if !app.may(Intent::EditObject(Some(object_id(&Slicers, slicer.id))), cx) {
        return;
    }
    let source_name = match &slicer.source {
        SlicerSource::Pivot { field, .. } => field.clone(),
        SlicerSource::Table { column, .. } => column.clone(),
    };
    app.open_modal(window, cx, move |modal, _, cx| SettingsDialog {
        modal,
        slicer: slicer.id,
        source_name: source_name.into(),
        cache_name: slicer.cache_name.clone().into(),
        name: widgets::input(cx, slicer.name.clone()),
        caption: widgets::input(cx, slicer.caption.clone()),
        show_header: slicer.show_header,
        descending: slicer.descending,
        custom_lists: slicer.custom_lists,
        hide_no_data: slicer.hide_no_data,
        indicate_no_data: slicer.indicate_no_data,
        no_data_last: slicer.no_data_last,
        show_deleted: slicer.show_deleted,
    });
}

impl SettingsDialog {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let id = self.slicer;
        let name = self.name.read(cx).text().trim().to_string();
        let caption = self.caption.read(cx).text().to_string();
        let (show_header, descending, custom_lists) =
            (self.show_header, self.descending, self.custom_lists);
        let (hide, indicate, last, deleted) = (
            self.hide_no_data,
            self.indicate_no_data,
            self.no_data_last,
            self.show_deleted,
        );
        let renamed = self.modal.update_app(cx, |app, cx| {
            if let Err(err) = app.wb.rename_slicer(id, &name) {
                app.wb.rollback();
                if err == SlicerError::InvalidName {
                    let text = t(
                        "Filter control name already in use. Enter a unique name.",
                        "Ce nom de contrôle de filtre est déjà utilisé. Veuillez entrer un nom unique.",
                    );
                    app.alert(Alert::new("OpenSheet", text).warning(), cx);
                }
                return false;
            }
            app.wb.update_slicer(id, |s| {
                s.caption = caption;
                s.show_header = show_header;
                s.descending = descending;
                s.custom_lists = custom_lists;
                s.hide_no_data = hide;
                s.indicate_no_data = indicate;
                s.no_data_last = last;
                s.show_deleted = deleted;
            });
            app.commit(t_ctx("dialog", "Slicer Settings", "Paramètres du segment"), cx);
            true
        });
        if renamed == Some(true) {
            self.modal.close(window, cx);
        }
    }
}

impl Render for SettingsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let fact = |title: &'static str, value: &SharedString| {
            div()
                .flex()
                .gap(px(6.))
                .child(label(title))
                .child(label(value.clone()).text_color(c(theme::TEXT_MUTED)))
        };
        let check = |id: &'static str,
                     text: &'static str,
                     on: bool,
                     enabled: bool,
                     set: fn(&mut Self, bool)| {
            Checkbox::new(id, text, on)
                .disabled(!enabled)
                .on_change(cx.listener(move |this, on: &bool, _, cx| {
                    set(this, *on);
                    cx.notify();
                }))
        };
        let sorting = RadioGroup::new(
            "order",
            vec![
                t("Ascending (A to Z)", "Croissant (de A à Z)").into(),
                t("Descending (Z to A)", "Décroissant (de Z à A)").into(),
            ],
            Some(usize::from(self.descending)),
        )
        .on_change(cx.listener(|this, ix: &usize, _, cx| {
            this.descending = *ix == 1;
            cx.notify();
        }));
        let (hide, indicate) = (self.hide_no_data, self.indicate_no_data);
        dialog(
            &self.modal,
            t_ctx("dialog", "Slicer Settings", "Paramètres du segment"),
        )
        .width(520.0)
        .on_ok(cx.listener(Self::ok))
        .child(fact(
            t("Source Name:", "Nom de la source\u{a0}:"),
            &self.source_name,
        ))
        .child(fact(
            t(
                "Name to use in formulas:",
                "Nom à utiliser dans les formules\u{a0}:",
            ),
            &self.cache_name,
        ))
        .child(labeled(
            t("Name:", "Nom\u{a0}:"),
            TextField::new(&self.name).width(260.0),
        ))
        .child(
            GroupBox::new(t("Header", "En-tête"))
                .child(check(
                    "header",
                    t("Display header", "Afficher l’en-tête"),
                    self.show_header,
                    true,
                    |this, on| this.show_header = on,
                ))
                .child(labeled(
                    t("Caption:", "Légende\u{a0}:"),
                    TextField::new(&self.caption).width(260.0),
                )),
        )
        .child(
            GroupBox::new(t(
                "Item Sorting and Filtering",
                "Tri et filtrage des éléments",
            ))
            .child(
                div()
                    .flex()
                    .gap(px(16.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(sorting)
                            .child(check(
                                "lists",
                                t(
                                    "Use Custom Lists when sorting",
                                    "Utiliser des listes personnalisées lors du tri",
                                ),
                                self.custom_lists,
                                true,
                                |this, on| this.custom_lists = on,
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(check(
                                "hide",
                                t(
                                    "Hide items with no data",
                                    "Masquer les éléments sans données",
                                ),
                                hide,
                                true,
                                |this, on| this.hide_no_data = on,
                            ))
                            .child(check(
                                "indicate",
                                t(
                                    "Visually indicate items with no data",
                                    "Indiquer visuellement les éléments sans données",
                                ),
                                indicate,
                                !hide,
                                |this, on| this.indicate_no_data = on,
                            ))
                            .child(check(
                                "last",
                                t(
                                    "Show items with no data last",
                                    "Afficher en dernier les éléments sans données",
                                ),
                                self.no_data_last,
                                !hide && indicate,
                                |this, on| this.no_data_last = on,
                            ))
                            .child(check(
                                "deleted",
                                t(
                                    "Show items deleted from the data source",
                                    "Afficher les éléments supprimés de la source de données",
                                ),
                                self.show_deleted,
                                true,
                                |this, on| this.show_deleted = on,
                            )),
                    ),
            ),
        )
        .footer(widgets::ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// -------------------------------------------------------------- connections

/// What a connections dialog ticks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Subject {
    /// The PivotTables a slicer filters.
    Slicer(u32),
    /// The PivotTables a timeline filters.
    Timeline(u32),
    /// The slicers and timelines filtering a PivotTable.
    Pivot(u32),
}

/// A row of a connections dialog: a PivotTable, or a filter.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Link {
    /// Id of the PivotTable, or object id of the slicer or timeline.
    pub(super) id: u32,
    pub(super) name: SharedString,
    sheet: SharedString,
    pub(super) on: bool,
}

/// "Report Connections" and "Filter Connections".
pub(super) struct ConnectionsDialog {
    modal: Modal,
    subject: Subject,
    caption: SharedString,
    pub(super) links: Vec<Link>,
}

/// The PivotTables a filter on `pivots` can be connected to, ticked when
/// it is.
fn report_links(app: &ExcelApp, pivots: &[u32]) -> Vec<Link> {
    let Some(first) = pivots
        .iter()
        .copied()
        .find(|id| app.wb.pivot(*id).is_some())
    else {
        return Vec::new();
    };
    app.wb
        .pivots_sharing_cache(first)
        .into_iter()
        .filter_map(|(sheet, id)| {
            let (_, p) = app.wb.pivot(id)?;
            Some(Link {
                id,
                name: p.name.clone().into(),
                sheet: app.wb.sheets[sheet].name.clone().into(),
                on: pivots.contains(&id),
            })
        })
        .collect()
}

/// The slicers and timelines that can filter a PivotTable: those on its
/// cache, ticked when they are connected to it.
fn filter_links(app: &ExcelApp, pivot: u32) -> Vec<Link> {
    let shared: Vec<u32> = app
        .wb
        .pivots_sharing_cache(pivot)
        .into_iter()
        .map(|(_, id)| id)
        .collect();
    let on_cache = |pivots: &[u32]| pivots.iter().any(|id| shared.contains(id));
    let mut out = Vec::new();
    for (sheet, s) in app.wb.slicers() {
        if on_cache(s.pivots()) {
            out.push(Link {
                id: s.id,
                name: s.caption.clone().into(),
                sheet: app.wb.sheets[sheet].name.clone().into(),
                on: s.pivots().contains(&pivot),
            });
        }
    }
    for (sheet, timeline) in app.wb.timelines() {
        if on_cache(&timeline.pivots) {
            out.push(Link {
                id: timeline.id | TIMELINE,
                name: timeline.caption.clone().into(),
                sheet: app.wb.sheets[sheet].name.clone().into(),
                on: timeline.pivots.contains(&pivot),
            });
        }
    }
    out
}

pub(super) fn open_report_connections(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let (subject, caption, links) = if let Some(slicer) = selected_slicer(app) {
        (
            Subject::Slicer(slicer.id),
            slicer.caption.clone(),
            report_links(app, slicer.pivots()),
        )
    } else if let Some(timeline) = selected_timeline(app) {
        (
            Subject::Timeline(timeline.id),
            timeline.caption.clone(),
            report_links(app, &timeline.pivots),
        )
    } else {
        return;
    };
    if !app.may(Intent::UsePivotTable, cx) {
        return;
    }
    app.open_modal(window, cx, move |modal, _, _| ConnectionsDialog {
        modal,
        subject,
        caption: caption.into(),
        links,
    });
}

pub(super) fn open_filter_connections(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some(p) = pivot::target_pivot(app) else {
        return;
    };
    let (id, caption) = (p.id, p.name.clone());
    let links = filter_links(app, id);
    if !app.may(Intent::UsePivotTable, cx) {
        return;
    }
    app.open_modal(window, cx, move |modal, _, _| ConnectionsDialog {
        modal,
        subject: Subject::Pivot(id),
        caption: caption.into(),
        links,
    });
}

/// Connect a PivotTable to exactly the filters `wanted` (object ids)
/// among those of `links`.
fn connect_filters(
    app: &mut ExcelApp,
    pivot: u32,
    links: &[(u32, bool)],
    overwrite: bool,
) -> Result<(), opensheet_engine::pivot::PivotError> {
    for (object, on) in links {
        let timeline = object & TIMELINE != 0;
        let id = object & !TIMELINE;
        let mut pivots: Vec<u32> = if timeline {
            app.wb
                .timeline(id)
                .map(|(_, t)| t.pivots.clone())
                .unwrap_or_default()
        } else {
            app.wb
                .slicer(id)
                .map(|(_, s)| s.pivots().to_vec())
                .unwrap_or_default()
        };
        if pivots.contains(&pivot) == *on {
            continue;
        }
        if *on {
            pivots.push(pivot);
        } else {
            pivots.retain(|p| *p != pivot);
        }
        if timeline {
            app.wb.timeline_connect(id, &pivots, overwrite)?;
        } else {
            app.wb.slicer_connect(id, &pivots, overwrite)?;
        }
    }
    Ok(())
}

impl ConnectionsDialog {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let subject = self.subject;
        let links: Vec<(u32, bool)> = self.links.iter().map(|link| (link.id, link.on)).collect();
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            let chosen: Vec<u32> = links
                .iter()
                .filter(|(_, on)| *on)
                .map(|(id, _)| *id)
                .collect();
            match subject {
                Subject::Slicer(id) => pivot::run_op(
                    app,
                    t("Report Connections", "Connexions de rapport"),
                    window,
                    cx,
                    move |app, overwrite| app.wb.slicer_connect(id, &chosen, overwrite),
                ),
                Subject::Timeline(id) => pivot::run_op(
                    app,
                    t("Report Connections", "Connexions de rapport"),
                    window,
                    cx,
                    move |app, overwrite| app.wb.timeline_connect(id, &chosen, overwrite),
                ),
                Subject::Pivot(id) => pivot::run_op(
                    app,
                    t("Filter Connections", "Connexions de filtre"),
                    window,
                    cx,
                    move |app, overwrite| connect_filters(app, id, &links, overwrite),
                ),
            }
        });
    }
}

impl Render for ConnectionsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (title, hint) = match self.subject {
            Subject::Pivot(_) => (
                t("Filter Connections ({})", "Connexions de filtre ({})"),
                t(
                    "Select filters to connect to this PivotTable",
                    "Sélectionner les filtres à connecter à ce tableau croisé dynamique",
                ),
            ),
            _ => (
                t("Report Connections ({})", "Connexions de rapport ({})"),
                t(
                    "Select PivotTable and PivotChart reports to connect to this filter",
                    "Sélectionner les rapports de tableaux croisés dynamiques et de graphiques croisés dynamiques à connecter à ce filtre",
                ),
            ),
        };
        let rows = self
            .links
            .iter()
            .map(|link| (link.name.clone(), Some(link.sheet.clone()), link.on))
            .collect();
        let this = cx.weak_entity();
        let heading = |text: &'static str| label(text).text_color(c(theme::TEXT_MUTED));
        dialog(&self.modal, title.replace("{}", &self.caption))
            .width(420.0)
            .on_ok(cx.listener(Self::ok))
            .child(div().child(hint))
            .child(
                div()
                    .px(px(6.))
                    .flex()
                    .child(div().flex_1().pl(px(22.)).child(heading(t("Name", "Nom"))))
                    .child(div().w(px(140.)).child(heading(t("Sheet", "Feuille")))),
            )
            .child(check_rows(
                "links",
                rows,
                widgets::ROW_H * 8.0,
                move |k, on, _, cx| {
                    this.update(cx, |this, cx| {
                        this.links[k].on = on;
                        cx.notify();
                    })
                    .ok();
                },
            ))
            .footer(widgets::ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}
