//! Protection of sheets and of the workbook structure: the dialogs, and
//! refusing what a protected sheet or workbook does not allow.
//!
//! Every change the application makes is submitted to `guard` as an
//! `Intent`; this module turns it into the engine's question
//! (`Workbook::permits`) and answers with Excel's message. Selections are
//! held to what the sheet lets its users select, and Tab walks the
//! unlocked cells.

use super::{Command, ContextMenu, Cx, Feature, Intent, MenuSection, MenuTarget};
use crate::app::{ExcelApp, MenuEntry};
use crate::grid::Selection;
use crate::i18n::t;
use crate::shell::{Alert, Modal};
use crate::text_input::TextInput;
use crate::theme::{self, c};
use crate::widgets::{self, Button, Checkbox, GroupBox, ROW_H, TextField, Tri, dialog, label};
use gpui::{App, Context, Entity, FocusHandle, KeyDownEvent, SharedString, Window, div, prelude::*, px};
use opensheet_engine::ops::Direction;
use opensheet_engine::protect::{Action, Denied, PasswordError, Permission, SheetProtection};
use opensheet_engine::{Locale, MAX_ROWS, Workbook};
use std::collections::HashMap;
use std::rc::Rc;

pub struct Protection;

commands! {
    /// What the user can ask of protection.
    pub enum Cmd {
        ProtectSheet = "lock", t("Protect Sheet", "Protéger la feuille");
        ProtectWorkbook = "lock", t("Protect Workbook", "Protéger le classeur");
        AllowEditRanges = "lock", t("Allow Edit Ranges", "Autoriser la modification des plages");
        UnshareWorkbook = "share", t("Unshare Workbook", "Annuler le partage du classeur");
        LockCell = "lock", t("Lock Cell", "Verrouiller les cellules");
    }
}

impl Command for Cmd {
    fn label(self) -> &'static str {
        self.face().1
    }

    /// The button that protects a sheet is the one that unprotects it.
    fn label_in(self, app: &ExcelApp) -> &'static str {
        match self {
            Cmd::ProtectSheet if app.wb.is_sheet_protected(app.sheet) => {
                t("Unprotect Sheet", "Ôter la protection de la feuille")
            }
            _ => self.label(),
        }
    }

    fn icon(self) -> &'static str {
        self.face().0
    }

    fn checked(self, app: &ExcelApp) -> bool {
        match self {
            Cmd::LockCell => app.active_style().locked,
            // Excel shows the button pressed while the workbook is protected.
            Cmd::ProtectWorkbook => app.wb.protection.is_protected(),
            _ => false,
        }
    }

    fn enabled(self, app: &ExcelApp) -> bool {
        match self {
            // A workbook is never shared here.
            Cmd::UnshareWorkbook => false,
            // Neither can be changed while the sheet is protected.
            Cmd::LockCell | Cmd::AllowEditRanges => !app.wb.is_sheet_protected(app.sheet),
            _ => true,
        }
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        if app.is_editing() && !app.commit_edit(None, window, cx) {
            return;
        }
        match self {
            Cmd::ProtectSheet => {
                let sheet = app.sheet;
                if !app.wb.is_sheet_protected(sheet) {
                    // The dialog proposes what the sheet allowed last time.
                    let name = &app.wb.sheets[sheet].name;
                    let options = app
                        .ext::<Memory>()
                        .and_then(|memory| memory.options.get(name).cloned())
                        .unwrap_or_default();
                    let locale = app.wb.locale;
                    app.open_modal(window, cx, move |modal, _, cx| {
                        ProtectSheet::new(modal, sheet, options, locale, cx)
                    });
                } else if app.wb.sheet_needs_password(sheet) {
                    app.open_modal(window, cx, move |modal, _, cx| Unprotect::new(modal, Target::Sheet(sheet), cx));
                } else {
                    let _ = unprotect(app, Target::Sheet(sheet), "", cx);
                }
            }
            Cmd::ProtectWorkbook => {
                if !app.wb.protection.is_protected() {
                    app.open_modal(window, cx, ProtectWorkbook::new);
                } else if app.wb.workbook_needs_password() {
                    app.open_modal(window, cx, |modal, _, cx| Unprotect::new(modal, Target::Workbook, cx));
                } else {
                    let _ = unprotect(app, Target::Workbook, "", cx);
                }
            }
            Cmd::LockCell => {
                let locked = !app.active_style().locked;
                if app.may(Intent::ChangeSheet, cx) {
                    app.apply_style(self.label(), cx, move |s| s.locked = locked);
                }
            }
            Cmd::AllowEditRanges | Cmd::UnshareWorkbook => app.not_implemented(self.label(), cx),
        }
    }
}

