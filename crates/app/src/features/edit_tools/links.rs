//! Hyperlinks: the Insert Hyperlink dialog, following a link with a click,
//! the hand and the ScreenTip over linked cells, the links AutoFormat
//! makes of what is typed, and the links that travel with pasted cells.
//! The links themselves are the engine's (`hyperlink.rs`).

use super::named;
use crate::app::ExcelApp;
use crate::features::names;
use crate::features::{CellEdit, ChangeCause, Cx, Intent, Painter, PaintLayer, Zone, ZoneEvent, ZoneResponse};
use crate::i18n::t;
use crate::shell::{Alert, Modal};
use crate::text_input::TextInput;
use crate::widgets::{self, Button, ListBox, TabStrip, TextField, dialog, label};
use gpui::{
    Context, CursorStyle, Entity, Focusable, MouseButton, PathPromptOptions, SharedString, Window, div, prelude::*, px,
};
use opensheet_engine::ast::{Expr, write_sheet_name};
use opensheet_engine::hyperlink::{Hyperlink, LINK_STYLE, LinkAction, LinkKind, auto_link};
use opensheet_engine::ops::PasteMode;
use opensheet_engine::protect::Action;
use opensheet_engine::workbook::ClearMode;
use opensheet_engine::{Content, Rect, Value};
use std::time::{Duration, Instant};

/// A press that lasts this long selects the cell instead of following its
/// link ("click and hold to select this cell").
const HOLD: Duration = Duration::from_millis(500);

/// What the feature remembers between events.
#[derive(Default)]
pub(super) struct LinkState {
    /// The linked cell the mouse button went down on, and when.
    press: Option<((u32, u32), Instant)>,
}

/// The payload of the zones over linked cells.
struct LinkZone;

fn has_link_function(e: &Expr) -> bool {
    let mut found = false;
    e.walk(&mut |x| found |= matches!(x, Expr::Func(name, _) if &**name == "HYPERLINK"));
    found
}

/// The range a click on a linked cell stands for: that of its link, or
/// the cell itself for a `HYPERLINK()` formula.
fn link_range(app: &ExcelApp, row: u32, col: u32) -> Rect {
    app.wb.hyperlink_at(app.sheet, row, col).map_or(Rect::cell(row, col), |link| link.range)
}

// ------------------------------------------------------------------ painting

/// Make the linked cells in view answer the pointer: the hand, and the
/// ScreenTip after a moment.
pub(super) fn paint(p: &mut Painter) {
    if p.layer != PaintLayer::Over {
        return;
    }
    let sheet = p.app.sheet;
    let Some(sh) = p.app.wb.sheets.get(sheet) else {
        return;
    };
    let visible = p.visible;
    let mut spots: Vec<(Rect, String)> = Vec::new();
    for link in &sh.hyperlinks {
        if let Some(shown) = link.range.intersection(&visible) {
            spots.push((shown, link.screen_tip()));
        }
    }
    // Cells whose formula makes a link with HYPERLINK().
    for ((row, col), cell) in sh.cells.range((visible.r0, 0)..=(visible.r1, u32::MAX)) {
        let Content::Formula(formula) = &cell.content else {
            continue;
        };
        if visible.contains(*row, *col) && has_link_function(formula) && p.app.wb.hyperlink_at(sheet, *row, *col).is_none() {
            if let Some(tip) = p.app.wb.link_tip_at(sheet, *row, *col) {
                spots.push((Rect::cell(*row, *col), tip));
            }
        }
    }
    for (rect, tip) in spots {
        let bounds = p.cell_box(rect);
        p.zone(bounds, LinkZone).cursor(CursorStyle::PointingHand).tooltip(tip);
    }
}

// ----------------------------------------------------------------- following

