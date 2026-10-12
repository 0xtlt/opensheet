//! The Format Cells dialog (⌘1): Number, Alignment, Font, Border, Fill and
//! Protection.
//!
//! The dialog opens on the formatting of the selection. A property the
//! selected cells disagree on is shown blank (or as a mixed check box) and
//! is left alone unless the user sets it: only what was touched is applied,
//! in one undo step.

mod number;

use self::number::{Category, Choice};
use super::{Command, ContextMenu, Cx, Feature, Intent, MenuSection, MenuTarget};
use crate::app::{ExcelApp, MenuEntry};
use crate::grid::{FontResolver, cell_font, pt_to_px};
use crate::i18n::t;
use crate::shell::Modal;
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{self, c};
use crate::widgets::{
    self, Checkbox, ColorButton, ColorGrid, GroupBox, ListBox, NumberField, Select, Sketch, TabStrip, TextField, Tri,
    dialog, icon, label, labeled, ok_cancel, preview, tooltip,
};
use gpui::{
    AnyElement, App, Bounds, Context, Entity, FontWeight, MouseButton, MouseDownEvent, Pixels, SharedString, Window,
    canvas, div, prelude::*, px,
};
use opensheet_engine::format;
use opensheet_engine::style::{Border, BorderStyle, Color, HAlign, Style, Underline, VAlign};
use opensheet_engine::{Locale, Rect, Value};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

pub struct FormatCells;

commands! {
    /// Opening the dialog, on the tab last used or on a given one.
    pub enum Cmd {
        Open = "format-cells", t("Format Cells…", "Format de cellule…");
        Number = "", t("More Number Formats…", "Autres formats numériques…");
        Alignment = "", t("Format Cell Alignment", "Format de cellule - Alignement");
        Font = "", t("Font Settings", "Paramètres de police");
        Border = "borders", t("More Borders…", "Autres bordures…");
    }
}

impl Command for Cmd {
    fn label(self) -> &'static str {
        self.face().1
    }

    fn icon(self) -> &'static str {
        self.face().0
    }

    fn run(self, app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
        let tab = match self {
            Cmd::Open => app.ext::<Memory>().map(|m| m.tab).unwrap_or(TAB_NUMBER),
            Cmd::Number => TAB_NUMBER,
            Cmd::Alignment => TAB_ALIGNMENT,
            Cmd::Font => TAB_FONT,
            Cmd::Border => TAB_BORDER,
        };
        if app.is_editing() && !app.commit_edit(None, window, cx) {
            return;
        }
        if !app.may_all(Intent::FormatCells, cx) {
            return;
        }
        let state = Opening::of(app, tab);
        app.open_modal(window, cx, move |modal, _, cx| Dialog::new(modal, state, cx));
    }
}

impl Feature for FormatCells {
    fn name(&self) -> &'static str {
        "format_cells"
    }

    fn context_menu(&self, app: &ExcelApp, target: &MenuTarget, menu: &mut ContextMenu) {
        if matches!(target, MenuTarget::Cell(..) | MenuTarget::RowHeader(_) | MenuTarget::ColHeader(_)) {
            menu.add(MenuSection::Format, MenuEntry::command(Cmd::Open, app).shortcut("⌘1"));
        }
    }
}

const TAB_NUMBER: usize = 0;
const TAB_ALIGNMENT: usize = 1;
const TAB_FONT: usize = 2;
const TAB_BORDER: usize = 3;
const TAB_FILL: usize = 4;
const TAB_PROTECTION: usize = 5;

/// The tab the dialog was last left on.
#[derive(Default)]
struct Memory {
    tab: usize,
}

// The six edges the Border tab deals with.
const TOP: usize = 0;
const BOTTOM: usize = 1;
const LEFT: usize = 2;
const RIGHT: usize = 3;
const INNER_H: usize = 4;
const INNER_V: usize = 5;

/// A property of the selection: its common value (`None` when the cells
/// disagree) and whether the user set it.
struct Prop<T> {
    value: Option<T>,
    /// The cells disagreed when the dialog opened.
    mixed: bool,
    changed: bool,
}

impl<T: Clone + PartialEq> Prop<T> {
    fn of(styles: &[Style], get: impl Fn(&Style) -> T) -> Prop<T> {
        let first = get(&styles[0]);
        let same = styles.iter().all(|style| get(style) == first);
        Prop {
            value: same.then_some(first),
            mixed: !same,
            changed: false,
        }
    }

    fn set(&mut self, value: T) {
        self.value = Some(value);
        self.changed = true;
    }

    /// Back to "leave each cell as it is".
    fn unset(&mut self) {
        self.value = None;
        self.changed = false;
    }

    /// The value to apply, if the user set one.
    fn change(&self) -> Option<T> {
        self.value.clone().filter(|_| self.changed)
    }
}

/// What the dialog needs to know about the selection, gathered before it
/// opens.
struct Opening {
    tab: usize,
    locale: &'static Locale,
    fonts: Rc<FontResolver>,
    /// Value of the active cell, for the Sample box.
    sample: Value,
    /// The distinct styles of the selected cells.
    styles: Vec<Style>,
    /// `None`: some of the selection is merged, some is not.
    merged: Option<bool>,
    /// The selection spans several rows / columns.
    many_rows: bool,
    many_cols: bool,
    /// The six edges of the current range, `None` where cells disagree.
    edges: [Option<Border>; 6],
    /// Format codes the workbook uses.
    used_codes: Vec<String>,
}

impl Opening {
    fn of(app: &ExcelApp, tab: usize) -> Opening {
        let sheet = &app.wb.sheets[app.sheet];
        let (row, col) = app.active();
        let ranges: Vec<Rect> = app
            .sel()
            .ranges
            .iter()
            .map(|range| app.wb.clamp_to_used(app.sheet, *range))
            .collect();
        // The distinct styles, without walking a million blank cells.
        let mut ids = vec![app.wb.style_id_at(app.sheet, row, col)];
        for range in &ranges {
            let area = range.rows() as u64 * range.cols() as u64;
            if area <= 50_000 {
                for r in range.r0..=range.r1 {
                    for c in range.c0..=range.c1 {
                        ids.push(sheet.style_id_at(r, c));
                    }
                }
            } else {
                ids.extend(
                    sheet
                        .cells
                        .range((range.r0, 0)..=(range.r1, u32::MAX))
                        .filter(|((_, c), _)| *c >= range.c0 && *c <= range.c1)
                        .map(|(_, cell)| cell.style),
                );
            }
        }
        ids.sort_unstable();
        ids.dedup();
        let styles: Vec<Style> = ids.iter().map(|id| app.wb.styles.get(*id).clone()).collect();

        let touches_merge = |range: &Rect| sheet.merges.iter().any(|m| m.intersects(range));
        let is_merge = |range: &Rect| sheet.merges.contains(range);
        let merged = if ranges.iter().all(is_merge) {
            Some(true)
        } else if !ranges.iter().any(touches_merge) {
            Some(false)
        } else {
            None
        };

        let current = app.wb.clamp_to_used(app.sheet, app.sel().current());
        let mut used_codes: Vec<String> = Vec::new();
        for id in 0..app.wb.styles.len() {
            let code = app.wb.styles.get(opensheet_engine::StyleId(id as u32)).num_fmt.to_string();
            if !used_codes.contains(&code) {
                used_codes.push(code);
            }
        }
        Opening {
            tab,
            locale: app.wb.locale,
            fonts: app.fonts.clone(),
            sample: app.wb.value_at(app.sheet, row, col),
            styles,
            merged,
            many_rows: current.rows() > 1 && !is_merge(&current),
            many_cols: current.cols() > 1 && !is_merge(&current),
            edges: edges_of(app, current),
            used_codes,
        }
    }
}