/// What this module remembers while a workbook is open.
#[derive(Default)]
struct Memory {
    /// What each sheet (by name) allowed when it was last protected: the
    /// Protect Sheet dialog proposes it again.
    options: HashMap<String, SheetProtection>,
    /// The last selection the protected sheet allowed (sheet, selection),
    /// put back when one it does not allow is made.
    selection: Option<(usize, Selection)>,
}

// --------------------------------------------------------------- the guard

/// The engine's answer to a change the application is about to make on
/// `sheet`. `object_locked`: whether the object an `EditObject` is about
/// is locked.
fn verdict(wb: &Workbook, sheet: usize, intent: &Intent, object_locked: bool) -> Result<(), Denied> {
    let action = match *intent {
        Intent::EditCells(rect) => {
            return wb.permits(&Action::EditCells {
                sheet,
                ranges: &[rect],
            });
        }
        Intent::FormatCells(_) => Action::FormatCells { sheet },
        Intent::FormatColumns => Action::FormatColumns { sheet },
        Intent::FormatRows => Action::FormatRows { sheet },
        Intent::InsertColumns { .. } => Action::InsertColumns { sheet },
        Intent::InsertRows { .. } => Action::InsertRows { sheet },
        Intent::DeleteColumns { first, last } => Action::DeleteColumns { sheet, first, last },
        Intent::DeleteRows { first, last } => Action::DeleteRows { sheet, first, last },
        Intent::Sort(range) => Action::Sort { sheet, range },
        Intent::UseAutoFilter => Action::UseAutoFilter { sheet },
        Intent::ToggleAutoFilter => Action::ToggleAutoFilter { sheet },
        Intent::UsePivotTable => Action::UsePivotTable { sheet },
        Intent::EditObject(_) => Action::EditObject {
            sheet,
            locked: object_locked,
        },
        Intent::EditComment => Action::EditComment { sheet },
        Intent::ChangeTable => Action::ChangeTable { sheet },
        Intent::ChangeSheet => Action::ChangeSheet { sheet },
        // The tab belongs to the sheet and to the structure of the workbook.
        Intent::TabColor => {
            wb.permits(&Action::ChangeSheet { sheet })?;
            Action::RenameSheet
        }
        Intent::InsertSheet => Action::InsertSheet,
        Intent::DeleteSheet => Action::DeleteSheet,
        Intent::RenameSheet => Action::RenameSheet,
        Intent::MoveSheet => Action::MoveSheet,
        Intent::CopySheet => Action::CopySheet,
        Intent::HideSheet => Action::HideSheet,
        Intent::UnhideSheet => Action::UnhideSheet,
    };
    wb.permits(&action)
}

// ------------------------------------------------- moving on a protected sheet

/// How far a walk along the cells of a protected sheet looks for the next
/// one before giving up.
const WALK_LIMIT: u64 = 200_000;

/// Where a key leads from `from` on a protected sheet when that is not
/// simply the neighbouring cell: Tab (an `entry` going left or right)
/// goes to the next unlocked cell, and so does every key when only
/// unlocked cells can be selected.
fn next_cell(wb: &Workbook, sheet: usize, from: (u32, u32), dir: Direction, entry: bool) -> Option<(u32, u32)> {
    let protection = wb.sheets.get(sheet)?.protection.as_ref()?;
    let horizontal = matches!(dir, Direction::Left | Direction::Right);
    let forward = matches!(dir, Direction::Right | Direction::Down);
    let confined =
        !protection.allows(Permission::SelectLockedCells) && protection.allows(Permission::SelectUnlockedCells);
    if horizontal && (entry || confined) {
        walk_unlocked(wb, sheet, from, forward)
    } else if confined {
        walk_column(wb, sheet, from, forward)
    } else {
        None
    }
}

/// The next unlocked cell in reading order that is shown and is not the
/// merged cell `from` belongs to.
fn walk_unlocked(wb: &Workbook, sheet: usize, from: (u32, u32), forward: bool) -> Option<(u32, u32)> {
    let sh = &wb.sheets[sheet];
    let home = sh.merge_at(from.0, from.1);
    let mut at = from;
    for _ in 0..WALK_LIMIT {
        at = wb.next_unlocked_cell(sheet, at, forward)?;
        if at == from {
            return None;
        }
        let hidden = sh.is_row_hidden(at.0) || sh.hidden_cols.contains(&at.1);
        if !hidden && !home.is_some_and(|merge| merge.contains(at.0, at.1)) {
            return Some(at);
        }
    }
    None
}

