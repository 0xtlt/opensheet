//! Form controls (the Controls group of the Developer tab): buttons,
//! check boxes, option buttons, combo and list boxes, spin buttons,
//! scroll bars, group boxes and labels floating over the cells.
//!
//! A sheet keeps its controls in `Sheet::controls`, bottom-most first. A
//! control sits like a slicer does: a cell, an offset inside it and a
//! size. What it shows (ticked or not, the choice of a list, the value of
//! a scroll bar) is kept on the control and written to its linked cell
//! when the user works it; when the linked cell says something else, the
//! cell wins ([`Workbook::control_state`]), which is how a control follows
//! its cell without anything having to watch the cells.
//!
//! A control read from a file remembers the pieces of the file it came
//! from ([`ControlSource`]): as long as nothing of it changed they are
//! written back as they were, so that what the engine does not model (the
//! font of a caption, the colours) is not lost on the way.

use crate::ast::{Expr, RefExpr, StructuralEdit};
use crate::refs::Rect;
use crate::value::{ErrorKind, Value};
use crate::workbook::{Sheet, Workbook};
use std::sync::Arc;

/// The kinds of form controls a worksheet can hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ControlKind {
    Button,
    CheckBox,
    OptionButton,
    /// A drop-down list ("Combo Box").
    ComboBox,
    ListBox,
    SpinButton,
    ScrollBar,
    GroupBox,
    Label,
}

impl ControlKind {
    pub const ALL: [ControlKind; 9] = [
        ControlKind::Button,
        ControlKind::CheckBox,
        ControlKind::OptionButton,
        ControlKind::ComboBox,
        ControlKind::ListBox,
        ControlKind::SpinButton,
        ControlKind::ScrollBar,
        ControlKind::GroupBox,
        ControlKind::Label,
    ];

    /// The English word Excel names a new control of this kind with
    /// ("Button 1", "Check Box 2").
    pub fn base_name(self) -> &'static str {
        match self {
            ControlKind::Button => "Button",
            ControlKind::CheckBox => "Check Box",
            ControlKind::OptionButton => "Option Button",
            ControlKind::ComboBox => "Drop Down",
            ControlKind::ListBox => "List Box",
            ControlKind::SpinButton => "Spinner",
            ControlKind::ScrollBar => "Scroll Bar",
            ControlKind::GroupBox => "Group Box",
            ControlKind::Label => "Label",
        }
    }

    /// The control carries a caption.
    pub fn has_text(self) -> bool {
        matches!(
            self,
            ControlKind::Button | ControlKind::CheckBox | ControlKind::OptionButton | ControlKind::GroupBox | ControlKind::Label
        )
    }

    /// The control has a state its linked cell mirrors.
    pub fn has_link(self) -> bool {
        !matches!(self, ControlKind::Button | ControlKind::GroupBox | ControlKind::Label)
    }

    /// The control lists the cells of an input range.
    pub fn has_list(self) -> bool {
        matches!(self, ControlKind::ComboBox | ControlKind::ListBox)
    }

    /// The control holds a number between a minimum and a maximum.
    pub fn has_value(self) -> bool {
        matches!(self, ControlKind::SpinButton | ControlKind::ScrollBar)
    }

    /// The size Excel gives a control inserted with a click.
    pub fn default_size(self) -> (f32, f32) {
        match self {
            ControlKind::Button => (96.0, 30.0),
            ControlKind::CheckBox | ControlKind::OptionButton => (104.0, 22.0),
            ControlKind::ComboBox => (128.0, 22.0),
            ControlKind::ListBox => (128.0, 92.0),
            ControlKind::SpinButton => (21.0, 44.0),
            ControlKind::ScrollBar => (21.0, 128.0),
            ControlKind::GroupBox => (160.0, 110.0),
            ControlKind::Label => (72.0, 20.0),
        }
    }
}

/// What a check box or an option button shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CheckState {
    #[default]
    Unchecked,
    Checked,
    /// Greyed: neither (a check box whose linked cell holds `#N/A`).
    Mixed,
}

/// "Selection type" of a list box.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SelectionType {
    #[default]
    Single,
    Multi,
    Extend,
}