/// The border along each edge of a range: `None` where its cells disagree.
fn edges_of(app: &ExcelApp, range: Rect) -> [Option<Border>; 6] {
    let sheet = &app.wb.sheets[app.sheet];
    let borders = |r: u32, c: u32| app.wb.styles.get(sheet.style_id_at(r, c)).borders;
    // Look at a bounded number of cells: beyond it, call the edge mixed.
    let cols: Vec<u32> = (range.c0..=range.c1).take(400).collect();
    let rows: Vec<u32> = (range.r0..=range.r1).take(400).collect();
    let complete = cols.len() as u32 == range.cols() && rows.len() as u32 == range.rows();
    let common = |values: Vec<Border>| -> Option<Border> {
        let first = *values.first()?;
        (complete && values.iter().all(|b| *b == first)).then_some(first)
    };
    let inner_h: Vec<Border> = rows
        .iter()
        .filter(|r| **r < range.r1)
        .flat_map(|r| cols.iter().map(move |c| (*r, *c)))
        .map(|(r, c)| borders(r, c).bottom)
        .collect();
    let inner_v: Vec<Border> = rows
        .iter()
        .flat_map(|r| cols.iter().filter(|c| **c < range.c1).map(move |c| (*r, *c)))
        .map(|(r, c)| borders(r, c).right)
        .collect();
    [
        common(cols.iter().map(|c| borders(range.r0, *c).top).collect()),
        common(cols.iter().map(|c| borders(range.r1, *c).bottom).collect()),
        common(rows.iter().map(|r| borders(*r, range.c0).left).collect()),
        common(rows.iter().map(|r| borders(*r, range.c1).right).collect()),
        if inner_h.is_empty() { Some(Border::NONE) } else { common(inner_h) },
        if inner_v.is_empty() { Some(Border::NONE) } else { common(inner_v) },
    ]
}

/// One edge in the Border tab.
#[derive(Clone, Copy)]
struct Edge {
    /// `None`: the cells disagree (drawn grey).
    border: Option<Border>,
    changed: bool,
}

/// What OK applies: only what the user set.
#[derive(Default)]
struct Changes {
    num_fmt: Option<Arc<str>>,
    h_align: Option<HAlign>,
    v_align: Option<VAlign>,
    indent: Option<u8>,
    wrap: Option<bool>,
    shrink: Option<bool>,
    rotation: Option<i16>,
    merge: Option<bool>,
    font_name: Option<Arc<str>>,
    bold: Option<bool>,
    italic: Option<bool>,
    size: Option<f32>,
    underline: Option<Underline>,
    color: Option<Option<Color>>,
    strike: Option<bool>,
    edges: [Option<Border>; 6],
    fill: Option<Option<Color>>,
    locked: Option<bool>,
    hidden: Option<bool>,
}

impl Changes {
    fn style(&self, s: &mut Style) {
        if let Some(code) = &self.num_fmt {
            s.num_fmt = code.clone();
        }
        if let Some(v) = self.h_align {
            s.h_align = v;
        }
        if let Some(v) = self.v_align {
            s.v_align = v;
        }
        if let Some(v) = self.indent {
            s.indent = v;
            // An indent needs a side to start from.
            if v > 0 && s.h_align == HAlign::General {
                s.h_align = HAlign::Left;
            }
        }
        if let Some(v) = self.wrap {
            s.wrap = v;
        }
        if let Some(v) = self.shrink {
            s.shrink_to_fit = v;
        }
        if let Some(v) = self.rotation {
            s.rotation = v;
        }
        if let Some(v) = &self.font_name {
            s.font.name = v.clone();
        }
        if let Some(v) = self.bold {
            s.font.bold = v;
        }
        if let Some(v) = self.italic {
            s.font.italic = v;
        }
        if let Some(v) = self.size {
            s.font.set_size_pt(v);
        }
        if let Some(v) = self.underline {
            s.font.underline = v;
        }
        if let Some(v) = self.color {
            s.font.color = v;
        }
        if let Some(v) = self.strike {
            s.font.strike = v;
        }
        if let Some(v) = self.fill {
            s.fill = v;
        }
        if let Some(v) = self.locked {
            s.locked = v;
        }
        if let Some(v) = self.hidden {
            s.hidden = v;
        }
    }

    /// Apply everything to the selection as one undo step.
    fn apply(&self, app: &mut ExcelApp, cx: &mut Cx) {
        if !app.may_all(Intent::FormatCells, cx) {
            return;
        }
        let sheet = app.sheet;
        let ranges = app.sel().ranges.clone();
        for range in &ranges {
            app.wb.update_style(sheet, *range, &|s| self.style(s));
        }
        if let Some(pt) = self.size {
            app.grow_rows_for_font(pt);
        }
        for range in &ranges {
            self.apply_borders(app, app.wb.clamp_to_used(sheet, *range));
            match self.merge {
                Some(true) if !range.is_single() => app.wb.merge(sheet, *range),
                Some(false) => {
                    app.wb.unmerge(sheet, *range);
                }
                _ => {}
            }
        }
        app.commit(t("Format Cells", "Format de cellule"), cx);
        if self.merge.is_some() {
            // The selection takes the shape of the merged cell (or of its
            // first cell again).
            let (row, col) = app.active();
            app.select_cell(row, col, cx);
        }
    }

    /// Set the edges the user chose on a range: the outline on its outer
    /// cells, the inside lines between its cells.
    fn apply_borders(&self, app: &mut ExcelApp, range: Rect) {
        let sheet = app.sheet;
        let Rect { r0, c0, r1, c1 } = range;
        let mut set = |rect: Rect, f: &dyn Fn(&mut Style)| app.wb.update_style(sheet, rect, f);
        if let Some(b) = self.edges[TOP] {
            set(Rect::new(r0, c0, r0, c1), &|s| s.borders.top = b);
            // A line removed here must not survive as the bottom of the
            // cells above.
            if b.is_none() && r0 > 0 {
                set(Rect::new(r0 - 1, c0, r0 - 1, c1), &|s| s.borders.bottom = b);
            }
        }
        if let Some(b) = self.edges[BOTTOM] {
            set(Rect::new(r1, c0, r1, c1), &|s| s.borders.bottom = b);
            if b.is_none() && r1 + 1 < opensheet_engine::MAX_ROWS {
                set(Rect::new(r1 + 1, c0, r1 + 1, c1), &|s| s.borders.top = b);
            }
        }
        if let Some(b) = self.edges[LEFT] {
            set(Rect::new(r0, c0, r1, c0), &|s| s.borders.left = b);
            if b.is_none() && c0 > 0 {
                set(Rect::new(r0, c0 - 1, r1, c0 - 1), &|s| s.borders.right = b);
            }
        }
        if let Some(b) = self.edges[RIGHT] {
            set(Rect::new(r0, c1, r1, c1), &|s| s.borders.right = b);
            if b.is_none() && c1 + 1 < opensheet_engine::MAX_COLS {
                set(Rect::new(r0, c1 + 1, r1, c1 + 1), &|s| s.borders.left = b);
            }
        }
        if let (Some(b), true) = (self.edges[INNER_H], r1 > r0) {
            set(Rect::new(r0, c0, r1 - 1, c1), &|s| s.borders.bottom = b);
            set(Rect::new(r0 + 1, c0, r1, c1), &|s| s.borders.top = b);
        }
        if let (Some(b), true) = (self.edges[INNER_V], c1 > c0) {
            set(Rect::new(r0, c0, r1, c1 - 1), &|s| s.borders.right = b);
            set(Rect::new(r0, c0 + 1, r1, c1), &|s| s.borders.left = b);
        }
    }
}