/// The next unlocked cell down (or up) the column of `from`, going round
/// the column.
fn walk_column(wb: &Workbook, sheet: usize, from: (u32, u32), down: bool) -> Option<(u32, u32)> {
    let sh = &wb.sheets[sheet];
    let (row, col) = from;
    // Below the last cell and the last formatted row, only an unlocked
    // column has unlocked cells.
    let column_open = sh.col_styles.get(&col).is_some_and(|id| !wb.styles.get(*id).locked);
    let last = if column_open {
        MAX_ROWS - 1
    } else {
        let styled = sh.row_styles.keys().next_back().copied().unwrap_or(0);
        sh.cells.keys().next_back().map_or(0, |(row, _)| *row).max(styled)
    };
    let count = last as u64 + 1;
    (1..count.min(WALK_LIMIT))
        .map(|step| {
            let at = if down { row as u64 + step } else { row as u64 + count - step % count };
            (at % count) as u32
        })
        .find(|at| !wb.style_at(sheet, *at, col).locked && !sh.is_row_hidden(*at))
        .map(|at| (at, col))
}

/// The first cell a protected sheet lets its users select, for when the
/// active cell is not one.
fn first_selectable(wb: &Workbook, sheet: usize) -> Option<(u32, u32)> {
    if wb.can_select_cell(sheet, 0, 0) {
        return Some((0, 0));
    }
    wb.next_unlocked_cell(sheet, (0, 0), true)
        .filter(|(row, col)| wb.can_select_cell(sheet, *row, *col))
}

fn selection_allowed(wb: &Workbook, sheet: usize, selection: &Selection) -> bool {
    selection.ranges.iter().all(|range| wb.can_select_range(sheet, *range))
}

/// A selection the current sheet does not allow gives way to the active
/// cell, or to the first cell that may be selected.
fn settle_selection(app: &mut ExcelApp) {
    let sheet = app.sheet;
    if selection_allowed(&app.wb, sheet, app.sel()) {
        return;
    }
    let (row, col) = app.active();
    let target = if app.wb.can_select_cell(sheet, row, col) {
        Some((row, col))
    } else {
        first_selectable(&app.wb, sheet)
    };
    if let Some((row, col)) = target {
        app.views[sheet].sel = Selection::single(row, col);
    }
}

impl Feature for Protection {
    fn name(&self) -> &'static str {
        "protection"
    }

    fn guard(&self, app: &ExcelApp, intent: &Intent) -> Option<SharedString> {
        // Charts are always locked; pictures and shapes say for themselves.
        let object_locked = match intent {
            Intent::EditObject(Some(id)) if id.owner == super::index_of(&super::drawings::Drawings) => {
                app.wb.drawing(app.sheet, id.id).is_none_or(|drawing| drawing.locked)
            }
            _ => true,
        };
        verdict(&app.wb, app.sheet, intent, object_locked)
            .err()
            .map(|denied| denied.message(app.wb.locale).into())
    }

    fn can_select(&self, app: &ExcelApp, row: u32, col: u32) -> bool {
        app.wb.can_select_cell(app.sheet, row, col)
    }

    fn next_cell(&self, app: &ExcelApp, from: (u32, u32), dir: Direction, entry: bool) -> Option<(u32, u32)> {
        next_cell(&app.wb, app.sheet, from, dir, entry)
    }

    /// Selections made without going through a cell (a drag, a header, the
    /// name box, Select All) are checked once made.
    fn selection_changed(&self, app: &mut ExcelApp, cx: &mut Cx) {
        let sheet = app.sheet;
        if !app.wb.is_sheet_protected(sheet) {
            return;
        }
        if selection_allowed(&app.wb, sheet, app.sel()) {
            let selection = app.sel().clone();
            app.ext_mut::<Memory>().selection = Some((sheet, selection));
            return;
        }
        let previous = app
            .ext::<Memory>()
            .and_then(|memory| memory.selection.clone())
            .filter(|(of, selection)| *of == sheet && selection_allowed(&app.wb, sheet, selection));
        match previous {
            Some((_, selection)) => app.views[sheet].sel = selection,
            None => settle_selection(app),
        }
        app.refresh(cx);
    }

    fn context_menu(&self, app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
        if matches!(target, MenuTarget::SheetTab(_)) {
            menu.add(MenuSection::Format, MenuEntry::command(Cmd::ProtectSheet, app));
        }
    }
}

// ------------------------------------------------- protecting and unprotecting

/// What a password protects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Sheet(usize),
    Workbook,
}

/// A protection ready to be applied, once its password is confirmed.
#[derive(Clone, Debug)]
enum Pending {
    Sheet { sheet: usize, options: SheetProtection },
    Workbook { structure: bool, windows: bool },
}