pub(super) fn zone_mouse_down(app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, _cx: &mut Cx) -> ZoneResponse {
    if zone.payload::<LinkZone>().is_none() {
        return ZoneResponse::Ignored;
    }
    // Only a plain click follows: with a modifier, a second click, while a
    // formula is typed, a range is picked for a dialog or the format
    // painter waits, the press is the cell's.
    let plain = ev.button == MouseButton::Left && ev.click_count == 1 && !ev.modifiers.modified();
    if !plain || app.is_editing() || app.pick.is_some() || app.painter.is_some() {
        return ZoneResponse::Ignored;
    }
    app.ext_mut::<LinkState>().press = Some((ev.cell, Instant::now()));
    ZoneResponse::Capture
}

pub(super) fn zone_mouse_up(app: &mut ExcelApp, zone: &Zone, ev: &ZoneEvent, window: &mut Window, cx: &mut Cx) {
    if zone.payload::<LinkZone>().is_none() {
        return;
    }
    let Some((cell, since)) = app.ext_mut::<LinkState>().press.take() else {
        return;
    };
    released(app, cell, ev.cell, since.elapsed(), window, cx);
}

/// The button came up over `at` after a press on the linked cell `cell`
/// that lasted `held`: a short click on the link follows it, anything
/// else selects the cell.
pub(super) fn released(app: &mut ExcelApp, cell: (u32, u32), at: (u32, u32), held: Duration, window: &mut Window, cx: &mut Cx) {
    let on_link = link_range(app, cell.0, cell.1).contains(at.0, at.1);
    if on_link && held < HOLD {
        follow(app, cell.0, cell.1, window, cx);
    } else {
        app.select_cell(cell.0, cell.1, cx);
    }
}

/// An address as the system opens it: the path of a file becomes a
/// `file:` URL, beside the workbook when it is relative.
fn openable(app: &ExcelApp, target: &str) -> String {
    if target.contains("://") || target.get(..7).is_some_and(|head| head.eq_ignore_ascii_case("mailto:")) {
        return target.to_string();
    }
    let path = std::path::Path::new(target);
    let full = match (path.is_absolute(), app.path.as_ref().and_then(|file| file.parent())) {
        (false, Some(folder)) => folder.join(path),
        _ => path.to_path_buf(),
    };
    format!("file://{}", full.display().to_string().replace(' ', "%20"))
}

/// Follow the link of a cell: a web page, a file or a message through the
/// system, a place in the workbook by selecting it. The link then takes
/// the look of a followed one.
pub(super) fn follow(app: &mut ExcelApp, row: u32, col: u32, window: &mut Window, cx: &mut Cx) {
    let sheet = app.sheet;
    let Some(action) = app.wb.link_action_at(sheet, row, col) else {
        return;
    };
    let range = link_range(app, row, col);
    match action {
        LinkAction::Open(target) => {
            cx.open_url(&openable(app, &target));
            // Excel leaves the linked cell selected behind the browser.
            app.select_cell(row, col, cx);
        }
        LinkAction::GoTo(place) => {
            let found = app.wb.reference_areas(sheet, row, col, &place);
            let shown = found.is_some_and(|areas| {
                let target = areas.first().map(|area| area.sheet);
                if let Some(target) = target.filter(|target| *target != sheet) {
                    app.switch_sheet(target, window, cx);
                }
                names::select_areas(app, &areas, cx)
            });
            if !shown {
                app.alert(Alert::new("OpenSheet", names::invalid_reference()).warning(), cx);
                return;
            }
        }
    }
    if app.wb.cell_style_at(sheet, row, col) == LINK_STYLE {
        app.wb.format_as_link(sheet, app.wb.clamp_to_used(sheet, range), true);
        app.commit(t("Followed Hyperlink", "Lien hypertexte visité"), cx);
    }
}

// ------------------------------------------------------------------- changes

/// Whether the protection of the sheet lets links be put on `range`;
/// says why when it does not.
fn may_link(app: &mut ExcelApp, range: Rect, cx: &mut Cx) -> bool {
    let ranges = [range];
    let asked = app.wb.permits(&Action::InsertHyperlink {
        sheet: app.sheet,
        ranges: &ranges,
    });
    match asked {
        Ok(()) => true,
        Err(denied) => {
            let message = denied.message(app.wb.locale);
            app.alert(Alert::new("OpenSheet", message).warning(), cx);
            false
        }
    }
}