const H_ALIGNS: [HAlign; 7] = [
    HAlign::General,
    HAlign::Left,
    HAlign::Center,
    HAlign::Right,
    HAlign::Fill,
    HAlign::Justify,
    HAlign::CenterAcross,
];
const V_ALIGNS: [VAlign; 3] = [VAlign::Top, VAlign::Center, VAlign::Bottom];
const UNDERLINES: [Underline; 3] = [Underline::None, Underline::Single, Underline::Double];
const LINE_STYLES: [BorderStyle; 8] = [
    BorderStyle::None,
    BorderStyle::Hair,
    BorderStyle::Dotted,
    BorderStyle::Dashed,
    BorderStyle::Thin,
    BorderStyle::Medium,
    BorderStyle::Thick,
    BorderStyle::Double,
];

struct Dialog {
    modal: Modal,
    tab: usize,
    locale: &'static Locale,
    fonts: Rc<FontResolver>,
    sample: Value,

    // Number
    /// The choices of the Number tab; meaningless while `code_mixed`.
    choice: Choice,
    /// The cells have different formats and none was chosen yet.
    code_mixed: bool,
    number_touched: bool,
    decimals: Entity<TextInput>,
    custom_codes: Vec<String>,
    code_input: Entity<TextInput>,

    // Alignment
    h_align: Prop<HAlign>,
    v_align: Prop<VAlign>,
    indent: Entity<TextInput>,
    indent_at_open: String,
    wrap: Prop<bool>,
    shrink: Prop<bool>,
    merge: Prop<bool>,
    can_merge: bool,
    rotation: Entity<TextInput>,
    rotation_at_open: String,
    dial_bounds: Rc<Cell<Bounds<Pixels>>>,

    // Font
    font_names: Vec<SharedString>,
    font_name: Entity<TextInput>,
    font_name_at_open: String,
    bold: Prop<bool>,
    italic: Prop<bool>,
    size: Entity<TextInput>,
    size_at_open: String,
    underline: Prop<Underline>,
    color: Prop<Option<Color>>,
    strike: Prop<bool>,

    // Border
    line_style: BorderStyle,
    line_color: Option<Color>,
    edges: [Edge; 6],
    many_rows: bool,
    many_cols: bool,
    preview_bounds: Rc<Cell<Bounds<Pixels>>>,

    // Fill
    fill: Prop<Option<Color>>,

    // Protection
    locked: Prop<bool>,
    hidden: Prop<bool>,
}

/// A size in points as the Size box shows it.
fn size_text(pt: f32, locale: &Locale) -> String {
    widgets::number_text(pt as f64, locale.decimal_sep)
}

impl Dialog {
    fn new(modal: Modal, open: Opening, cx: &mut Context<Self>) -> Self {
        let locale = open.locale;
        let styles = &open.styles;
        let code = Prop::of(styles, |s| s.num_fmt.to_string());
        let choice = match &code.value {
            Some(code) => Choice::from_code(code, locale),
            None => Choice::general(),
        };
        let custom_codes = number::custom_codes(locale, &open.used_codes);
        let decimals = widgets::number_input(cx, Some(choice.decimals as f64), 0.0, number::MAX_DECIMALS as f64, 1.0);
        let code_input = widgets::input(cx, code.value.as_deref().map(|c| number::shown(c, locale)).unwrap_or_default());
        cx.subscribe(&decimals, |this, _, ev: &InputEvent, cx| {
            if *ev == InputEvent::Changed {
                if let Some(n) = this.decimals.read(cx).number() {
                    this.choice.decimals = (n.max(0.0) as usize).min(number::MAX_DECIMALS);
                }
                this.number_touched = true;
                cx.notify();
            }
        })
        .detach();
        cx.subscribe(&code_input, |this, _, ev: &InputEvent, cx| {
            if *ev == InputEvent::Changed {
                this.number_touched = true;
                cx.notify();
            }
        })
        .detach();

        let indent = Prop::of(styles, |s| s.indent);
        let indent_at_open = indent.value.map(|v| v.to_string()).unwrap_or_default();
        let indent_input = widgets::number_input(cx, indent.value.map(|v| v as f64), 0.0, 250.0, 1.0);
        let rotation = Prop::of(styles, |s| s.rotation);
        let rotation_at_open = rotation.value.map(|v| v.to_string()).unwrap_or_default();
        let rotation_input = widgets::number_input(cx, rotation.value.map(|v| v as f64), -90.0, 90.0, 1.0);

        // Office fonts first, as in the ribbon's list, then the system's.
        let mut font_names: Vec<SharedString> = Vec::new();
        for name in ["Aptos Narrow", "Aptos", "Calibri", "Arial", "Times New Roman", "Courier New"]
            .iter()
            .map(|n| n.to_string())
            .chain(open.fonts.names.iter().cloned())
        {
            if !font_names.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                font_names.push(name.into());
            }
        }
        let font_name = Prop::of(styles, |s| s.font.name.to_string());
        let font_name_at_open = font_name.value.clone().unwrap_or_default();
        let size = Prop::of(styles, |s| s.font.size);
        let size_at_open = size.value.map(|half| size_text(half as f32 / 2.0, locale)).unwrap_or_default();