impl Pending {
    fn apply(&self, app: &mut ExcelApp, password: &str, cx: &mut Cx) {
        match self {
            Pending::Sheet { sheet, options } => {
                app.wb.protect_sheet(*sheet, options.clone(), password);
                settle_selection(app);
            }
            Pending::Workbook { structure, windows } => app.wb.protect_workbook(*structure, *windows, password),
        }
        app.refresh(cx);
    }
}

/// Remove a protection; the sheet's options are kept for the next time it
/// is protected.
fn unprotect(app: &mut ExcelApp, target: Target, password: &str, cx: &mut Cx) -> Result<(), PasswordError> {
    match target {
        Target::Sheet(sheet) => {
            let before = app.wb.sheets.get(sheet).and_then(|sh| sh.protection.clone());
            app.wb.unprotect_sheet(sheet, password)?;
            if let Some(mut options) = before {
                options.password = None;
                let name = app.wb.sheets[sheet].name.clone();
                app.ext_mut::<Memory>().options.insert(name, options);
            }
        }
        Target::Workbook => app.wb.unprotect_workbook(password)?,
    }
    app.refresh(cx);
    Ok(())
}

/// A field in which a password is typed.
fn password_field<V: 'static>(cx: &mut Context<V>) -> Entity<TextInput> {
    let field = widgets::input(cx, "");
    field.update(cx, |field, _| field.masked = true);
    field
}

/// OK (greyed out while `enabled` is false) and Cancel.
fn buttons(modal: &Modal, enabled: bool, on_ok: impl Fn(&(), &mut Window, &mut App) + 'static) -> impl IntoElement {
    let modal = modal.clone();
    div()
        .flex()
        .gap(px(8.))
        .child(Button::new("ok", t("OK", "OK")).primary().disabled(!enabled).on_click(on_ok))
        .child(Button::new("cancel", t("Cancel", "Annuler")).on_click(move |_, window, cx| modal.close(window, cx)))
}

/// Tick or untick a line of the Protect Sheet dialog. As in Excel, locked
/// cells cannot be selectable when unlocked ones are not.
fn toggle_permission(options: &mut SheetProtection, permission: Permission) {
    let allowed = !options.allows(permission);
    options.set(permission, allowed);
    match permission {
        Permission::SelectLockedCells if allowed => options.set(Permission::SelectUnlockedCells, true),
        Permission::SelectUnlockedCells if !allowed => options.set(Permission::SelectLockedCells, false),
        _ => {}
    }
}

/// Lines of the permission list shown at a time.
const LIST_ROWS: usize = 8;

/// The Protect Sheet dialog.
struct ProtectSheet {
    modal: Modal,
    sheet: usize,
    locale: &'static Locale,
    password: Entity<TextInput>,
    /// "Protect worksheet and contents of locked cells".
    protect: bool,
    options: SheetProtection,
    /// Keyboard focus, cursor and scroll position of the permission list.
    list_focus: FocusHandle,
    cursor: usize,
    scroll: Entity<widgets::ScrollState>,
}

impl ProtectSheet {
    fn new(modal: Modal, sheet: usize, options: SheetProtection, locale: &'static Locale, cx: &mut Context<Self>) -> Self {
        ProtectSheet {
            modal,
            sheet,
            locale,
            password: password_field(cx),
            protect: true,
            options,
            list_focus: cx.focus_handle().tab_stop(true),
            cursor: 0,
            scroll: cx.new(|_| widgets::ScrollState::new()),
        }
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        if !self.protect {
            return;
        }
        let pending = Pending::Sheet {
            sheet: self.sheet,
            options: self.options.clone(),
        };
        confirm_then_apply(&self.modal, pending, &self.password, window, cx);
    }

    fn toggle(&mut self, row: usize, cx: &mut Context<Self>) {
        if !self.protect {
            return;
        }
        self.cursor = row;
        toggle_permission(&mut self.options, Permission::ALL[row]);
        cx.notify();
    }

    fn list_key(&mut self, ev: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if !widgets::plain(ev) {
            return;
        }
        let last = Permission::ALL.len() - 1;
        let next = match ev.keystroke.key.as_str() {
            "space" => {
                self.toggle(self.cursor, cx);
                cx.stop_propagation();
                return;
            }
            "up" => self.cursor.saturating_sub(1),
            "down" => (self.cursor + 1).min(last),
            "pageup" => self.cursor.saturating_sub(LIST_ROWS),
            "pagedown" => (self.cursor + LIST_ROWS).min(last),
            "home" => 0,
            "end" => last,
            _ => return,
        };
        self.cursor = next;
        let height = LIST_ROWS as f32 * ROW_H;
        self.scroll.update(cx, |scroll, _| scroll.reveal(next, next, ROW_H, height));
        cx.stop_propagation();
        cx.notify();
    }