/// Put `link` on its range, as OK does in the dialog: `display`, when
/// given, becomes the text of the first cell (an empty cell takes the
/// address), and the range takes the look of a link. One undo step.
pub(super) fn insert(app: &mut ExcelApp, link: Hyperlink, display: Option<String>, cx: &mut Cx) -> bool {
    if !may_link(app, link.range, cx) {
        return false;
    }
    let sheet = app.sheet;
    let (row, col) = (link.range.r0, link.range.c0);
    let shown = match link.kind() {
        LinkKind::Place => link.location.clone(),
        LinkKind::Email => link.email_parts().map(|(address, _)| address).unwrap_or_default(),
        _ => link.target(),
    };
    let empty = app.wb.sheets[sheet].is_blank(row, col);
    let text = match display {
        Some(text) if !text.is_empty() => Some(text),
        _ if empty => Some(shown),
        _ => None,
    };
    if let Some(text) = text {
        if app.wb.display_text(sheet, row, col).text != text {
            app.wb.set_content(sheet, row, col, Content::Value(Value::text(&text)));
        }
    }
    let range = link.range;
    app.wb.set_hyperlink(sheet, link);
    app.wb.format_as_link(sheet, app.wb.clamp_to_used(sheet, range).intersection(&range).unwrap_or(range), false);
    app.commit(t("Insert Hyperlink", "Insérer un lien hypertexte"), cx);
    true
}

/// Take the links off the selection: Clear Hyperlinks leaves the cells
/// their look, Remove Hyperlinks takes it off too.
pub(super) fn remove(app: &mut ExcelApp, with_format: bool, cx: &mut Cx) {
    if !app.may_all(Intent::EditCells, cx) {
        return;
    }
    let (sheet, ranges) = (app.sheet, app.sel().ranges.clone());
    let removed = app.wb.remove_hyperlinks(sheet, &ranges);
    if with_format && removed > 0 {
        for range in &ranges {
            app.wb.unformat_links(sheet, *range);
        }
    }
    let label = if with_format {
        t("Remove Hyperlinks", "Supprimer les liens hypertexte")
    } else {
        t("Clear Hyperlinks", "Effacer les liens hypertexte")
    };
    app.commit(label, cx);
}

/// Links follow what happens to their cells, in the same undo step: what
/// is typed may become one (AutoFormat), pasted cells bring theirs, and
/// cleared cells lose theirs.
pub(super) fn cells_written(app: &mut ExcelApp, edit: &CellEdit) {
    let sheet = edit.sheet;
    let (row, col) = (edit.rect.r0, edit.rect.c0);
    match edit.cause {
        ChangeCause::Typing if edit.rect.is_single() || app.wb.sheets[sheet].merge_at(row, col) == Some(edit.rect) => {
            let typed = match app.wb.cell(sheet, row, col).map(|cell| &cell.content) {
                Some(Content::Value(Value::Text(text))) => auto_link(text),
                _ => None,
            };
            let free = app.wb.hyperlink_at(sheet, row, col).is_none();
            let allowed = app
                .wb
                .permits(&Action::InsertHyperlink {
                    sheet,
                    ranges: &[edit.rect],
                })
                .is_ok();
            if let (Some(address), true, true) = (typed, free, allowed) {
                app.wb.set_hyperlink(sheet, Hyperlink::external(edit.rect, &address));
                app.wb.format_as_link(sheet, edit.rect, false);
            }
        }
        ChangeCause::Paste {
            from: Some((from_sheet, from)),
            cut,
            mode: PasteMode::All | PasteMode::AllExceptBorders,
        } => {
            if cut {
                app.wb.move_hyperlinks(from_sheet, from, sheet, row, col);
            } else {
                app.wb.copy_hyperlinks(from_sheet, from, sheet, row, col);
            }
        }
        ChangeCause::Clear(ClearMode::Contents | ClearMode::All) => {
            app.wb.remove_hyperlinks(sheet, &[edit.rect]);
        }
        _ => {}
    }
}