/// The largest number a spin button or a scroll bar holds.
pub const MAX_CONTROL_VALUE: i32 = 30_000;

/// A form control of a sheet.
#[derive(Clone, Debug, PartialEq)]
pub struct FormControl {
    /// Unique in the workbook.
    pub id: u32,
    pub kind: ControlKind,
    /// The name of the object ("Button 1"), unique on its sheet.
    pub name: String,
    /// Cell holding the top-left corner (row, col), the offset of the
    /// corner inside it and the size, in pixels at 100 %.
    pub anchor: (u32, u32),
    pub offset: (f32, f32),
    pub size: (f32, f32),
    /// "Move and size with cells" is both; "Move but don't size" the
    /// first; "Don't move or size" neither. Kept for the file: on the
    /// sheet a control moves with its cell and keeps its size.
    pub move_with_cells: bool,
    pub size_with_cells: bool,
    /// The caption.
    pub text: String,
    /// The macro assigned to the control, as files name it
    /// (`[0]!Macro1`).
    pub macro_name: Option<String>,
    /// "Cell link".
    pub link: Option<RefExpr>,
    /// "Input range" of a combo or list box.
    pub input: Option<RefExpr>,
    pub checked: CheckState,
    /// The first option button of its group, as files mark it.
    pub first_button: bool,
    /// The choice of a combo or list box (1 = the first line, 0 = none).
    pub selected: u32,
    /// "Current value", "Minimum value", "Maximum value", "Incremental
    /// change" and "Page change" of a spin button or a scroll bar.
    pub value: i32,
    pub min: i32,
    pub max: i32,
    pub inc: i32,
    pub page: i32,
    /// "Drop down lines" of a combo box.
    pub drop_lines: u32,
    /// "3-D shading".
    pub shaded: bool,
    /// A scroll bar that lies down.
    pub horizontal: bool,
    pub selection: SelectionType,
    /// "Print object".
    pub print: bool,
    /// "Locked" and "Lock text" (Format Control ▸ Protection).
    pub locked: bool,
    pub lock_text: bool,
    /// What the file it was read from says of it.
    pub source: Option<Arc<ControlSource>>,
}

/// The pieces of a file a control was read from, and the control as they
/// describe it: while the control still reads the same, the pieces are
/// written back unchanged.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlSource {
    /// Its `<v:shape>` in the VML drawing of the sheet.
    pub shape: String,
    /// The numeric id the shape has there (`_x0000_s1025` is 1025).
    pub shape_id: u32,
    /// Its `ctrlProp` part; empty when the file has none.
    pub properties: String,
    /// What its `<control>` element in the worksheet holds; empty when
    /// the file has none (a file of Excel 2007).
    pub control: String,
    /// The control as read, with its place measured on the sheet as it
    /// was then (`id` 0, no source).
    pub as_read: Box<FormControl>,
}

impl FormControl {
    /// A control as Insert makes it, to be placed and named by the caller.
    pub fn new(kind: ControlKind) -> FormControl {
        FormControl {
            id: 0,
            kind,
            name: String::new(),
            anchor: (0, 0),
            offset: (0.0, 0.0),
            size: kind.default_size(),
            // Excel: a button moves and sizes with its cells, a check box
            // or an option button only moves, the others do both.
            move_with_cells: true,
            size_with_cells: !matches!(kind, ControlKind::CheckBox | ControlKind::OptionButton | ControlKind::Label),
            text: String::new(),
            macro_name: None,
            link: None,
            input: None,
            checked: CheckState::Unchecked,
            first_button: false,
            selected: 0,
            value: 0,
            min: 0,
            max: if kind == ControlKind::SpinButton { MAX_CONTROL_VALUE } else { 100 },
            inc: 1,
            page: 10,
            drop_lines: 8,
            shaded: false,
            horizontal: false,
            selection: SelectionType::Single,
            print: kind != ControlKind::Button,
            locked: true,
            lock_text: true,
            source: None,
        }
    }

    /// The control without what tells it from another that reads the
    /// same: for comparing it with what a file said.
    pub fn content(&self) -> FormControl {
        FormControl {
            id: 0,
            source: None,
            ..self.clone()
        }
    }