    /// The check-list of what the users of the sheet may do.
    fn permission_list(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let locale = self.locale;
        let focused = self.list_focus.is_focused(window);
        let (cursor, enabled) = (self.cursor, self.protect);
        let options = self.options.clone();
        let view = cx.entity().downgrade();
        let toggle: Rc<dyn Fn(usize, &mut App)> = Rc::new(move |row, cx| {
            view.update(cx, |this, cx| this.toggle(row, cx)).ok();
        });
        let height = LIST_ROWS as f32 * ROW_H;
        let rows = widgets::virtual_rows(&self.scroll, Permission::ALL.len(), ROW_H, height, cx, |row| {
            let permission = Permission::ALL[row];
            let toggle = toggle.clone();
            div()
                .id(("permission", row))
                .size_full()
                .px(px(6.))
                .flex()
                .items_center()
                .gap(px(6.))
                .when(focused && row == cursor, |d| d.bg(c(widgets::SELECTED_BG)))
                .when(!(focused && row == cursor) && enabled, |d| d.hover(|s| s.bg(c(0xF5F5F5))))
                .when(!enabled, |d| d.text_color(c(theme::TEXT_DISABLED)))
                .child(widgets::check_mark(Tri::from(options.allows(permission)), !enabled))
                .child(div().truncate().child(permission.label(locale)))
                .on_click(move |_, _, cx| toggle(row, cx))
                .into_any_element()
        });
        div()
            .id("permissions")
            .track_focus(&self.list_focus)
            .bg(c(0xFFFFFF))
            .border_1()
            .border_color(c(if focused { theme::ACCENT } else { 0xABABAB }))
            .rounded(px(3.))
            .overflow_hidden()
            .on_key_down(cx.listener(Self::list_key))
            .child(rows)
    }
}

impl Render for ProtectSheet {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let list = self.permission_list(window, cx);
        dialog(&self.modal, t("Protect Sheet", "Protéger la feuille"))
            .width(300.0)
            .on_ok(cx.listener(Self::ok))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(label(t(
                        "Password to unprotect sheet:",
                        "Mot de passe pour ôter la protection de la feuille :",
                    )))
                    .child(div().flex().child(TextField::new(&self.password))),
            )
            .child(
                Checkbox::new(
                    "protect",
                    t(
                        "Protect worksheet and contents of locked cells",
                        "Protéger la feuille et le contenu des cellules verrouillées",
                    ),
                    self.protect,
                )
                .on_change(cx.listener(|this, on: &bool, _, cx| {
                    this.protect = *on;
                    cx.notify();
                })),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(label(t(
                        "Allow all users of this worksheet to:",
                        "Autoriser tous les utilisateurs de cette feuille à :",
                    )))
                    .child(list),
            )
            // Unticked, the dialog would protect nothing.
            .footer(buttons(&self.modal, self.protect, cx.listener(Self::ok)))
    }
}

/// The Protect Structure and Windows dialog.
struct ProtectWorkbook {
    modal: Modal,
    structure: bool,
    windows: bool,
    password: Entity<TextInput>,
}

impl ProtectWorkbook {
    fn new(modal: Modal, _: &mut Window, cx: &mut Context<Self>) -> Self {
        ProtectWorkbook {
            modal,
            structure: true,
            windows: false,
            password: password_field(cx),
        }
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        if !self.structure && !self.windows {
            return;
        }
        let pending = Pending::Workbook {
            structure: self.structure,
            windows: self.windows,
        };
        confirm_then_apply(&self.modal, pending, &self.password, window, cx);
    }
}

impl Render for ProtectWorkbook {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        dialog(
            &self.modal,
            t("Protect Structure and Windows", "Protéger la structure et les fenêtres"),
        )
        .width(280.0)
        .on_ok(cx.listener(Self::ok))
        .child(
            GroupBox::new(t("Protect workbook for", "Protéger le classeur pour"))
                .child(
                    Checkbox::new("structure", t("Structure", "Structure"), self.structure).on_change(cx.listener(
                        |this, on: &bool, _, cx| {
                            this.structure = *on;
                            cx.notify();
                        },
                    )),
                )
                .child(
                    Checkbox::new("windows", t("Windows", "Fenêtres"), self.windows).on_change(cx.listener(
                        |this, on: &bool, _, cx| {
                            this.windows = *on;
                            cx.notify();
                        },
                    )),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(label(t("Password (optional):", "Mot de passe (facultatif) :")))
                .child(div().flex().child(TextField::new(&self.password))),
        )
        .footer(buttons(
            &self.modal,
            self.structure || self.windows,
            cx.listener(Self::ok),
        ))
    }
}