        Dialog {
            tab: open.tab.min(TAB_PROTECTION),
            locale,
            sample: open.sample,
            code_mixed: code.mixed,
            choice,
            number_touched: false,
            decimals,
            custom_codes,
            code_input,
            h_align: Prop::of(styles, |s| s.h_align),
            v_align: Prop::of(styles, |s| s.v_align),
            indent: indent_input,
            indent_at_open,
            wrap: Prop::of(styles, |s| s.wrap),
            shrink: Prop::of(styles, |s| s.shrink_to_fit),
            merge: Prop {
                value: open.merged,
                mixed: open.merged.is_none(),
                changed: false,
            },
            can_merge: open.many_rows || open.many_cols || open.merged != Some(false),
            rotation: rotation_input,
            rotation_at_open,
            dial_bounds: Rc::default(),
            font_names,
            font_name: widgets::input(cx, font_name_at_open.clone()),
            font_name_at_open,
            bold: Prop::of(styles, |s| s.font.bold),
            italic: Prop::of(styles, |s| s.font.italic),
            size: widgets::input(cx, size_at_open.clone()),
            size_at_open,
            underline: Prop::of(styles, |s| s.font.underline),
            color: Prop::of(styles, |s| s.font.color),
            strike: Prop::of(styles, |s| s.font.strike),
            line_style: BorderStyle::Thin,
            line_color: None,
            edges: open.edges.map(|border| Edge { border, changed: false }),
            many_rows: open.many_rows,
            many_cols: open.many_cols,
            preview_bounds: Rc::default(),
            fill: Prop::of(styles, |s| s.fill),
            locked: Prop::of(styles, |s| s.locked),
            hidden: Prop::of(styles, |s| s.hidden),
            fonts: open.fonts,
            modal,
        }
    }

    // ---- reading the controls ---------------------------------------------

    /// The format code the Number tab stands for right now.
    fn code(&self, cx: &App) -> String {
        match self.choice.code(self.locale) {
            Some(code) => code,
            None => number::typed(self.code_input.read(cx).text(), &self.custom_codes, self.locale),
        }
    }

    /// The size typed in the Size box, in points.
    fn typed_size(&self, cx: &App) -> Option<f32> {
        let text = self.size.read(cx).text().trim().replace(self.locale.decimal_sep, ".");
        text.parse::<f32>().ok().filter(|pt| (1.0..=409.0).contains(pt))
    }

    /// The font the cells would get, for the preview.
    fn preview_style(&self, cx: &App) -> Style {
        let mut style = Style::default();
        let name = self.font_name.read(cx).text().trim().to_string();
        if !name.is_empty() {
            style.font.name = Arc::from(name);
        }
        if let Some(pt) = self.typed_size(cx) {
            style.font.set_size_pt(pt);
        }
        style.font.bold = self.bold.value.unwrap_or(false);
        style.font.italic = self.italic.value.unwrap_or(false);
        style.font.underline = self.underline.value.unwrap_or_default();
        style.font.strike = self.strike.value.unwrap_or(false);
        style.font.color = self.color.value.flatten();
        style
    }

    fn changes(&self, cx: &App) -> Changes {
        let typed = |input: &Entity<TextInput>, at_open: &str| -> Option<String> {
            let text = input.read(cx).text().trim().to_string();
            (text != at_open && !text.is_empty()).then_some(text)
        };
        let mut edges = [None; 6];
        for (i, edge) in self.edges.iter().enumerate() {
            edges[i] = edge.border.filter(|_| edge.changed);
        }
        Changes {
            num_fmt: (self.number_touched && !self.code_mixed).then(|| Arc::from(self.code(cx))),
            h_align: self.h_align.change(),
            v_align: self.v_align.change(),
            indent: typed(&self.indent, &self.indent_at_open)
                .and_then(|_| self.indent.read(cx).number())
                .map(|n| n.clamp(0.0, 250.0) as u8),
            wrap: self.wrap.change(),
            shrink: self.shrink.change(),
            rotation: typed(&self.rotation, &self.rotation_at_open)
                .and_then(|_| self.rotation.read(cx).number())
                .map(|n| n.clamp(-90.0, 90.0) as i16),
            merge: self.merge.change(),
            font_name: typed(&self.font_name, &self.font_name_at_open).map(Arc::from),
            bold: self.bold.change(),
            italic: self.italic.change(),
            size: typed(&self.size, &self.size_at_open).and_then(|_| self.typed_size(cx)),
            underline: self.underline.change(),
            color: self.color.change(),
            strike: self.strike.change(),
            edges,
            fill: self.fill.change(),
            locked: self.locked.change(),
            hidden: self.hidden.change(),
        }
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let changes = self.changes(cx);
        self.modal.update_app(cx, |app, cx| changes.apply(app, cx));
        self.modal.close(window, cx);
    }

    // ---- small builders -----------------------------------------------------

    /// A check box bound to a property: one that opened mixed can go back
    /// to mixed.
    fn check(
        &self,
        id: &'static str,
        text: &'static str,
        prop: &Prop<bool>,
        field: fn(&mut Self) -> &mut Prop<bool>,
        cx: &mut Context<Self>,
    ) -> Checkbox {
        let state = if prop.changed || !prop.mixed { Tri::from(prop.value) } else { Tri::Mixed };
        let checkbox = Checkbox::new(id, text, state);
        if prop.mixed {
            checkbox.on_change_tri(cx.listener(move |this, state: &Tri, _, cx| {
                match state.as_bool() {
                    Some(on) => field(this).set(on),
                    None => field(this).unset(),
                }
                cx.notify();
            }))
        } else {
            checkbox.on_change(cx.listener(move |this, on: &bool, _, cx| {
                field(this).set(*on);
                cx.notify();
            }))
        }
    }

    /// A title over a control.
    fn titled(title: &'static str, control: impl IntoElement) -> gpui::Div {
        div().flex().flex_col().gap(px(3.)).child(label(title)).child(control)
    }

    // ---- Number ---------------------------------------------------------------

    fn set_category(&mut self, index: usize, cx: &mut Context<Self>) {
        let before = (!self.code_mixed).then(|| self.code(cx));
        self.choice.category = Category::ALL[index];
        self.choice.kind = 0;
        if self.choice.category == Category::Custom {
            // Custom starts from the code the previous choices gave.
            let text = before.map(|code| number::shown(&code, self.locale)).unwrap_or_default();
            self.code_input.update(cx, |input, cx| input.set_text(text, cx));
        }
        self.code_mixed = false;
        self.number_touched = true;
        cx.notify();
    }

    fn number_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let locale = self.locale;
        let categories: Vec<SharedString> = Category::ALL.iter().map(|cat| cat.name().into()).collect();
        let selected = (!self.code_mixed).then(|| Category::ALL.iter().position(|c| *c == self.choice.category).unwrap_or(0));
        let code = self.code(cx);
        let sample = if self.code_mixed {
            String::new()
        } else {
            format::format_value(&self.sample, &code, locale).text
        };

        let decimals = || {
            labeled(
                t("Decimal places:", "Nombre de décimales :"),
                NumberField::new("decimals", &self.decimals).width(60.0),
            )
        };
        // A list of types: what each one makes of the sample date, or its
        // name.
        let kinds = |names: Vec<SharedString>, cx: &mut Context<Self>| {
            Self::titled(
                t("Type:", "Type :"),
                ListBox::new("kind", names, Some(self.choice.kind))
                    .height(widgets::ROW_H * 8.0)
                    .on_change(cx.listener(|this, ix: &usize, _, cx| {
                        this.choice.kind = *ix;
                        this.number_touched = true;
                        cx.notify();
                    })),
            )
        };
        let dated = |codes: Vec<String>| -> Vec<SharedString> {
            codes
                .iter()
                .map(|code| format::format_number(number::SAMPLE_DATE, code, locale).text.into())
                .collect()
        };
        let symbol = |cx: &mut Context<Self>| {
            let names: Vec<SharedString> = number::symbols(locale)
                .iter()
                .map(|s| if s.is_empty() { t("None", "Aucune").into() } else { (*s).into() })
                .collect();
            labeled(
                t("Symbol:", "Symbole :"),
                Select::new("symbol", names, Some(self.choice.symbol))
                    .width(120.0)
                    .on_change(cx.listener(|this, ix: &usize, _, cx| {
                        this.choice.symbol = *ix;
                        this.number_touched = true;
                        cx.notify();
                    })),
            )
        };
        // The negative styles, each shown on -1234.10 with these options.
        let negatives = |cx: &mut Context<Self>| {
            let samples: Vec<(SharedString, Option<Color>)> = (0..number::negatives(locale).len())
                .map(|negative| {
                    let choice = Choice {
                        negative,
                        ..self.choice.clone()
                    };
                    let code = choice.code(locale).unwrap_or_default();
                    let shown = format::format_number(-1234.1, &code, locale);
                    (shown.text.into(), shown.color)
                })
                .collect();
            let names = samples.iter().map(|(text, _)| text.clone()).collect();
            Self::titled(
                t("Negative numbers:", "Nombres négatifs :"),
                ListBox::new("negative", names, Some(self.choice.negative))
                    .height(widgets::ROW_H * 4.0)
                    .row(move |i, _| {
                        let (text, color) = samples[i].clone();
                        div()
                            .when_some(color, |d, color| d.text_color(c(color.0)))
                            .child(text)
                            .into_any_element()
                    })
                    .on_change(cx.listener(|this, ix: &usize, _, cx| {
                        this.choice.negative = *ix;
                        this.number_touched = true;
                        cx.notify();
                    })),
            )
        };

        let mut options: Vec<AnyElement> = Vec::new();
        if !self.code_mixed {
            match self.choice.category {
                Category::General | Category::Text => {}
                Category::Number => {
                    options.push(decimals().into_any_element());
                    options.push(
                        Checkbox::new(
                            "thousands",
                            t("Use 1000 Separator", "Utiliser le séparateur de milliers"),
                            self.choice.thousands,
                        )
                        .on_change(cx.listener(|this, on: &bool, _, cx| {
                            this.choice.thousands = *on;
                            this.number_touched = true;
                            cx.notify();
                        }))
                        .into_any_element(),
                    );
                    options.push(negatives(cx).into_any_element());
                }
                Category::Currency => {
                    options.push(decimals().into_any_element());
                    options.push(symbol(cx).into_any_element());
                    options.push(negatives(cx).into_any_element());
                }
                Category::Accounting => {
                    options.push(decimals().into_any_element());
                    options.push(symbol(cx).into_any_element());
                }
                Category::Date => options.push(kinds(dated(number::date_types(locale)), cx).into_any_element()),
                Category::Time => options.push(kinds(dated(number::time_types(locale)), cx).into_any_element()),
                Category::Percentage | Category::Scientific => options.push(decimals().into_any_element()),
                Category::Fraction => {
                    let names = number::fraction_types().iter().map(|(_, name)| (*name).into()).collect();
                    options.push(kinds(names, cx).into_any_element());
                }
                Category::Special => {
                    let names = number::special_types(locale).iter().map(|(_, name)| (*name).into()).collect();
                    options.push(kinds(names, cx).into_any_element());
                }
                Category::Custom => {
                    let shown: Vec<SharedString> = self
                        .custom_codes
                        .iter()
                        .map(|code| number::shown(code, locale).into())
                        .collect();
                    let current = self.custom_codes.iter().position(|c| *c == code);
                    options.push(
                        Self::titled(
                            t("Type:", "Type :"),
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(3.))
                                .child(div().flex().child(TextField::new(&self.code_input)))
                                .child(
                                    ListBox::new("custom", shown, current)
                                        .height(widgets::ROW_H * 8.0)
                                        .on_change(cx.listener(|this, ix: &usize, _, cx| {
                                            let text = number::shown(&this.custom_codes[*ix], this.locale);
                                            this.code_input.update(cx, |input, cx| input.set_text(text, cx));
                                            this.number_touched = true;
                                            cx.notify();
                                        })),
                                ),
                        )
                        .into_any_element(),
                    );
                }
            }
        }
        let description = if self.code_mixed { "" } else { self.choice.category.description() };

        div()
            .flex()
            .gap(px(12.))
            .child(Self::titled(
                t("Category:", "Catégorie :"),
                ListBox::new("category", categories, selected)
                    .width(130.0)
                    .height(widgets::ROW_H * 12.0)
                    .on_change(cx.listener(|this, ix: &usize, _, cx| this.set_category(*ix, cx))),
            ))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(
                        GroupBox::new(t("Sample", "Exemple")).child(
                            div()
                                .h(px(20.))
                                .flex()
                                .items_center()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .child(SharedString::from(sample)),
                        ),
                    )
                    .children(options)
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_color(c(theme::TEXT_MUTED))
                            .line_height(px(16.))
                            .child(description),
                    ),
            )
            .into_any_element()
    }

    // ---- Alignment ------------------------------------------------------------

    fn alignment_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let horizontal: Vec<SharedString> = [
            t("General", "Standard"),
            t("Left (Indent)", "Gauche (Retrait)"),
            t("Center", "Centré"),
            t("Right (Indent)", "Droite (Retrait)"),
            t("Fill", "Recopié"),
            t("Justify", "Justifié"),
            t("Center Across Selection", "Centré sur plusieurs colonnes"),
        ]
        .iter()
        .map(|name| (*name).into())
        .collect();
        let vertical: Vec<SharedString> = [t("Top", "Haut"), t("Center", "Centré"), t("Bottom", "Bas")]
            .iter()
            .map(|name| (*name).into())
            .collect();
        let h = self.h_align.value.and_then(|v| H_ALIGNS.iter().position(|a| *a == v));
        let v = self.v_align.value.and_then(|v| V_ALIGNS.iter().position(|a| *a == v));
        let degrees = self.rotation.read(cx).number().unwrap_or(0.0).clamp(-90.0, 90.0) as f32;
        let text = t("Text", "Texte");

        // The dial: a half circle of marks and a hand at the chosen angle.
        let bounds = self.dial_bounds.clone();
        let dial = div()
            .id("dial")
            .relative()
            .bg(c(0xFFFFFF))
            .border_1()
            .border_color(c(theme::BORDER_STRONG))
            .child(preview(96.0, 150.0, move |s: &mut Sketch| {
                let (cx0, cy0, radius) = (14.0, s.height / 2.0, 64.0);
                for step in -6..=6 {
                    let a = (step as f32 * 15.0).to_radians();
                    let (x, y) = (cx0 + radius * a.cos(), cy0 - radius * a.sin());
                    let half = if step % 3 == 0 { 2.0 } else { 1.0 };
                    s.rect(x - half, y - half, x + half, y + half, c(if step % 3 == 0 { 0x000000 } else { 0x808080 }));
                }
                let a = degrees.to_radians();
                let end = (cx0 + (radius - 8.0) * a.cos(), cy0 - (radius - 8.0) * a.sin());
                s.line((cx0, cy0), end, 1.0, c(0x000000));
                s.rect(end.0 - 3.0, end.1 - 3.0, end.0 + 3.0, end.1 + 3.0, c(theme::ACCENT));
                if degrees.abs() < 0.5 {
                    // Level text can be shown on the hand, as Excel does.
                    let font = widgets::ui_font();
                    s.rect(cx0 + 2.0, cy0 - 8.0, cx0 + 6.0 + s.measure(text, &font, 12.0), cy0 + 8.0, c(0xFFFFFF));
                    s.text(text, cx0 + 4.0, cy0 - 8.0, &font, 12.0, c(0x000000));
                }
            }))
            .child(
                canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    // The angle of the pointer seen from the pivot of the hand.
                    let b = this.dial_bounds.get();
                    let dx = f32::from(ev.position.x - b.left()) - 14.0;
                    let dy = f32::from(b.top() + b.size.height / 2.0 - ev.position.y);
                    let angle = dy.atan2(dx.max(0.01)).to_degrees().round().clamp(-90.0, 90.0);
                    this.rotation
                        .update(cx, |input, cx| input.set_text(format!("{angle:.0}"), cx));
                    cx.notify();
                }),
            );

        div()
            .flex()
            .gap(px(20.))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .child(
                        GroupBox::new(t("Text alignment", "Alignement du texte"))
                            .child(
                                div()
                                    .flex()
                                    .items_end()
                                    .gap(px(12.))
                                    .child(Self::titled(
                                        t("Horizontal:", "Horizontal :"),
                                        Select::new("h-align", horizontal, h).width(210.0).on_change(cx.listener(
                                            |this, ix: &usize, _, cx| {
                                                this.h_align.set(H_ALIGNS[*ix]);
                                                cx.notify();
                                            },
                                        )),
                                    ))
                                    .child(Self::titled(
                                        t("Indent:", "Retrait :"),
                                        NumberField::new("indent", &self.indent).width(60.0),
                                    )),
                            )
                            .child(Self::titled(
                                t("Vertical:", "Vertical :"),
                                Select::new("v-align", vertical, v).width(210.0).on_change(cx.listener(
                                    |this, ix: &usize, _, cx| {
                                        this.v_align.set(V_ALIGNS[*ix]);
                                        cx.notify();
                                    },
                                )),
                            )),
                    )
                    .child(
                        GroupBox::new(t("Text control", "Contrôle du texte"))
                            .child(self.check(
                                "wrap",
                                t("Wrap text", "Renvoyer à la ligne automatiquement"),
                                &self.wrap,
                                |d| &mut d.wrap,
                                cx,
                            ))
                            .child(self.check("shrink", t("Shrink to fit", "Ajuster"), &self.shrink, |d| &mut d.shrink, cx))
                            .child(
                                self.check(
                                    "merge",
                                    t("Merge cells", "Fusionner les cellules"),
                                    &self.merge,
                                    |d| &mut d.merge,
                                    cx,
                                )
                                .disabled(!self.can_merge),
                            ),
                    ),
            )
            .child(
                GroupBox::new(t("Orientation", "Orientation")).child(dial).child(labeled(
                    t("Degrees:", "Degrés :"),
                    NumberField::new("degrees", &self.rotation).width(60.0),
                )),
            )
            .into_any_element()
    }

    // ---- Font -------------------------------------------------------------------

    fn font_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let locale = self.locale;
        let name = self.font_name.read(cx).text().trim().to_string();
        let name_ix = self.font_names.iter().position(|n| n.eq_ignore_ascii_case(&name));
        let styles: Vec<SharedString> = [
            t("Regular", "Normal"),
            t("Italic", "Italique"),
            t("Bold", "Gras"),
            t("Bold Italic", "Gras italique"),
        ]
        .iter()
        .map(|name| (*name).into())
        .collect();
        let style_ix = match (self.bold.value, self.italic.value) {
            (Some(bold), Some(italic)) => Some(bold as usize * 2 + italic as usize),
            _ => None,
        };
        let sizes: Vec<SharedString> = ExcelApp::FONT_SIZES.iter().map(|s| size_text(*s, locale).into()).collect();
        let size_ix = self
            .typed_size(cx)
            .and_then(|pt| ExcelApp::FONT_SIZES.iter().position(|s| (*s - pt).abs() < 0.01));
        let underlines: Vec<SharedString> = [t("None", "Aucun"), t("Single", "Simple"), t("Double", "Double")]
            .iter()
            .map(|name| (*name).into())
            .collect();
        let underline_ix = self.underline.value.and_then(|u| UNDERLINES.iter().position(|v| *v == u));

        // The sample, in the font the cells would get.
        let style = self.preview_style(cx);
        let font = cell_font(&style, &self.fonts);
        let font_px = pt_to_px(style.font.size_pt());
        let color = style.font.color.map(|col| c(col.0)).unwrap_or(c(0x000000));
        let (underline, strike) = (style.font.underline != Underline::None, style.font.strike);
        let sample = preview(300.0, 64.0, move |s: &mut Sketch| {
            let text = "AaBbCcYyZz";
            let width = s.measure(text, &font, font_px);
            let x = ((s.width - width) / 2.0).max(4.0);
            let y = (s.height - font_px * 1.25) / 2.0;
            s.styled_text(text, x, y, &font, font_px, color, underline, strike);
        });

        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(Self::titled(
                        t("Font:", "Police :"),
                        div()
                            .w(px(220.))
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(div().flex().child(TextField::new(&self.font_name)))
                            .child(
                                ListBox::new("fonts", self.font_names.clone(), name_ix)
                                    .height(widgets::ROW_H * 6.0)
                                    .on_change(cx.listener(|this, ix: &usize, _, cx| {
                                        let name = this.font_names[*ix].to_string();
                                        this.font_name.update(cx, |input, cx| input.set_text(name, cx));
                                        cx.notify();
                                    })),
                            ),
                    ))
                    .child(Self::titled(
                        t("Font style:", "Style :"),
                        ListBox::new("font-style", styles, style_ix)
                            .width(130.0)
                            .height(widgets::ROW_H * 6.0 + widgets::CONTROL_H + 2.0)
                            .on_change(cx.listener(|this, ix: &usize, _, cx| {
                                this.bold.set(*ix >= 2);
                                this.italic.set(*ix % 2 == 1);
                                cx.notify();
                            })),
                    ))
                    .child(Self::titled(
                        t("Size:", "Taille :"),
                        div()
                            .w(px(80.))
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(div().flex().child(TextField::new(&self.size)))
                            .child(
                                ListBox::new("sizes", sizes, size_ix)
                                    .height(widgets::ROW_H * 6.0)
                                    .on_change(cx.listener(|this, ix: &usize, _, cx| {
                                        let text = size_text(ExcelApp::FONT_SIZES[*ix], this.locale);
                                        this.size.update(cx, |input, cx| input.set_text(text, cx));
                                        cx.notify();
                                    })),
                            ),
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap(px(24.))
                    .child(Self::titled(
                        t("Underline:", "Soulignement :"),
                        Select::new("underline", underlines, underline_ix).width(150.0).on_change(cx.listener(
                            |this, ix: &usize, _, cx| {
                                this.underline.set(UNDERLINES[*ix]);
                                cx.notify();
                            },
                        )),
                    ))
                    .child(Self::titled(
                        t("Color:", "Couleur :"),
                        ColorButton::new("font-color", self.color.value)
                            .none_label(t("Automatic", "Automatique"))
                            .more(&self.modal.app)
                            .width(150.0)
                            .on_pick(cx.listener(|this, color: &Option<Color>, _, cx| {
                                this.color.set(*color);
                                cx.notify();
                            })),
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap(px(24.))
                    .child(
                        div()
                            .w(px(150.))
                            .child(GroupBox::new(t("Effects", "Effets")).child(self.check(
                                "strike",
                                t("Strikethrough", "Barré"),
                                &self.strike,
                                |d| &mut d.strike,
                                cx,
                            ))),
                    )
                    .child(
                        div().flex_1().child(
                            GroupBox::new(t("Preview", "Aperçu")).child(
                                div()
                                    .bg(c(0xFFFFFF))
                                    .border_1()
                                    .border_color(c(theme::BORDER_STRONG))
                                    .flex()
                                    .justify_center()
                                    .child(sample),
                            ),
                        ),
                    ),
            )
            .into_any_element()
    }

    // ---- Border -----------------------------------------------------------------

    /// What a click on an edge does: draw it with the chosen line, or
    /// remove it when it already is that line.
    fn toggle_edge(&mut self, index: usize, cx: &mut Context<Self>) {
        if (index == INNER_H && !self.many_rows) || (index == INNER_V && !self.many_cols) {
            return;
        }
        let line = Border {
            style: self.line_style,
            color: self.line_color,
        };
        let edge = &mut self.edges[index];
        edge.border = Some(if edge.border == Some(line) || line.is_none() { Border::NONE } else { line });
        edge.changed = true;
        cx.notify();
    }

    fn set_edges(&mut self, indices: &[usize], border: Border, cx: &mut Context<Self>) {
        for index in indices {
            if (*index == INNER_H && !self.many_rows) || (*index == INNER_V && !self.many_cols) {
                continue;
            }
            self.edges[*index] = Edge {
                border: Some(border),
                changed: true,
            };
        }
        cx.notify();
    }

    fn border_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        // A line style drawn as a short sample.
        let sample = |style: BorderStyle| -> AnyElement {
            let bar = |w: f32, h: f32| div().w(px(w)).h(px(h)).flex_none().bg(c(0x000000));
            let dashes = |on: f32, off: f32, h: f32| {
                let count = (96.0 / (on + off)) as usize;
                div()
                    .flex()
                    .gap(px(off))
                    .children((0..count).map(move |_| bar(on, h)))
                    .into_any_element()
            };
            match style {
                BorderStyle::None => div().child(t("None", "Aucune")).into_any_element(),
                BorderStyle::Hair => dashes(1.0, 1.0, 1.0),
                BorderStyle::Dotted => dashes(2.0, 2.0, 1.0),
                BorderStyle::Dashed => dashes(4.0, 2.0, 1.0),
                BorderStyle::Thin => bar(96.0, 1.0).into_any_element(),
                BorderStyle::Medium => bar(96.0, 2.0).into_any_element(),
                BorderStyle::Thick => bar(96.0, 3.0).into_any_element(),
                BorderStyle::Double => div()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .child(bar(96.0, 1.0))
                    .child(bar(96.0, 1.0))
                    .into_any_element(),
            }
        };
        let style_ix = LINE_STYLES.iter().position(|s| *s == self.line_style);
        let names: Vec<SharedString> = LINE_STYLES.iter().map(|_| SharedString::default()).collect();

        // The preview: one cell, or two by two when the selection has
        // inside edges, with each edge as it would be drawn.
        let edges = self.edges;
        let (many_rows, many_cols) = (self.many_rows, self.many_cols);
        let text = t("Text", "Texte");
        let drawing = preview(190.0, 120.0, move |s: &mut Sketch| {
            let (x0, y0, x1, y1) = (14.0, 14.0, s.width - 14.0, s.height - 14.0);
            let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            // Corner marks, as in Excel's diagram.
            let mark = c(0x9A9A9A);
            for (x, y, dx, dy) in [(x0, y0, -1.0, -1.0), (x1, y0, 1.0, -1.0), (x0, y1, -1.0, 1.0), (x1, y1, 1.0, 1.0)] {
                s.line((x, y + dy * 2.0), (x, y + dy * 9.0), 1.0, mark);
                s.line((x + dx * 2.0, y), (x + dx * 9.0, y), 1.0, mark);
            }
            let font = widgets::ui_font();
            let width = s.measure(text, &font, 12.0);
            let cells: Vec<(f32, f32)> = match (many_cols, many_rows) {
                (false, false) => vec![(mx, my)],
                (true, false) => vec![((x0 + mx) / 2.0, my), ((mx + x1) / 2.0, my)],
                (false, true) => vec![(mx, (y0 + my) / 2.0), (mx, (my + y1) / 2.0)],
                (true, true) => vec![
                    ((x0 + mx) / 2.0, (y0 + my) / 2.0),
                    ((mx + x1) / 2.0, (y0 + my) / 2.0),
                    ((x0 + mx) / 2.0, (my + y1) / 2.0),
                    ((mx + x1) / 2.0, (my + y1) / 2.0),
                ],
            };
            for (cx0, cy0) in cells {
                s.text(text, cx0 - width / 2.0, cy0 - 8.0, &font, 12.0, c(0x000000));
            }
            let mut edge = |index: usize, from: (f32, f32), to: (f32, f32)| {
                let horizontal = from.1 == to.1;
                let Some(border) = edges[index].border else {
                    // The cells disagree: a grey line.
                    s.line(from, to, 3.0, c(0xC8C8C8));
                    return;
                };
                let color = c(border.color.map(|col| col.0).unwrap_or(0x000000));
                let mut solid = |offset: f32, width: f32, s: &mut Sketch| {
                    if horizontal {
                        s.rect(from.0, from.1 + offset, to.0, from.1 + offset + width, color);
                    } else {
                        s.rect(from.0 + offset, from.1, from.0 + offset + width, to.1, color);
                    }
                };
                let mut dashed = |on: f32, off: f32, s: &mut Sketch| {
                    let (mut p, end) = if horizontal { (from.0, to.0) } else { (from.1, to.1) };
                    while p < end {
                        let e = (p + on).min(end);
                        if horizontal {
                            s.rect(p, from.1, e, from.1 + 1.0, color);
                        } else {
                            s.rect(from.0, p, from.0 + 1.0, e, color);
                        }
                        p += on + off;
                    }
                };
                match border.style {
                    BorderStyle::None => {}
                    BorderStyle::Hair => dashed(1.0, 1.0, s),
                    BorderStyle::Dotted => dashed(2.0, 2.0, s),
                    BorderStyle::Dashed => dashed(4.0, 2.0, s),
                    BorderStyle::Thin => solid(0.0, 1.0, s),
                    BorderStyle::Medium => solid(-0.5, 2.0, s),
                    BorderStyle::Thick => solid(-1.0, 3.0, s),
                    BorderStyle::Double => {
                        solid(-1.0, 1.0, s);
                        solid(1.0, 1.0, s);
                    }
                }
            };
            edge(TOP, (x0, y0), (x1, y0));
            edge(BOTTOM, (x0, y1), (x1, y1));
            edge(LEFT, (x0, y0), (x0, y1));
            edge(RIGHT, (x1, y0), (x1, y1));
            if many_rows {
                edge(INNER_H, (x0, my), (x1, my));
            }
            if many_cols {
                edge(INNER_V, (mx, y0), (mx, y1));
            }
        });
        let bounds = self.preview_bounds.clone();
        let diagram = div()
            .id("border-preview")
            .relative()
            .bg(c(0xFFFFFF))
            .border_1()
            .border_color(c(theme::BORDER_STRONG))
            .child(drawing)
            .child(
                canvas(move |b, _, _| bounds.set(b), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| {
                    // The edge nearest to the pointer, if it is close.
                    let b = this.preview_bounds.get();
                    let (w, h) = (f32::from(b.size.width), f32::from(b.size.height));
                    let (x, y) = (f32::from(ev.position.x - b.left()), f32::from(ev.position.y - b.top()));
                    let (x0, y0, x1, y1) = (14.0, 14.0, w - 14.0, h - 14.0);
                    let candidates = [
                        (TOP, (y - y0).abs()),
                        (BOTTOM, (y - y1).abs()),
                        (LEFT, (x - x0).abs()),
                        (RIGHT, (x - x1).abs()),
                        (INNER_H, (y - (y0 + y1) / 2.0).abs()),
                        (INNER_V, (x - (x0 + x1) / 2.0).abs()),
                    ];
                    let nearest = candidates
                        .iter()
                        .filter(|(i, _)| (*i != INNER_H || this.many_rows) && (*i != INNER_V || this.many_cols))
                        .min_by(|a, b| a.1.total_cmp(&b.1));
                    if let Some((index, distance)) = nearest {
                        if *distance <= 10.0 {
                            this.toggle_edge(*index, cx);
                        }
                    }
                }),
            );

        // The buttons around the diagram. The two diagonals are there for
        // the shape of Excel's tab; the workbook model has no diagonal
        // borders, so they stay disabled.
        let edge_button = |id: &'static str, icon_name: &'static str, tip: &'static str, index: Option<usize>, this: &Self, cx: &mut Context<Self>| {
            let enabled = match index {
                Some(INNER_H) => this.many_rows,
                Some(INNER_V) => this.many_cols,
                Some(_) => true,
                None => false,
            };
            let on = index.is_some_and(|i| this.edges[i].border.is_some_and(|b| !b.is_none()));
            div()
                .id(id)
                .size(px(26.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .border_1()
                .border_color(c(if on { theme::ACCENT } else { theme::BORDER_STRONG }))
                .bg(c(if on { theme::ACCENT_LIGHT } else { 0xFFFFFF }))
                .when(enabled, |d| d.hover(|s| s.bg(c(theme::HOVER))))
                .child(icon(icon_name, 16.0, if enabled { theme::TEXT } else { theme::TEXT_DISABLED }))
                .tooltip(tooltip(tip))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(index) = index {
                        this.toggle_edge(index, cx);
                    }
                }))
        };
        let preset = |id: &'static str, icon_name: &'static str, text: &'static str, enabled: bool, cx: &mut Context<Self>, f: fn(&mut Self, &mut Context<Self>)| {
            div()
                .id(id)
                .w(px(64.))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(2.))
                .child(
                    div()
                        .size(px(34.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(3.))
                        .border_1()
                        .border_color(c(theme::BORDER_STRONG))
                        .bg(c(0xFFFFFF))
                        .when(enabled, |d| d.hover(|s| s.bg(c(theme::HOVER))))
                        .child(icon(icon_name, 22.0, if enabled { theme::TEXT } else { theme::TEXT_DISABLED })),
                )
                .child(div().when(!enabled, |d| d.text_color(c(theme::TEXT_DISABLED))).child(text))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if enabled {
                        f(this, cx)
                    }
                }))
        };
        let has_inside = self.many_rows || self.many_cols;

        div()
            .flex()
            .gap(px(20.))
            .child(
                div().w(px(150.)).child(
                    GroupBox::new(t("Line", "Ligne"))
                        .child(Self::titled(
                            t("Style:", "Style :"),
                            ListBox::new("line-style", names, style_ix)
                                .height(widgets::ROW_H * 8.0)
                                .row(move |i, _| div().h_full().flex().items_center().child(sample(LINE_STYLES[i])).into_any_element())
                                .on_change(cx.listener(|this, ix: &usize, _, cx| {
                                    this.line_style = LINE_STYLES[*ix];
                                    cx.notify();
                                })),
                        ))
                        .child(Self::titled(
                            t("Color:", "Couleur :"),
                            ColorButton::new("line-color", Some(self.line_color))
                                .none_label(t("Automatic", "Automatique"))
                                .more(&self.modal.app)
                                .width(130.0)
                                .on_pick(cx.listener(|this, color: &Option<Color>, _, cx| {
                                    this.line_color = *color;
                                    cx.notify();
                                })),
                        )),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(
                        GroupBox::new(t("Presets", "Présélections")).child(
                            div()
                                .flex()
                                .gap(px(12.))
                                .child(preset("preset-none", "border-none", t("None", "Aucune"), true, cx, |this, cx| {
                                    this.set_edges(&[TOP, BOTTOM, LEFT, RIGHT, INNER_H, INNER_V], Border::NONE, cx)
                                }))
                                .child(preset(
                                    "preset-outline",
                                    "border-outside",
                                    t("Outline", "Contour"),
                                    true,
                                    cx,
                                    |this, cx| {
                                        let line = Border {
                                            style: this.line_style,
                                            color: this.line_color,
                                        };
                                        this.set_edges(&[TOP, BOTTOM, LEFT, RIGHT], line, cx)
                                    },
                                ))
                                .child(preset(
                                    "preset-inside",
                                    "border-inside",
                                    t("Inside", "Intérieur"),
                                    has_inside,
                                    cx,
                                    |this, cx| {
                                        let line = Border {
                                            style: this.line_style,
                                            color: this.line_color,
                                        };
                                        this.set_edges(&[INNER_H, INNER_V], line, cx)
                                    },
                                )),
                        ),
                    )
                    .child(
                        GroupBox::new(t("Border", "Bordure"))
                            .child(
                                div()
                                    .flex()
                                    .gap(px(8.))
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .justify_between()
                                            .child(edge_button("edge-top", "border-top", t("Top Border", "Bordure supérieure"), Some(TOP), self, cx))
                                            .child(edge_button(
                                                "edge-inner-h",
                                                "border-inner-h",
                                                t("Inside Horizontal Border", "Bordure intérieure horizontale"),
                                                Some(INNER_H),
                                                self,
                                                cx,
                                            ))
                                            .child(edge_button(
                                                "edge-bottom",
                                                "border-bottom",
                                                t("Bottom Border", "Bordure inférieure"),
                                                Some(BOTTOM),
                                                self,
                                                cx,
                                            )),
                                    )
                                    .child(diagram),
                            )
                            .child(
                                div()
                                    .pl(px(34.))
                                    .w(px(226.))
                                    .flex()
                                    .justify_between()
                                    .child(edge_button(
                                        "edge-diag-up",
                                        "border-diag-up",
                                        t("Diagonal Up Border", "Bordure diagonale montante"),
                                        None,
                                        self,
                                        cx,
                                    ))
                                    .child(edge_button("edge-left", "border-left", t("Left Border", "Bordure gauche"), Some(LEFT), self, cx))
                                    .child(edge_button(
                                        "edge-inner-v",
                                        "border-inner-v",
                                        t("Inside Vertical Border", "Bordure intérieure verticale"),
                                        Some(INNER_V),
                                        self,
                                        cx,
                                    ))
                                    .child(edge_button("edge-right", "border-right", t("Right Border", "Bordure droite"), Some(RIGHT), self, cx))
                                    .child(edge_button(
                                        "edge-diag-down",
                                        "border-diag-down",
                                        t("Diagonal Down Border", "Bordure diagonale descendante"),
                                        None,
                                        self,
                                        cx,
                                    )),
                            ),
                    )
                    .child(div().text_color(c(theme::TEXT_MUTED)).line_height(px(16.)).child(t(
                        "The selected border style can be applied by clicking the presets, preview diagram or the buttons above.",
                        "Le style de bordure sélectionné peut être appliqué en cliquant sur l'une des présélections, sur l'aperçu ou les boutons ci-dessus.",
                    ))),
            )
            .into_any_element()
    }

    // ---- Fill ---------------------------------------------------------------------

    fn fill_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let shown = if self.fill.mixed && !self.fill.changed { None } else { self.fill.value };
        div()
            .flex()
            .gap(px(28.))
            .child(Self::titled(
                t("Background Color:", "Couleur d'arrière-plan :"),
                ColorGrid::new("fill", shown)
                    .none_label(t("No Color", "Aucune couleur"))
                    .more(&self.modal.app)
                    .on_pick(cx.listener(|this, color: &Option<Color>, _, cx| {
                        this.fill.set(*color);
                        cx.notify();
                    })),
            ))
            .child(
                div().flex_1().child(
                    GroupBox::new(t("Sample", "Exemple")).child(
                        div()
                            .h(px(60.))
                            .border_1()
                            .border_color(c(theme::BORDER_STRONG))
                            .bg(c(shown.flatten().map(|col| col.0).unwrap_or(0xFFFFFF))),
                    ),
                ),
            )
            .into_any_element()
    }

    // ---- Protection ---------------------------------------------------------------

    fn protection_tab(&mut self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .flex_col()
            .gap(px(8.))
            .child(self.check("locked", t("Locked", "Verrouillée"), &self.locked, |d| &mut d.locked, cx))
            .child(self.check("hidden", t("Hidden", "Masquée"), &self.hidden, |d| &mut d.hidden, cx))
            .child(div().pt(px(8.)).max_w(px(420.)).line_height(px(17.)).child(t(
                "Locking cells or hiding formulas has no effect until you protect the worksheet (Review tab, Protect group, Protect Sheet button).",
                "Le verrouillage des cellules ou le masquage des formules sont sans effet si la feuille n'est pas protégée (onglet Révision, groupe Protéger, bouton Protéger la feuille).",
            )))
            .into_any_element()
    }
}