    /// The file pieces of the control, when it still reads as they say.
    /// `now` is the control with its state resolved against the cells
    /// ([`Workbook::control_state`]).
    pub fn unchanged_source(now: &FormControl) -> Option<&ControlSource> {
        let source = now.source.as_deref()?;
        (now.content() == *source.as_read).then_some(source)
    }

    /// The name of the macro without what says where it lives
    /// (`[0]!Macro1`, `Book.xlsm!Macro1` → `Macro1`).
    pub fn macro_short_name(&self) -> Option<&str> {
        let name = self.macro_name.as_deref()?;
        Some(name.rsplit_once('!').map_or(name, |(_, short)| short))
    }

    fn map_refs(&mut self, f: &mut dyn FnMut(&RefExpr) -> Option<RefExpr>) {
        if let Some(link) = &self.link {
            self.link = f(link);
        }
        if let Some(input) = &self.input {
            self.input = f(input);
        }
    }
}

/// Where a spin button or a scroll bar goes from `value` by `delta`,
/// kept between the ends.
pub fn stepped(control: &FormControl, value: i32, delta: i32) -> i32 {
    let (lo, hi) = (control.min.min(control.max), control.min.max(control.max));
    value.saturating_add(delta).clamp(lo, hi)
}

impl Sheet {
    pub fn control(&self, id: u32) -> Option<&FormControl> {
        self.controls.iter().find(|c| c.id == id)
    }

    /// The frame of a control on the sheet: x, y, width, height in pixels
    /// at 100 % from the corner of A1.
    pub fn control_frame(&self, control: &FormControl) -> (f64, f64, f64, f64) {
        let (x, y) = self.corner_point((control.anchor, control.offset));
        (x, y, control.size.0 as f64, control.size.1 as f64)
    }

    /// The option buttons that work together with `id`, in the order of
    /// the sheet: those inside the same group box, or those outside every
    /// group box.
    pub fn option_group(&self, id: u32) -> Vec<u32> {
        let boxes: Vec<(f64, f64, f64, f64)> =
            self.controls.iter().filter(|c| c.kind == ControlKind::GroupBox).map(|c| self.control_frame(c)).collect();
        // The group box around the middle of a control, if any.
        let home = |control: &FormControl| -> Option<usize> {
            let (x, y, w, h) = self.control_frame(control);
            let (cx, cy) = (x + w / 2.0, y + h / 2.0);
            boxes.iter().position(|(bx, by, bw, bh)| cx >= *bx && cx <= bx + bw && cy >= *by && cy <= by + bh)
        };
        let Some(own) = self.control(id).filter(|c| c.kind == ControlKind::OptionButton) else {
            return Vec::new();
        };
        let group = home(own);
        self.controls.iter().filter(|c| c.kind == ControlKind::OptionButton && home(c) == group).map(|c| c.id).collect()
    }

    /// A name no control of the sheet has yet: "Button 3".
    pub fn new_control_name(&self, kind: ControlKind) -> String {
        let taken = |name: &str| self.controls.iter().any(|c| c.name.eq_ignore_ascii_case(name));
        let mut n = self.controls.len() + 1;
        loop {
            let name = format!("{} {n}", kind.base_name());
            if !taken(&name) {
                return name;
            }
            n += 1;
        }
    }
}

impl Workbook {
    /// Every control of the workbook with its sheet.
    pub fn controls(&self) -> impl Iterator<Item = (usize, &FormControl)> {
        self.sheets.iter().enumerate().flat_map(|(si, sheet)| sheet.controls.iter().map(move |c| (si, c)))
    }

    pub fn control(&self, id: u32) -> Option<(usize, &FormControl)> {
        self.controls().find(|(_, c)| c.id == id)
    }

    fn next_control_id(&self) -> u32 {
        self.controls().map(|(_, c)| c.id).max().unwrap_or(0) + 1
    }

    /// The cells a reference of a control on `own_sheet` names.
    fn control_ref(&self, r: &RefExpr, own_sheet: usize) -> Option<(usize, Rect)> {
        let sheet = match &r.sheet {
            Some(name) => self.sheet_index(name)?,
            None => own_sheet,
        };
        Some((sheet, self.clamp_to_used_or_cell(sheet, r.rect())))
    }