/// What OK does in the two Protect dialogs: protect at once without a
/// password, else ask for the password a second time first.
fn confirm_then_apply<V: 'static>(
    parent: &Modal,
    pending: Pending,
    password: &Entity<TextInput>,
    window: &mut Window,
    cx: &mut Context<V>,
) {
    let password = password.read(cx).text().to_string();
    if password.is_empty() {
        parent.update_app(cx, |app, cx| pending.apply(app, "", cx));
        parent.close(window, cx);
        return;
    }
    let dialog = parent.clone();
    parent.update_app(cx, |app, cx| {
        app.open_modal(window, cx, move |modal, _, cx| ConfirmPassword {
            modal,
            parent: dialog,
            pending,
            password,
            again: password_field(cx),
        });
    });
}

/// The Confirm Password dialog, over a Protect dialog.
struct ConfirmPassword {
    modal: Modal,
    /// The dialog the password was typed in.
    parent: Modal,
    pending: Pending,
    password: String,
    again: Entity<TextInput>,
}

impl ConfirmPassword {
    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        if self.again.read(cx).text() != self.password {
            // Back to the Protect dialog, as in Excel.
            self.modal.close(window, cx);
            self.modal.update_app(cx, |app, cx| {
                let message = PasswordError::Mismatch.message(app.wb.locale);
                app.alert(Alert::new("OpenSheet", message).warning(), cx);
            });
            return;
        }
        let (pending, password) = (self.pending.clone(), self.password.clone());
        self.modal.update_app(cx, |app, cx| pending.apply(app, &password, cx));
        self.parent.close(window, cx);
    }
}

impl Render for ConfirmPassword {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        dialog(&self.modal, t("Confirm Password", "Confirmer le mot de passe"))
            .width(340.0)
            .on_ok(cx.listener(Self::ok))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(label(t(
                        "Reenter password to proceed.",
                        "Entrez de nouveau le mot de passe pour continuer.",
                    )))
                    .child(div().flex().child(TextField::new(&self.again))),
            )
            .child(div().line_height(px(17.)).child(t(
                "Caution: If you lose or forget the password, it cannot be recovered. It is advisable to keep a list of passwords and their corresponding workbook and sheet names in a safe place. (Remember that passwords are case-sensitive.)",
                "Attention : si vous oubliez ou perdez votre mot de passe, il sera impossible de le récupérer. Il est recommandé de conserver en lieu sûr une liste des mots de passe et des noms de feuilles et classeurs correspondants (n'oubliez pas que les mots de passe respectent la casse).",
            )))
            .footer(buttons(&self.modal, true, cx.listener(Self::ok)))
    }
}

/// The Unprotect Sheet / Unprotect Workbook dialog: the password.
struct Unprotect {
    modal: Modal,
    target: Target,
    password: Entity<TextInput>,
}

impl Unprotect {
    fn new(modal: Modal, target: Target, cx: &mut Context<Self>) -> Self {
        Unprotect {
            modal,
            target,
            password: password_field(cx),
        }
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let password = self.password.read(cx).text().to_string();
        let target = self.target;
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            if let Err(error) = unprotect(app, target, &password, cx) {
                app.alert(Alert::new("OpenSheet", error.message(app.wb.locale)).warning(), cx);
            }
        });
    }
}