// ------------------------------------------------------------------- dialog

/// What "This Document" lists: the sheets, then the names.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Place {
    Sheet(String),
    Name(String),
}

/// Open the dialog on the selection: Insert Hyperlink, or Edit Hyperlink
/// when the active cell has a link.
pub(super) fn open(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let sheet = app.sheet;
    let (row, col) = app.active();
    let existing = app.wb.hyperlink_at(sheet, row, col).cloned();
    let range = existing.as_ref().map_or(app.sel().current(), |link| link.range);
    if !may_link(app, range, cx) {
        return;
    }
    // The text can be given when the cell holds text or nothing; a number
    // or a formula stays what it is.
    let (first_row, first_col) = (range.r0, range.c0);
    let editable = matches!(
        app.wb.cell(sheet, first_row, first_col).map(|cell| &cell.content),
        None | Some(Content::Empty) | Some(Content::Value(Value::Text(_)))
    );
    let shown = app.wb.display_text(sheet, first_row, first_col).text;
    let mut places: Vec<Place> = app.wb.sheets.iter().map(|sh| Place::Sheet(sh.name.clone())).collect();
    places.extend(names::visible_names(&app.wb, sheet).into_iter().map(Place::Name));
    let current = app.wb.sheets[sheet].name.clone();
    app.open_modal(window, cx, move |modal, _, cx| {
        let link = existing.clone().unwrap_or_default();
        let kind = existing.as_ref().map(Hyperlink::kind);
        let (mail, subject) = link.email_parts().unwrap_or_default();
        // A place is shown as its sheet in the list and its cells in the
        // box.
        let (place_sheet, reference) = match (kind, link.location.rsplit_once('!')) {
            (Some(LinkKind::Place), Some((sheet, cells))) => {
                let plain = sheet.trim_matches('\'').replace("''", "'");
                (Some(Place::Sheet(plain)), cells.to_string())
            }
            (Some(LinkKind::Place), None) => (Some(Place::Name(link.location.clone())), String::new()),
            _ => (Some(Place::Sheet(current.clone())), "A1".to_string()),
        };
        let place = place_sheet.and_then(|wanted| places.iter().position(|place| *place == wanted));
        LinkDialog {
            modal,
            range,
            editing: existing.is_some(),
            display: widgets::input(cx, if editable { shown.clone() } else { String::new() }),
            display_editable: editable,
            tooltip: link.tooltip.clone(),
            tab: match kind {
                Some(LinkKind::Place) => 1,
                Some(LinkKind::Email) => 2,
                _ => 0,
            },
            address: widgets::input(cx, if matches!(kind, Some(LinkKind::Web | LinkKind::File)) { link.target() } else { String::new() }),
            reference: widgets::input(cx, reference),
            places,
            place,
            email: widgets::input(cx, mail),
            subject: widgets::input(cx, subject),
            placed: false,
        }
    });
}

struct LinkDialog {
    modal: Modal,
    range: Rect,
    /// The cells already have a link: the dialog edits it.
    editing: bool,
    display: Entity<TextInput>,
    display_editable: bool,
    tooltip: String,
    /// 0 = Web Page or File, 1 = This Document, 2 = Email Address.
    tab: usize,
    address: Entity<TextInput>,
    reference: Entity<TextInput>,
    places: Vec<Place>,
    place: Option<usize>,
    email: Entity<TextInput>,
    subject: Entity<TextInput>,
    /// The keyboard was given to the first field of the tab.
    placed: bool,
}