    /// Whole rows or columns stop at the cells in use.
    fn clamp_to_used_or_cell(&self, sheet: usize, rect: Rect) -> Rect {
        if rect.is_whole_rows() || rect.is_whole_cols() { self.clamp_to_used(sheet, rect) } else { rect }
    }

    /// The linked cell of a control: (sheet, row, col).
    pub fn control_link(&self, sheet: usize, control: &FormControl) -> Option<(usize, u32, u32)> {
        let (si, rect) = self.control_ref(control.link.as_ref()?, sheet)?;
        Some((si, rect.r0, rect.c0))
    }

    /// The lines of a combo or list box: the cells of its input range as
    /// they are displayed.
    pub fn control_items(&self, sheet: usize, control: &FormControl) -> Vec<String> {
        let Some((si, rect)) = control.input.as_ref().and_then(|input| self.control_ref(input, sheet)) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for row in rect.r0..=rect.r1 {
            for col in rect.c0..=rect.c1 {
                out.push(self.display_text(si, row, col).text);
            }
        }
        out
    }

    /// The control as it shows now: its own state, or what its linked
    /// cell says when the cell holds something the control understands.
    pub fn control_state(&self, sheet: usize, control: &FormControl) -> FormControl {
        let mut now = control.clone();
        let cell = match control.kind {
            // The option buttons of a group share the link of the group.
            ControlKind::OptionButton => self.sheets.get(sheet).and_then(|sh| {
                let group = sh.option_group(control.id);
                let linked = group.iter().filter_map(|id| sh.control(*id)).find(|c| c.link.is_some())?;
                let rank = group.iter().position(|id| *id == control.id)? as f64 + 1.0;
                let (si, row, col) = self.control_link(sheet, linked)?;
                Some((self.value_at(si, row, col), rank))
            }),
            _ => self.control_link(sheet, control).map(|(si, row, col)| (self.value_at(si, row, col), 0.0)),
        };
        let Some((value, rank)) = cell else {
            return now;
        };
        match (control.kind, value) {
            (ControlKind::CheckBox, Value::Bool(on)) => now.checked = if on { CheckState::Checked } else { CheckState::Unchecked },
            (ControlKind::CheckBox, Value::Number(n)) => now.checked = if n != 0.0 { CheckState::Checked } else { CheckState::Unchecked },
            (ControlKind::CheckBox, Value::Error(ErrorKind::NA)) => now.checked = CheckState::Mixed,
            (ControlKind::OptionButton, Value::Number(n)) => now.checked = if n == rank { CheckState::Checked } else { CheckState::Unchecked },
            (ControlKind::ComboBox | ControlKind::ListBox, Value::Number(n)) if n >= 0.0 && n <= u32::MAX as f64 => now.selected = n as u32,
            (ControlKind::SpinButton | ControlKind::ScrollBar, Value::Number(n)) if n.is_finite() => {
                now.value = stepped(control, n.clamp(i32::MIN as f64, i32::MAX as f64) as i32, 0);
            }
            _ => {}
        }
        now
    }

    /// Put a control on a sheet (Developer ▸ Insert): it gets an id and,
    /// without one, a name. The caller has set its place.
    pub fn add_control(&mut self, sheet: usize, mut control: FormControl) -> Option<u32> {
        if sheet >= self.sheets.len() {
            return None;
        }
        self.snapshot();
        control.id = self.next_control_id();
        let sh = &mut self.sheets[sheet];
        if control.name.is_empty() {
            control.name = sh.new_control_name(control.kind);
        }
        if control.text.is_empty() && control.kind.has_text() {
            control.text = control.name.clone();
        }
        if control.kind == ControlKind::OptionButton {
            // The first of its group is the one a new group starts with.
            control.first_button = !sh.controls.iter().any(|c| c.kind == ControlKind::OptionButton);
        }
        let id = control.id;
        sh.controls.push(control);
        Some(id)
    }