impl Render for Dialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs: Vec<SharedString> = [
            crate::i18n::t_ctx("format", "Number", "Nombre"),
            t("Alignment", "Alignement"),
            t("Font", "Police"),
            t("Border", "Bordure"),
            t("Fill", "Remplissage"),
            t("Protection", "Protection"),
        ]
        .iter()
        .map(|name| (*name).into())
        .collect();
        let body = match self.tab {
            TAB_ALIGNMENT => self.alignment_tab(cx),
            TAB_FONT => self.font_tab(cx),
            TAB_BORDER => self.border_tab(cx),
            TAB_FILL => self.fill_tab(cx),
            TAB_PROTECTION => self.protection_tab(cx),
            _ => self.number_tab(cx),
        };
        dialog(&self.modal, t("Format Cells", "Format de cellule"))
            .width(560.0)
            .on_ok(cx.listener(Self::ok))
            .child(
                TabStrip::new("tabs", tabs, self.tab).on_change(cx.listener(|this, ix: &usize, _, cx| {
                    this.tab = *ix;
                    // The dialog reopens on the tab it was left on.
                    let tab = *ix;
                    this.modal.update_app(cx, |app, _| app.ext_mut::<Memory>().tab = tab);
                    cx.notify();
                })),
            )
            .child(div().h(px(352.)).flex().flex_col().font_weight(FontWeight::NORMAL).child(body))
            .footer(ok_cancel(&self.modal, cx.listener(Self::ok)))
    }
}
