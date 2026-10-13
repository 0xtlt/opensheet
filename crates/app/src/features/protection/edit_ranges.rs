//! Review ▸ Allow Edit Ranges: the ranges of a sheet that stay editable
//! once it is protected, by anyone or by who knows their password.
//!
//! Excel for Mac has the command and the words of its dialogs (range
//! title, cells, password) but its nibs hold no layout for them, so the
//! dialogs are those of Excel for Windows — Allow Users to Edit Ranges,
//! New Range, Modify Range, Unlock Range — built from this program's own
//! controls, without what only Windows accounts give (Permissions).
//!
//! The list is edited on a draft: OK writes it to the sheet in one undo
//! step. The password of a range is asked when a cell of it is about to
//! be changed, and only once while the workbook is open.

use super::{Cmd, Memory, password_field};
use crate::app::ExcelApp;
use crate::features::{Command, Cx, Intent};
use crate::i18n::t;
use crate::shell::{Alert, Modal, RefStyle};
use crate::text_input::TextInput;
use crate::theme::{self, c};
use crate::widgets::{self, Button, ListBox, RefEdit, TextField, dialog, label, labeled, ok_cancel};
use gpui::{Context, Entity, SharedString, WeakEntity, Window, div, prelude::*, px};
use opensheet_engine::Rect;
use opensheet_engine::protect::{PasswordError, PasswordHash, ProtectedRange, RangeError};

/// References as the dialogs write them: `=$A$1:$B$2`.
const CELLS: RefStyle = RefStyle {
    absolute: true,
    sheet: false,
    equals: true,
};

/// The cells of a range as text, its areas apart by the separator of the
/// workbook's language.
fn cells_text(app: &ExcelApp, ranges: &[Rect]) -> String {
    let mut text = String::new();
    for (n, rect) in ranges.iter().enumerate() {
        match n {
            0 => text.push_str(&CELLS.format(*rect, "")),
            _ => {
                text.push(app.wb.locale.arg_sep);
                text.push_str(CELLS.format(*rect, "").trim_start_matches('='));
            }
        }
    }
    text
}

/// The areas a typed reference names on the current sheet; `None` when
/// it is not a reference to cells of this sheet.
fn typed_cells(app: &ExcelApp, text: &str) -> Option<Vec<Rect>> {
    let (row, col) = app.active();
    let areas = app.wb.reference_areas(app.sheet, row, col, text.trim().trim_start_matches('='))?;
    (!areas.is_empty() && areas.iter().all(|area| area.sheet == app.sheet)).then(|| areas.iter().map(|area| area.rect).collect())
}

fn range_error(error: RangeError) -> &'static str {
    match error {
        RangeError::BadTitle => t("The title you entered is not valid.", "Le titre que vous avez entré n’est pas valide."),
        RangeError::TitleTaken => t("The title you entered already exists. Enter a unique title.", "Le titre que vous avez entré existe déjà. Entrez un titre unique."),
        RangeError::NoCells => t("Reference is not valid.", "La référence n’est pas valide."),
    }
}

// ------------------------------------------ Allow Users to Edit Ranges

/// Review ▸ Allow Edit Ranges.
pub(super) fn open(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) -> Entity<EditRanges> {
    let (sheet, ranges) = (app.sheet, app.wb.sheets[app.sheet].protected_ranges.clone());
    app.open_modal(window, cx, move |modal, _, _| EditRanges {
        modal,
        sheet,
        selected: (!ranges.is_empty()).then_some(0),
        ranges,
    })
}

pub(crate) struct EditRanges {
    modal: Modal,
    sheet: usize,
    /// The list being edited; the sheet has it at OK.
    ranges: Vec<ProtectedRange>,
    selected: Option<usize>,
}