    /// Change a control (its place, its caption, its settings); recorded
    /// for undo.
    pub fn update_control(&mut self, id: u32, f: impl FnOnce(&mut FormControl)) -> bool {
        let Some((sheet, _)) = self.control(id) else {
            return false;
        };
        self.snapshot();
        if let Some(control) = self.sheets[sheet].controls.iter_mut().find(|c| c.id == id) {
            f(control);
            let clean = |v: f32| if v.is_finite() { v.max(1.0) } else { 1.0 };
            control.size = (clean(control.size.0), clean(control.size.1));
        }
        true
    }

    pub fn remove_control(&mut self, id: u32) -> bool {
        let Some((sheet, _)) = self.control(id) else {
            return false;
        };
        self.snapshot();
        self.sheets[sheet].controls.retain(|c| c.id != id);
        true
    }

    /// Write a value to the linked cell of a control, if it has one.
    fn write_control_link(&mut self, sheet: usize, control: &FormControl, value: Value) {
        if let Some((si, row, col)) = self.control_link(sheet, control) {
            self.set_value(si, row, col, value);
        }
    }

    /// Tick, untick or grey a check box: the control and its linked cell
    /// (`TRUE`, `FALSE`, `#N/A`). The caller commits.
    pub fn set_control_checked(&mut self, id: u32, state: CheckState) -> bool {
        let Some((sheet, control)) = self.control(id).map(|(si, c)| (si, c.clone())) else {
            return false;
        };
        if control.kind != ControlKind::CheckBox {
            return false;
        }
        self.update_control(id, |c| c.checked = state);
        let value = match state {
            CheckState::Checked => Value::Bool(true),
            CheckState::Unchecked => Value::Bool(false),
            CheckState::Mixed => Value::Error(ErrorKind::NA),
        };
        self.write_control_link(sheet, &control, value);
        true
    }

    /// Choose an option button: the others of its group are released and
    /// the linked cell of the group gets its rank (1 for the first).
    pub fn choose_option(&mut self, id: u32) -> bool {
        let Some((sheet, _)) = self.control(id).filter(|(_, c)| c.kind == ControlKind::OptionButton) else {
            return false;
        };
        let group = self.sheets[sheet].option_group(id);
        let rank = group.iter().position(|member| *member == id).map_or(1.0, |at| at as f64 + 1.0);
        let linked = group.iter().filter_map(|member| self.sheets[sheet].control(*member)).find(|c| c.link.is_some()).cloned();
        self.snapshot();
        for control in self.sheets[sheet].controls.iter_mut().filter(|c| group.contains(&c.id)) {
            control.checked = if control.id == id { CheckState::Checked } else { CheckState::Unchecked };
        }
        if let Some(linked) = linked {
            self.write_control_link(sheet, &linked, Value::Number(rank));
        }
        true
    }

    /// Choose line `selected` (1 = the first) of a combo or list box; the
    /// linked cell gets the number.
    pub fn set_control_selected(&mut self, id: u32, selected: u32) -> bool {
        let Some((sheet, control)) = self.control(id).map(|(si, c)| (si, c.clone())) else {
            return false;
        };
        if !control.kind.has_list() {
            return false;
        }
        self.update_control(id, |c| c.selected = selected);
        // A list box that takes several lines has no single choice to tell.
        if control.selection == SelectionType::Single {
            self.write_control_link(sheet, &control, Value::Number(selected as f64));
        }
        true
    }

    /// Give a spin button or a scroll bar a value (kept between its
    /// ends); the linked cell gets it.
    pub fn set_control_value(&mut self, id: u32, value: i32) -> bool {
        let Some((sheet, control)) = self.control(id).map(|(si, c)| (si, c.clone())) else {
            return false;
        };
        if !control.kind.has_value() {
            return false;
        }
        let value = stepped(&control, value, 0);
        self.update_control(id, |c| c.value = value);
        self.write_control_link(sheet, &control, Value::Number(value as f64));
        true
    }
}

// ------------------------------------------------------------------- hooks

fn shift(pos: u32, insert: bool, at: u32, count: u32) -> u32 {
    if insert {
        if pos >= at { pos + count } else { pos }
    } else if pos >= at + count {
        pos - count
    } else if pos >= at {
        at
    } else {
        pos
    }
}