/// The place a choice of "This Document" and the cell reference typed
/// stand for.
fn place_text(place: Option<&Place>, reference: &str) -> String {
    let reference = reference.trim();
    match place {
        Some(Place::Name(name)) => name.clone(),
        Some(Place::Sheet(sheet)) => {
            let mut out = String::new();
            write_sheet_name(sheet, &mut out);
            out.push('!');
            out.push_str(if reference.is_empty() { "A1" } else { reference });
            out
        }
        None => reference.to_string(),
    }
}

impl LinkDialog {
    /// The link the dialog describes; `None` while its tab is not filled.
    fn link(&self, cx: &Context<Self>) -> Option<Hyperlink> {
        let text = |field: &Entity<TextInput>| field.read(cx).text().trim().to_string();
        let mut link = match self.tab {
            0 => {
                let address = text(&self.address);
                if address.is_empty() {
                    return None;
                }
                // "www.example.org" is a web address, as AutoFormat has it.
                let address = auto_link(&address).filter(|_| !address.contains("://")).unwrap_or(address);
                Hyperlink::external(self.range, &address)
            }
            1 => {
                let place = place_text(self.place.and_then(|index| self.places.get(index)), &text(&self.reference));
                if place.is_empty() {
                    return None;
                }
                Hyperlink::place(self.range, &place)
            }
            _ => {
                let address = text(&self.email);
                if address.is_empty() {
                    return None;
                }
                Hyperlink::email(self.range, &address, &text(&self.subject))
            }
        };
        link.tooltip = self.tooltip.clone();
        Some(link)
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let Some(link) = self.link(cx) else {
            return;
        };
        let display = self.display_editable.then(|| self.display.read(cx).text().to_string());
        let done = self.modal.update_app(cx, |app, cx| {
            // A place must be one of this workbook.
            if link.kind() == LinkKind::Place {
                let (row, col) = app.active();
                if app.wb.reference_areas(app.sheet, row, col, &link.location).is_none() {
                    app.alert(Alert::new("OpenSheet", names::invalid_reference()).warning(), cx);
                    return false;
                }
            }
            insert(app, link, display, cx)
        });
        if done == Some(true) {
            self.modal.close(window, cx);
        }
    }

    fn remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| remove(app, true, cx));
    }

    fn screen_tip(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = cx.entity();
        let tip = self.tooltip.clone();
        self.modal.update_app(cx, |app, cx| {
            app.open_modal(window, cx, move |modal, _, cx| ScreenTipDialog {
                modal,
                dialog,
                text: widgets::input(cx, tip),
                placed: false,
            });
        });
    }

    /// "Select…": choose the file the link leads to.
    fn choose_file(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t("Select", "Sélectionner").into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                if let Some(path) = paths.first() {
                    let path = path.display().to_string();
                    this.update(cx, |this, cx| this.address.update(cx, |field, cx| field.set_text(path, cx))).ok();
                }
            }
        })
        .detach();
    }

    fn first_field(&self) -> &Entity<TextInput> {
        match self.tab {
            0 => &self.address,
            1 => &self.reference,
            _ => &self.email,
        }
    }
}