impl Render for Unprotect {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = match self.target {
            Target::Sheet(_) => t("Unprotect Sheet", "Ôter la protection de la feuille"),
            Target::Workbook => t("Unprotect Workbook", "Ôter la protection du classeur"),
        };
        dialog(&self.modal, title)
            .width(300.0)
            .on_ok(cx.listener(Self::ok))
            .child(widgets::labeled(
                t("Password:", "Mot de passe :"),
                TextField::new(&self.password),
            ))
            .footer(buttons(&self.modal, true, cx.listener(Self::ok)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::Rect;

    fn rect(name: &str) -> Rect {
        Rect::parse(name).unwrap()
    }

    fn cell(name: &str) -> (u32, u32) {
        opensheet_engine::refs::parse_cell_name(name).unwrap()
    }

    /// A sheet whose cells B2:C3 and whole column E are unlocked.
    fn form() -> Workbook {
        let mut wb = Workbook::new(Locale::en());
        wb.set_cell_text(0, 0, 0, "Title").unwrap();
        wb.update_style(0, rect("B2:C3"), &|s| s.locked = false);
        wb.update_style(0, rect("E:E"), &|s| s.locked = false);
        wb.commit("format");
        wb
    }

    /// Every intent the application can submit.
    fn intents() -> Vec<Intent> {
        vec![
            Intent::EditCells(rect("A1")),
            Intent::FormatCells(rect("A1")),
            Intent::FormatColumns,
            Intent::FormatRows,
            Intent::InsertColumns { at: 0, count: 1 },
            Intent::InsertRows { at: 0, count: 1 },
            Intent::DeleteColumns { first: 0, last: 0 },
            Intent::DeleteRows { first: 0, last: 0 },
            Intent::Sort(rect("A1:A3")),
            Intent::UseAutoFilter,
            Intent::ToggleAutoFilter,
            Intent::UsePivotTable,
            Intent::EditObject(None),
            Intent::EditComment,
            Intent::ChangeTable,
            Intent::ChangeSheet,
            Intent::TabColor,
            Intent::InsertSheet,
            Intent::DeleteSheet,
            Intent::RenameSheet,
            Intent::MoveSheet,
            Intent::CopySheet,
            Intent::HideSheet,
            Intent::UnhideSheet,
        ]
    }

    #[test]
    fn nothing_is_refused_without_protection() {
        let wb = form();
        for intent in intents() {
            assert_eq!(verdict(&wb, 0, &intent, true), Ok(()), "{intent:?}");
        }
    }

    #[test]
    fn a_protected_sheet_refuses_all_but_its_structure() {
        let mut wb = form();
        wb.protect_sheet(0, SheetProtection::default(), "");
        let structure = [
            Intent::InsertSheet,
            Intent::DeleteSheet,
            Intent::RenameSheet,
            Intent::MoveSheet,
            Intent::CopySheet,
            Intent::HideSheet,
            Intent::UnhideSheet,
        ];
        for intent in intents() {
            let answer = verdict(&wb, 0, &intent, true);
            if structure.contains(&intent) {
                assert_eq!(answer, Ok(()), "{intent:?}");
            } else {
                assert!(answer.is_err(), "{intent:?}");
            }
        }
        // Each refusal has the message of its kind.
        assert_eq!(verdict(&wb, 0, &Intent::EditCells(rect("A1")), true), Err(Denied::Sheet));
        assert_eq!(verdict(&wb, 0, &Intent::UseAutoFilter, true), Err(Denied::Filter));
        assert_eq!(verdict(&wb, 0, &Intent::EditComment, true), Err(Denied::Comment));
        assert_eq!(verdict(&wb, 0, &Intent::ChangeTable, true), Err(Denied::Table));
        assert_eq!(verdict(&wb, 0, &Intent::TabColor, true), Err(Denied::Sheet));
        // Unlocked cells take entries, locked ones beside them do not.
        assert_eq!(verdict(&wb, 0, &Intent::EditCells(rect("B2:C3")), true), Ok(()));
        assert_eq!(verdict(&wb, 0, &Intent::EditCells(rect("E7")), true), Ok(()));
        assert_eq!(
            verdict(&wb, 0, &Intent::EditCells(rect("B2:C4")), true),
            Err(Denied::Sheet)
        );
        // An unlocked picture can be changed, a locked one or a new one not.
        assert_eq!(verdict(&wb, 0, &Intent::EditObject(None), false), Ok(()));
        assert_eq!(verdict(&wb, 0, &Intent::EditObject(None), true), Err(Denied::Sheet));
        // The other sheets are not concerned.
        let other = wb.add_sheet(1);
        assert_eq!(verdict(&wb, other, &Intent::EditCells(rect("A1")), true), Ok(()));
    }

    #[test]
    fn permissions_open_what_they_name() {
        let mut wb = form();
        let all = SheetProtection::allowing(&Permission::ALL);
        wb.protect_sheet(0, all, "");
        let open = [
            Intent::FormatCells(rect("A1")),
            Intent::FormatColumns,
            Intent::FormatRows,
            Intent::InsertColumns { at: 0, count: 1 },
            Intent::InsertRows { at: 0, count: 1 },
            Intent::UseAutoFilter,
            Intent::UsePivotTable,
            Intent::EditObject(None),
            Intent::EditComment,
        ];
        for intent in open {
            assert_eq!(verdict(&wb, 0, &intent, true), Ok(()), "{intent:?}");
        }
        // Locked cells are never edited, sorted or deleted.
        assert_eq!(verdict(&wb, 0, &Intent::EditCells(rect("A1")), true), Err(Denied::Sheet));
        assert_eq!(verdict(&wb, 0, &Intent::Sort(rect("A1:A3")), true), Err(Denied::Sheet));
        assert_eq!(verdict(&wb, 0, &Intent::Sort(rect("B2:C3")), true), Ok(()));
        assert_eq!(
            verdict(&wb, 0, &Intent::DeleteRows { first: 0, last: 0 }, true),
            Err(Denied::LockedCells { rows: true })
        );
        assert_eq!(
            verdict(&wb, 0, &Intent::DeleteColumns { first: 4, last: 4 }, true),
            Ok(())
        );
        // What no permission opens.
        for intent in [Intent::ToggleAutoFilter, Intent::ChangeTable, Intent::ChangeSheet, Intent::TabColor] {
            assert!(verdict(&wb, 0, &intent, true).is_err(), "{intent:?}");
        }
    }

    #[test]
    fn a_protected_workbook_keeps_its_sheets() {
        let mut wb = form();
        wb.protect_workbook(true, false, "");
        for intent in intents() {
            let structural = matches!(
                intent,
                Intent::TabColor
                    | Intent::InsertSheet
                    | Intent::DeleteSheet
                    | Intent::RenameSheet
                    | Intent::MoveSheet
                    | Intent::CopySheet
                    | Intent::HideSheet
                    | Intent::UnhideSheet
            );
            let expected = if structural { Err(Denied::Workbook) } else { Ok(()) };
            assert_eq!(verdict(&wb, 0, &intent, true), expected, "{intent:?}");
        }
        // Windows alone leave the sheets free.
        wb.protect_workbook(false, true, "");
        assert_eq!(verdict(&wb, 0, &Intent::RenameSheet, true), Ok(()));
    }

    #[test]
    fn tab_goes_to_the_next_unlocked_cell() {
        let mut wb = form();
        let go = |wb: &Workbook, from: &str, dir: Direction, entry: bool| next_cell(wb, 0, cell(from), dir, entry);
        // Not protected: the neighbouring cell.
        assert_eq!(go(&wb, "A1", Direction::Right, true), None);
        wb.protect_sheet(0, SheetProtection::default(), "");
        assert_eq!(go(&wb, "A1", Direction::Right, true), Some(cell("E1")));
        assert_eq!(go(&wb, "E1", Direction::Right, true), Some(cell("B2")));
        assert_eq!(go(&wb, "C2", Direction::Right, true), Some(cell("E2")));
        assert_eq!(go(&wb, "B2", Direction::Left, true), Some(cell("E1")));
        // Enter and the arrows move as usual while locked cells can be selected.
        assert_eq!(go(&wb, "B2", Direction::Down, true), None);
        assert_eq!(go(&wb, "B2", Direction::Right, false), None);
        // Hidden rows are stepped over, and a merged cell counts once.
        wb.set_rows_hidden(0, 1, 1, true);
        assert_eq!(go(&wb, "E1", Direction::Right, true), Some(cell("B3")));
        wb.set_rows_hidden(0, 1, 1, false);
        wb.merge(0, rect("B2:C2"));
        assert_eq!(go(&wb, "B2", Direction::Right, true), Some(cell("E2")));
    }

    #[test]
    fn only_unlocked_cells_within_reach() {
        let mut wb = form();
        wb.protect_sheet(0, SheetProtection::allowing(&[Permission::SelectUnlockedCells]), "");
        let go = |from: &str, dir: Direction| next_cell(&wb, 0, cell(from), dir, false);
        // The arrows walk the unlocked cells too.
        assert_eq!(go("C2", Direction::Right), Some(cell("E2")));
        assert_eq!(go("E2", Direction::Left), Some(cell("C2")));
        assert_eq!(go("B2", Direction::Down), Some(cell("B3")));
        assert_eq!(go("B3", Direction::Down), Some(cell("B2")));
        assert_eq!(go("B2", Direction::Up), Some(cell("B3")));
        assert_eq!(go("E5", Direction::Down), Some(cell("E6")));
        assert_eq!(go("E1", Direction::Up), Some((MAX_ROWS - 1, 4)));
        assert_eq!(first_selectable(&wb, 0), Some(cell("E1")));
        assert!(selection_allowed(&wb, 0, &Selection::single(1, 1)));
        assert!(!selection_allowed(&wb, 0, &Selection::single(0, 0)));
    }

    #[test]
    fn the_dialog_keeps_the_two_select_boxes_consistent() {
        let mut options = SheetProtection::default();
        toggle_permission(&mut options, Permission::SelectUnlockedCells);
        assert!(!options.allows(Permission::SelectUnlockedCells) && !options.allows(Permission::SelectLockedCells));
        toggle_permission(&mut options, Permission::SelectLockedCells);
        assert!(options.allows(Permission::SelectUnlockedCells) && options.allows(Permission::SelectLockedCells));
        toggle_permission(&mut options, Permission::SelectLockedCells);
        assert!(options.allows(Permission::SelectUnlockedCells) && !options.allows(Permission::SelectLockedCells));
        toggle_permission(&mut options, Permission::Sort);
        assert!(options.allows(Permission::Sort));
        toggle_permission(&mut options, Permission::Sort);
        assert!(!options.allows(Permission::Sort));
    }
}