/// Rows or columns were inserted or deleted: the controls of the edited
/// sheet follow their cells, and every control the cells it names.
pub(crate) fn on_structural_edit(sheet: &mut Sheet, edit: StructuralEdit, edited_sheet: &str) {
    if sheet.controls.is_empty() {
        return;
    }
    let own = sheet.name.clone();
    let here = own.eq_ignore_ascii_case(edited_sheet);
    for control in &mut sheet.controls {
        control.map_refs(&mut |r| match Expr::Ref(r.clone()).adjusted(edit, edited_sheet, &own) {
            Expr::Ref(r) => Some(r),
            _ => None,
        });
        if here && control.move_with_cells {
            match edit {
                StructuralEdit::InsertRows { at, count } => control.anchor.0 = shift(control.anchor.0, true, at, count),
                StructuralEdit::DeleteRows { at, count } => control.anchor.0 = shift(control.anchor.0, false, at, count),
                StructuralEdit::InsertCols { at, count } => control.anchor.1 = shift(control.anchor.1, true, at, count),
                StructuralEdit::DeleteCols { at, count } => control.anchor.1 = shift(control.anchor.1, false, at, count),
            }
        }
    }
}

pub(crate) fn on_sheet_renamed(sheet: &mut Sheet, old: &str, new: &str) {
    for control in &mut sheet.controls {
        control.map_refs(&mut |r| {
            let mut out = r.clone();
            if r.sheet.as_deref().is_some_and(|s| s.eq_ignore_ascii_case(old)) {
                out.sheet = Some(Arc::from(new));
            }
            Some(out)
        });
    }
}

/// The controls of a copied sheet get ids of their own; what they named
/// on the original they name on the copy.
pub(crate) fn on_sheet_duplicated(wb: &mut Workbook, copy: usize, original: &str) {
    let mut next = wb.next_control_id();
    let name = wb.sheets[copy].name.clone();
    for control in &mut wb.sheets[copy].controls {
        control.id = next;
        next += 1;
    }
    on_sheet_renamed(&mut wb.sheets[copy], original, &name);
}