impl EditRanges {
    pub(super) fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = Some(index);
        cx.notify();
    }

    /// New (`None`) or Modify: the dialog of one range, over this one.
    pub(super) fn edit(&mut self, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) -> Option<Entity<RangeDialog>> {
        let existing = index.and_then(|index| self.ranges.get(index).cloned());
        if index.is_some() && existing.is_none() {
            return None;
        }
        let list = cx.entity().downgrade();
        let others: Vec<ProtectedRange> = self.ranges.iter().enumerate().filter(|(i, _)| Some(*i) != index).map(|(_, range)| range.clone()).collect();
        self.modal.update_app(cx, |app, cx| {
            let (title, cells) = match &existing {
                Some(range) => (range.name.clone(), cells_text(app, &range.ranges)),
                // A new range proposes the selection, under a free title.
                None => {
                    let base = t("Range", "Plage");
                    let title = (1..).map(|n| format!("{base}{n}")).find(|title| !others.iter().any(|other| other.name.eq_ignore_ascii_case(title))).unwrap_or_default();
                    (title, cells_text(app, &app.sel().ranges.clone()))
                }
            };
            app.open_modal(window, cx, move |modal, _, cx| RangeDialog {
                title: widgets::input(cx, title),
                cells: modal.ref_input(CELLS, cells, cx),
                password: password_field(cx),
                new_password: None,
                modal,
                list,
                index,
                existing,
                others,
            })
        })
    }

    pub(super) fn delete(&mut self, cx: &mut Context<Self>) {
        if let Some(index) = self.selected.filter(|index| *index < self.ranges.len()) {
            self.ranges.remove(index);
            self.selected = if self.ranges.is_empty() { None } else { Some(index.min(self.ranges.len() - 1)) };
            cx.notify();
        }
    }

    /// Take a range from its dialog: a new one, or the one at `index`
    /// changed.
    fn put(&mut self, index: Option<usize>, range: ProtectedRange, cx: &mut Context<Self>) {
        match index {
            Some(index) if index < self.ranges.len() => {
                self.ranges[index] = range;
                self.selected = Some(index);
            }
            _ => {
                self.ranges.push(range);
                self.selected = Some(self.ranges.len() - 1);
            }
        }
        cx.notify();
    }

    /// Write the list to the sheet, when it differs. False when the
    /// sheet refuses.
    fn apply(&mut self, cx: &mut Context<Self>) -> bool {
        let (sheet, ranges) = (self.sheet, self.ranges.clone());
        self.modal
            .update_app(cx, |app, cx| {
                if app.wb.sheets.get(sheet).is_none_or(|on| on.protected_ranges == ranges) {
                    return true;
                }
                if !app.may(Intent::ChangeSheet, cx) {
                    return false;
                }
                app.wb.set_protected_ranges(sheet, ranges);
                app.commit(Cmd::AllowEditRanges.label(), cx);
                true
            })
            .unwrap_or(false)
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        if self.apply(cx) {
            self.modal.close(window, cx);
        }
    }

    /// Protect Sheet: the list is kept, then the sheet is protected in
    /// its own dialog.
    fn protect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.apply(cx) {
            self.modal.close(window, cx);
            self.modal.update_app(cx, |app, cx| app.run_command(Cmd::ProtectSheet, window, cx));
        }
    }
}