impl Render for LinkDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.placed {
            self.placed = true;
            let handle = self.first_field().read(cx).focus_handle(cx);
            window.focus(&handle);
        }
        let ready = self.link(cx).is_some();
        let row = |text: &'static str, control: gpui::AnyElement| {
            div().flex().items_center().gap(px(8.)).child(label(text).w(px(150.))).child(control)
        };
        let display = if self.display_editable {
            TextField::new(&self.display).into_any_element()
        } else {
            div()
                .flex_1()
                .text_color(crate::theme::c(crate::theme::TEXT_DISABLED))
                .child(t("<<Selection in Document>>", "<<Sélection dans le document>>"))
                .into_any_element()
        };
        let page = match self.tab {
            0 => div().flex().flex_col().gap(px(8.)).child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(label(t("Address:", "Adresse :")).w(px(150.)))
                    .child(TextField::new(&self.address))
                    .child(Button::new("select", t("Select...", "Sélectionner...")).on_click(cx.listener(|this, _, _, cx| this.choose_file(cx)))),
            ),
            1 => {
                let items: Vec<SharedString> = self
                    .places
                    .iter()
                    .map(|place| match place {
                        Place::Sheet(name) | Place::Name(name) => name.clone().into(),
                    })
                    .collect();
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(row(
                        t("Type in the cell reference:", "Entrer la référence de la cellule :"),
                        TextField::new(&self.reference).into_any_element(),
                    ))
                    .child(label(t("Select a place in this document:", "Sélectionnez un emplacement dans ce document :")))
                    .child(div().flex().child(
                        ListBox::new("places", items, self.place).height(140.0).on_change(cx.listener(|this, index: &usize, _, cx| {
                            this.place = Some(*index);
                            cx.notify();
                        })),
                    ))
            }
            _ => div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(row(t("Email address:", "Adresse de courrier :"), TextField::new(&self.email).into_any_element()))
                .child(row(t("Subject:", "Objet :"), TextField::new(&self.subject).into_any_element())),
        };
        let modal = self.modal.clone();
        let title = if self.editing {
            t("Edit Hyperlink", "Modifier le lien hypertexte")
        } else {
            t("Insert Hyperlink", "Insérer un lien hypertexte")
        };
        dialog(&self.modal, title)
            .width(560.0)
            .on_ok(cx.listener(Self::ok))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(label(t("Text to Display:", "Texte à afficher :")).w(px(150.)))
                    .child(display)
                    .child(named(
                        "link-screen-tip",
                        Button::new("screen-tip", t("ScreenTip...", "Info-bulle..."))
                            .on_click(cx.listener(|this, _, window, cx| this.screen_tip(window, cx))),
                    )),
            )
            .child(named(
                "link-tabs",
                TabStrip::new(
                    "tabs",
                    vec![
                        t("Web Page or File", "Page web ou fichier").into(),
                        t("This Document", "Ce document").into(),
                        t("Email Address", "Adresse de courrier").into(),
                    ],
                    self.tab,
                )
                .on_change(cx.listener(|this, index: &usize, window, cx| {
                    this.tab = *index;
                    let handle = this.first_field().read(cx).focus_handle(cx);
                    window.focus(&handle);
                    cx.notify();
                })),
            ))
            .child(div().min_h(px(210.)).child(page))
            .footer(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .when(self.editing, |d| {
                        d.child(named(
                            "link-remove",
                            Button::new("remove", t("Remove Link", "Supprimer le lien"))
                                .on_click(cx.listener(|this, _, window, cx| this.remove(window, cx))),
                        ))
                    })
                    .child(div().flex_1())
                    .child(Button::new("cancel", t("Cancel", "Annuler")).on_click(move |_, window, cx| modal.close(window, cx)))
                    .child(named(
                        "link-ok",
                        Button::new("ok", t("OK", "OK")).primary().disabled(!ready).on_click(cx.listener(Self::ok)),
                    )),
            )
    }
}

/// "Set Hyperlink ScreenTip", over the link dialog.
struct ScreenTipDialog {
    modal: Modal,
    dialog: Entity<LinkDialog>,
    text: Entity<TextInput>,
    /// The keyboard was given to the text box.
    placed: bool,
}

impl ScreenTipDialog {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let text = self.text.read(cx).text().to_string();
        self.dialog.update(cx, |dialog, cx| {
            dialog.tooltip = text;
            cx.notify();
        });
        self.modal.close(window, cx);
    }
}

impl Render for ScreenTipDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.placed {
            self.placed = true;
            let handle = self.text.read(cx).focus_handle(cx);
            window.focus(&handle);
        }
        dialog(&self.modal, t("Set Hyperlink ScreenTip", "Définir une info-bulle pour le lien hypertexte"))
            .width(380.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(t("ScreenTip text:", "Texte de l'info-bulle :")))
            .child(div().flex().child(TextField::new(&self.text)))
            .footer(widgets::ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}