/// The controls that named cells of a deleted sheet lose their link.
pub(crate) fn on_sheet_deleted(wb: &mut Workbook, deleted: &Sheet) {
    for sheet in &mut wb.sheets {
        for control in &mut sheet.controls {
            control.map_refs(&mut |r| match &r.sheet {
                Some(name) if name.eq_ignore_ascii_case(&deleted.name) => None,
                _ => Some(r.clone()),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::locale::Locale;
    use crate::parser::parse_formula;

    fn range(text: &str) -> RefExpr {
        match parse_formula(text, Locale::en()) {
            Ok(Expr::Ref(r)) => r,
            other => panic!("{text}: {other:?}"),
        }
    }

    fn book() -> Workbook {
        let mut wb = Workbook::new(Locale::en());
        for (r, item) in ["Red", "Green", "Blue"].iter().enumerate() {
            wb.set_cell_text(0, r as u32, 3, item).unwrap();
        }
        wb.commit("setup");
        wb
    }

    fn add(wb: &mut Workbook, kind: ControlKind, at: (u32, u32), edit: impl FnOnce(&mut FormControl)) -> u32 {
        let mut control = FormControl::new(kind);
        control.anchor = at;
        edit(&mut control);
        let id = wb.add_control(0, control).unwrap();
        wb.commit("Insert Control");
        id
    }

    #[test]
    fn new_controls_get_ids_names_and_captions() {
        let mut wb = book();
        let button = add(&mut wb, ControlKind::Button, (1, 1), |_| {});
        let check = add(&mut wb, ControlKind::CheckBox, (3, 1), |_| {});
        let (_, b) = wb.control(button).unwrap();
        assert_eq!((b.name.as_str(), b.text.as_str(), b.print), ("Button 1", "Button 1", false));
        let (_, c) = wb.control(check).unwrap();
        assert_eq!((c.name.as_str(), c.text.as_str(), c.print), ("Check Box 2", "Check Box 2", true));
        assert_ne!(button, check);
        assert!(wb.undo());
        assert_eq!(wb.sheets[0].controls.len(), 1);
        // A scroll bar holds 0 to 100, a spin button 0 to 30000; neither has a caption.
        let spin = FormControl::new(ControlKind::SpinButton);
        assert_eq!((spin.min, spin.max, spin.inc), (0, MAX_CONTROL_VALUE, 1));
        let id = add(&mut wb, ControlKind::ScrollBar, (5, 1), |_| {});
        assert_eq!(wb.control(id).unwrap().1.text, "");
    }

    #[test]
    fn a_check_box_writes_its_cell_and_follows_it() {
        let mut wb = book();
        let id = add(&mut wb, ControlKind::CheckBox, (1, 1), |c| c.link = Some(range("$A$1")));
        assert!(wb.set_control_checked(id, CheckState::Checked));
        wb.commit("Check Box");
        assert_eq!(wb.value_at(0, 0, 0), Value::Bool(true));
        // The cell changes: the control shows what the cell says.
        wb.set_cell_text(0, 0, 0, "FALSE").unwrap();
        wb.commit("type");
        let shown = |wb: &Workbook| wb.control_state(0, wb.control(id).unwrap().1).checked;
        assert_eq!(shown(&wb), CheckState::Unchecked);
        wb.set_cell_text(0, 0, 0, "=NA()").unwrap();
        wb.commit("type");
        assert_eq!(shown(&wb), CheckState::Mixed);
        wb.set_cell_text(0, 0, 0, "hello").unwrap();
        wb.commit("type");
        assert_eq!(shown(&wb), CheckState::Checked, "text says nothing: the control keeps its own state");
        // One undo step takes the tick and the cell back together.
        assert!(wb.set_control_checked(id, CheckState::Mixed));
        wb.commit("Check Box");
        assert_eq!(wb.value_at(0, 0, 0), Value::Error(ErrorKind::NA));
        assert!(wb.undo());
        assert_eq!(wb.display_text(0, 0, 0).text, "hello");
        assert_eq!(wb.control(id).unwrap().1.checked, CheckState::Checked);
        // Other kinds are not ticked.
        let button = add(&mut wb, ControlKind::Button, (5, 1), |_| {});
        assert!(!wb.set_control_checked(button, CheckState::Checked));
    }

    #[test]
    fn option_buttons_work_by_group() {
        let mut wb = book();
        let a = add(&mut wb, ControlKind::OptionButton, (1, 1), |c| c.link = Some(range("$A$1")));
        let b = add(&mut wb, ControlKind::OptionButton, (2, 1), |_| {});
        // A group box further down with two buttons of its own.
        let frame = add(&mut wb, ControlKind::GroupBox, (10, 1), |c| c.size = (200.0, 100.0));
        let c = add(&mut wb, ControlKind::OptionButton, (11, 1), |c| c.link = Some(range("$A$2")));
        let d = add(&mut wb, ControlKind::OptionButton, (12, 1), |_| {});
        assert!(wb.control(a).unwrap().1.first_button && !wb.control(b).unwrap().1.first_button);
        assert_eq!(wb.sheets[0].option_group(a), [a, b]);
        assert_eq!(wb.sheets[0].option_group(d), [c, d]);
        assert!(wb.sheets[0].option_group(frame).is_empty());
        assert!(wb.choose_option(b));
        wb.commit("Option Button");
        assert_eq!(wb.value_at(0, 0, 0), Value::Number(2.0));
        assert_eq!(wb.value_at(0, 1, 0), Value::Empty, "the other group is left alone");
        let checked = |wb: &Workbook, id: u32| wb.control_state(0, wb.control(id).unwrap().1).checked == CheckState::Checked;
        assert!(checked(&wb, b) && !checked(&wb, a) && !checked(&wb, c));
        assert!(wb.choose_option(d));
        wb.commit("Option Button");
        assert_eq!(wb.value_at(0, 1, 0), Value::Number(2.0));
        // The cell of the group picks the button.
        wb.set_cell_text(0, 0, 0, "1").unwrap();
        wb.commit("type");
        assert!(checked(&wb, a) && !checked(&wb, b) && checked(&wb, d));
    }

    #[test]
    fn lists_spin_buttons_and_scroll_bars() {
        let mut wb = book();
        let combo = add(&mut wb, ControlKind::ComboBox, (1, 1), |c| {
            c.link = Some(range("$A$1"));
            c.input = Some(range("$D$1:$D$3"));
        });
        assert_eq!(wb.control_items(0, wb.control(combo).unwrap().1), ["Red", "Green", "Blue"]);
        assert!(wb.set_control_selected(combo, 3));
        wb.commit("Drop Down");
        assert_eq!(wb.value_at(0, 0, 0), Value::Number(3.0));
        wb.set_cell_text(0, 0, 0, "2").unwrap();
        wb.commit("type");
        assert_eq!(wb.control_state(0, wb.control(combo).unwrap().1).selected, 2);
        let spin = add(&mut wb, ControlKind::SpinButton, (4, 1), |c| {
            c.link = Some(range("$B$1"));
            c.min = 10;
            c.max = 20;
            c.inc = 5;
        });
        let step = |wb: &mut Workbook, delta: i32| {
            let control = wb.control(spin).unwrap().1.clone();
            let now = wb.control_state(0, &control).value;
            wb.set_control_value(spin, stepped(&control, now, delta));
            wb.commit("Spinner");
        };
        step(&mut wb, 5);
        assert_eq!(wb.value_at(0, 0, 1), Value::Number(10.0), "0 is under the minimum");
        step(&mut wb, 5);
        step(&mut wb, 5);
        step(&mut wb, 5);
        assert_eq!(wb.value_at(0, 0, 1), Value::Number(20.0), "the maximum holds it");
        // The cell moves the control, within its ends.
        wb.set_cell_text(0, 0, 1, "12").unwrap();
        wb.commit("type");
        step(&mut wb, -5);
        assert_eq!(wb.value_at(0, 0, 1), Value::Number(10.0));
        assert!(wb.undo());
        assert_eq!(wb.value_at(0, 0, 1), Value::Number(12.0));
        assert!(!wb.set_control_value(combo, 3) && !wb.set_control_selected(spin, 1));
    }

    #[test]
    fn controls_follow_rows_columns_and_sheets() {
        let mut wb = book();
        let id = add(&mut wb, ControlKind::ListBox, (4, 2), |c| {
            c.link = Some(range("$A$1"));
            c.input = Some(range("$D$1:$D$3"));
        });
        wb.insert_rows(0, 0, 2);
        wb.insert_cols(0, 0, 1);
        wb.commit("insert");
        let (_, control) = wb.control(id).unwrap();
        assert_eq!(control.anchor, (6, 3));
        assert_eq!(control.link.as_ref().unwrap().to_text(), "$B$3");
        assert_eq!(control.input.as_ref().unwrap().to_text(), "$E$3:$E$5");
        assert_eq!(wb.control_items(0, control), ["Red", "Green", "Blue"]);
        // A copy of the sheet has controls of its own.
        let copy = wb.duplicate_sheet(0).unwrap();
        wb.commit("copy");
        assert_eq!(wb.sheets[copy].controls.len(), 1);
        assert_ne!(wb.sheets[copy].controls[0].id, id);
        // The linked cell goes: the link goes with it.
        wb.delete_cols(0, 1, 1);
        wb.commit("delete");
        assert_eq!(wb.control(id).unwrap().1.link, None);
        assert!(wb.remove_control(id));
        wb.commit("Delete");
        assert!(wb.control(id).is_none());
    }

    #[test]
    fn a_control_read_from_a_file_knows_when_it_changed() {
        let mut control = FormControl::new(ControlKind::Button);
        control.text = "Go".to_string();
        control.macro_name = Some("[0]!Run".to_string());
        assert_eq!(control.macro_short_name(), Some("Run"));
        let source = ControlSource {
            shape: "<v:shape/>".to_string(),
            shape_id: 1025,
            properties: String::new(),
            control: String::new(),
            as_read: Box::new(control.content()),
        };
        control.id = 7;
        control.source = Some(Arc::new(source));
        assert!(FormControl::unchanged_source(&control).is_some());
        control.text = "Stop".to_string();
        assert!(FormControl::unchanged_source(&control).is_none());
    }
}