impl Render for EditRanges {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<(SharedString, SharedString)> = self
            .modal
            .read_app(cx, |app| self.ranges.iter().map(|range| (range.name.clone().into(), cells_text(app, &range.ranges).trim_start_matches('=').to_string().into())).collect())
            .unwrap_or_default();
        let names: Vec<SharedString> = rows.iter().map(|(title, _)| title.clone()).collect();
        let none = self.selected.is_none();
        let columns = |title: SharedString, cells: SharedString| div().flex().child(div().w(px(130.)).flex_none().overflow_hidden().child(title)).child(div().flex_1().overflow_hidden().child(cells));
        let side = |button: Button| button.width(100.0);
        let modal = self.modal.clone();
        dialog(&self.modal, t("Allow Users to Edit Ranges", "Permettre aux utilisateurs de modifier des plages"))
            .width(470.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(t(
                "Ranges unlocked by a password when sheet is protected:",
                "Plages déverrouillées par mot de passe quand la feuille est protégée :",
            )))
            .child(
                div()
                    .flex()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .px(px(6.))
                                    .text_color(c(theme::TEXT_MUTED))
                                    .child(columns(t("Title", "Titre").into(), t("Refers to cells", "Fait référence aux cellules").into())),
                            )
                            .child(
                                ListBox::new("edit-ranges", names, self.selected)
                                    .width(330.0)
                                    .height(widgets::ROW_H * 6.0)
                                    .row(move |index, _| {
                                        let (title, cells) = rows[index].clone();
                                        columns(title, cells).into_any_element()
                                    })
                                    .on_change(cx.listener(|this, index: &usize, _, cx| this.select(*index, cx)))
                                    .on_activate(cx.listener(|this, index: &usize, window, cx| {
                                        this.edit(Some(*index), window, cx);
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .pt(px(18.))
                            .child(side(Button::new("ranges-new", t("New...", "Nouveau..."))).on_click(cx.listener(|this, _, window, cx| {
                                this.edit(None, window, cx);
                            })))
                            .child(side(Button::new("ranges-modify", t("Modify...", "Modifier..."))).disabled(none).on_click(cx.listener(|this, _, window, cx| {
                                let index = this.selected;
                                if index.is_some() {
                                    this.edit(index, window, cx);
                                }
                            })))
                            .child(side(Button::new("ranges-delete", t("Delete", "Supprimer"))).disabled(none).on_click(cx.listener(|this, _, _, cx| this.delete(cx)))),
                    ),
            )
            .footer(
                div()
                    .flex()
                    .w_full()
                    .gap(px(8.))
                    .child(Button::new("ranges-protect", t("Protect Sheet...", "Protéger la feuille...")).on_click(cx.listener(|this, _, window, cx| this.protect(window, cx))))
                    .child(div().flex_1())
                    .child(Button::new("ranges-ok", t("OK", "OK")).primary().on_click(cx.listener(Self::ok)))
                    .child(Button::new("ranges-cancel", t("Cancel", "Annuler")).on_click(move |_, window, cx| modal.close(window, cx))),
            )
    }
}

// ------------------------------------------- New Range / Modify Range

pub(crate) struct RangeDialog {
    modal: Modal,
    /// The list this range goes to.
    list: WeakEntity<EditRanges>,
    /// The place of the range being modified; `None` for a new one.
    index: Option<usize>,
    existing: Option<ProtectedRange>,
    /// The other ranges of the list: their titles are taken.
    others: Vec<ProtectedRange>,
    title: Entity<TextInput>,
    cells: Entity<TextInput>,
    /// The password of a new range.
    password: Entity<TextInput>,
    /// The password chosen with Password… for a range being modified:
    /// `Some(None)` removes it.
    new_password: Option<Option<PasswordHash>>,
}

impl RangeDialog {
    #[cfg(test)]
    pub(super) fn set(&mut self, title: &str, cells: &str, password: &str, cx: &mut Context<Self>) {
        self.title.update(cx, |field, cx| field.set_text(title, cx));
        self.cells.update(cx, |field, cx| field.set_text(cells, cx));
        self.password.update(cx, |field, cx| field.set_text(password, cx));
    }

    /// The range the dialog describes, its password aside; the message
    /// to show when it cannot be.
    fn described(&self, cx: &Context<Self>) -> Result<ProtectedRange, &'static str> {
        let title = self.title.read(cx).text().trim().to_string();
        let cells = self.cells.read(cx).text().to_string();
        let ranges = self.modal.read_app(cx, |app| typed_cells(app, &cells)).flatten().ok_or_else(|| range_error(RangeError::NoCells))?;
        let mut range = match &self.existing {
            Some(existing) => existing.clone(),
            None => ProtectedRange::new("", Vec::new(), None),
        };
        range.name = title;
        range.ranges = ranges;
        if let Some(password) = &self.new_password {
            range.password = password.clone();
        }
        // Checked against the list being edited, not against the sheet.
        let key = range.name.to_lowercase();
        if self.others.iter().any(|other| other.name.to_lowercase() == key) {
            return Err(range_error(RangeError::TitleTaken));
        }
        self.modal.read_app(cx, |app| app.wb.protected_range_valid(usize::MAX, &range, None)).unwrap_or(Ok(())).map_err(range_error)?;
        Ok(range)
    }

    fn finish(&mut self, range: ProtectedRange, window: &mut Window, cx: &mut Context<Self>) {
        let index = self.index;
        self.list.update(cx, |list, cx| list.put(index, range, cx)).ok();
        self.modal.close(window, cx);
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let range = match self.described(cx) {
            Ok(range) => range,
            Err(message) => {
                self.modal.update_app(cx, |app, cx| app.alert(Alert::new("OpenSheet", message).warning(), cx));
                return;
            }
        };
        let password = self.password.read(cx).text().to_string();
        if self.existing.is_some() || password.is_empty() {
            self.finish(range, window, cx);
            return;
        }
        // The password of a new range is typed a second time.
        let dialog = cx.entity().downgrade();
        self.modal.update_app(cx, |app, cx| {
            app.open_modal(window, cx, move |modal, _, cx| ConfirmRangePassword {
                modal,
                again: password_field(cx),
                password,
                then: Box::new(move |password, window, cx| {
                    dialog
                        .update(cx, |dialog, cx| {
                            let mut range = range.clone();
                            range.password = Some(PasswordHash::new(password));
                            dialog.finish(range, window, cx);
                        })
                        .ok();
                }),
            });
        });
    }

    /// Password…: another password for the range being modified, or none.
    fn change_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = cx.entity().downgrade();
        self.modal.update_app(cx, |app, cx| {
            app.open_modal(window, cx, move |modal, _, cx| ChangePassword {
                modal,
                dialog,
                password: password_field(cx),
                again: password_field(cx),
            });
        });
    }
}

impl Render for RangeDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = match self.existing {
            Some(_) => t("Modify Range", "Modifier la plage"),
            None => t("New Range", "Nouvelle plage"),
        };
        let password = match self.existing {
            None => div().child(label(t("Range password:", "Mot de passe de la plage :"))).child(TextField::new(&self.password).width(280.0)),
            Some(_) => div().flex().child(Button::new("range-password", t("Password...", "Mot de passe...")).on_click(cx.listener(|this, _, window, cx| this.change_password(window, cx)))),
        };
        dialog(&self.modal, title)
            .width(320.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(t("Title:", "Titre :")))
            .child(TextField::new(&self.title).width(280.0))
            .child(label(t("Refers to cells:", "Fait référence aux cellules :")))
            .child(RefEdit::new(&self.cells, &self.modal).width(280.0))
            .child(password)
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

/// The password of a new range, typed again.
pub(crate) struct ConfirmRangePassword {
    modal: Modal,
    password: String,
    again: Entity<TextInput>,
    then: Box<dyn Fn(&str, &mut Window, &mut gpui::App)>,
}

impl ConfirmRangePassword {
    #[cfg(test)]
    pub(super) fn set(&mut self, text: &str, cx: &mut Context<Self>) {
        self.again.update(cx, |field, cx| field.set_text(text, cx));
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        self.modal.close(window, cx);
        if self.again.read(cx).text() != self.password {
            self.modal.update_app(cx, |app, cx| {
                let message = PasswordError::Mismatch.message(app.wb.locale);
                app.alert(Alert::new("OpenSheet", message).warning(), cx);
            });
            return;
        }
        (self.then)(&self.password, window, cx);
    }
}

impl Render for ConfirmRangePassword {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        dialog(&self.modal, t("Confirm Password", "Confirmer le mot de passe"))
            .width(340.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(t("Reenter password to proceed.", "Entrez de nouveau le mot de passe pour continuer.")))
            .child(div().flex().child(TextField::new(&self.again)))
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

/// Change Range Password, from Modify Range.
struct ChangePassword {
    modal: Modal,
    dialog: WeakEntity<RangeDialog>,
    password: Entity<TextInput>,
    again: Entity<TextInput>,
}

impl ChangePassword {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let password = self.password.read(cx).text().to_string();
        if self.again.read(cx).text() != password {
            self.modal.update_app(cx, |app, cx| {
                let message = PasswordError::Mismatch.message(app.wb.locale);
                app.alert(Alert::new("OpenSheet", message).warning(), cx);
            });
            return;
        }
        let hash = (!password.is_empty()).then(|| PasswordHash::new(&password));
        self.dialog.update(cx, |dialog, _| dialog.new_password = Some(hash)).ok();
        self.modal.close(window, cx);
    }
}

impl Render for ChangePassword {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        dialog(&self.modal, t("Change range password", "Modifier le mot de passe de la plage"))
            .width(320.0)
            .on_ok(cx.listener(Self::ok))
            .child(labeled(t("New password:", "Nouveau mot de passe :"), TextField::new(&self.password)))
            .child(labeled(t("Confirm new password:", "Confirmer le nouveau mot de passe :"), TextField::new(&self.again)))
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}

// --------------------------------------------------------- Unlock Range

/// A change was refused on a protected sheet: when a range with a
/// password would allow it, its password is asked instead of saying no.
/// Returns whether that is so. The dialog opens when the window is next
/// drawn, since no window is at hand where changes are refused.
pub(crate) fn password_wanted(app: &mut ExcelApp, intent: &Intent, cx: &mut Cx) -> bool {
    let Intent::EditCells(rect) = intent else {
        return false;
    };
    let sheet = app.sheet;
    let Some(index) = app.wb.range_awaiting_password(sheet, &[*rect]) else {
        return false;
    };
    app.ext_mut::<Memory>().unlock = Some((sheet, index));
    cx.notify();
    true
}

/// Open Unlock Range when a password was found wanted.
pub(super) fn ask_pending(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some((sheet, index)) = app.ext::<Memory>().and_then(|memory| memory.unlock) else {
        return;
    };
    app.ext_mut::<Memory>().unlock = None;
    // Not while the window is being drawn.
    cx.spawn_in(window, async move |this, cx| {
        this.update_in(cx, |app, window, cx| {
            open_unlock(app, sheet, index, window, cx);
        })
        .ok();
    })
    .detach();
}

pub(super) fn open_unlock(app: &mut ExcelApp, sheet: usize, index: usize, window: &mut Window, cx: &mut Cx) -> Entity<UnlockRange> {
    app.open_modal(window, cx, move |modal, _, cx| UnlockRange {
        modal,
        sheet,
        index,
        password: password_field(cx),
    })
}

pub(crate) struct UnlockRange {
    modal: Modal,
    sheet: usize,
    index: usize,
    password: Entity<TextInput>,
}

impl UnlockRange {
    #[cfg(test)]
    pub(super) fn set_password(&mut self, text: &str, cx: &mut Context<Self>) {
        self.password.update(cx, |field, cx| field.set_text(text, cx));
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let password = self.password.read(cx).text().to_string();
        let (sheet, index) = (self.sheet, self.index);
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            if let Err(error) = app.wb.unlock_range(sheet, index, &password) {
                app.alert(Alert::new("OpenSheet", error.message(app.wb.locale)).warning(), cx);
            }
            app.refresh(cx);
        });
    }
}

impl Render for UnlockRange {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        dialog(&self.modal, t("Unlock Range", "Déverrouiller la plage"))
            .width(340.0)
            .on_ok(cx.listener(Self::ok))
            .child(div().line_height(px(17.)).child(t(
                "A cell you are trying to change is password protected.",
                "Une cellule que vous essayez de modifier est protégée par mot de passe.",
            )))
            .child(label(t("Enter the password to change this cell:", "Entrez le mot de passe pour modifier cette cellule :")))
            .child(div().flex().child(TextField::new(&self.password).width(300.0)))
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}
