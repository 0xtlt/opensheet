//! Insert Function (the fx button) and Function Arguments.
//!
//! Function Arguments works on the formula being typed: the cell is in
//! edit mode behind the dialog and every change of an argument is written
//! into it, so the formula bar, the cell and the coloured ranges follow.
//! OK enters the formula the way Return would; Cancel puts back what was
//! there.

use super::fn_help::{self, FunctionHelp};
use crate::app::{EditMode, EditSource, ExcelApp};
use crate::features::{Cx, names};
use crate::i18n::{t, t_ctx};
use crate::shell::{Modal, RefStyle};
use crate::text_input::{InputEvent, TextInput};
use crate::theme::{self, c};
use crate::widgets::{self, Button, ListBox, RefEdit, ScrollArea, Select, TextField, dialog, label};
use gpui::{AnyElement, Context, Entity, Focusable, FontWeight, SharedString, Window, div, prelude::*, px};
use opensheet_engine::eval::Calc;
use opensheet_engine::functions::{self, Category, FunctionDef};
use opensheet_engine::parser::parse_formula;
use opensheet_engine::{Locale, Workbook};
use std::ops::Range;
use std::sync::Mutex;

// ------------------------------------------------------- calls in formula text

/// A function call found in formula text.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Call {
    /// Bytes of the whole call: name, parentheses and arguments.
    pub range: Range<usize>,
    /// The name as typed.
    pub name: String,
    /// The arguments as typed, without the spaces around them.
    pub args: Vec<String>,
}

/// The function calls of a formula, in the order their names appear. A
/// call still open at the end of the text (a formula being typed) ends
/// there.
pub(crate) fn calls(text: &str, locale: &Locale) -> Vec<Call> {
    /// A parenthesis that is open: the call it belongs to, if it follows
    /// a name, and where its arguments start.
    struct Open {
        name_start: Option<usize>,
        /// Where the parenthesis is.
        paren: usize,
        arg_start: usize,
        args: Vec<String>,
        order: usize,
    }
    let mut out: Vec<(usize, Call)> = Vec::new();
    let mut stack: Vec<Open> = Vec::new();
    // Inside "text", 'a sheet name', [a column] or {an array}, nothing is
    // a parenthesis or a separator.
    let mut quote: Option<char> = None;
    let (mut brackets, mut braces) = (0u32, 0u32);
    let mut name_start: Option<usize> = None;
    let mut order = 0;
    let close = |open: Open, end: usize, text_end: usize, out: &mut Vec<(usize, Call)>| {
        let Open {
            name_start: Some(start),
            paren,
            arg_start,
            mut args,
            order,
        } = open
        else {
            return;
        };
        let last = text[arg_start..end].trim();
        if !(args.is_empty() && last.is_empty()) {
            args.push(last.to_string());
        }
        let call = Call {
            range: start..text_end,
            name: text[start..paren].to_string(),
            args,
        };
        out.push((order, call));
    };
    // In a structured reference, an apostrophe says the next character
    // is not a bracket.
    let mut escaped = false;
    for (at, ch) in text.char_indices() {
        if let Some(q) = quote {
            if ch == q {
                quote = None;
            }
            name_start = None;
            continue;
        }
        if std::mem::take(&mut escaped) {
            continue;
        }
        match ch {
            '\'' if brackets > 0 => escaped = true,
            '"' | '\'' => quote = Some(ch),
            '[' => brackets += 1,
            ']' => brackets = brackets.saturating_sub(1),
            '{' if brackets == 0 => braces += 1,
            '}' if brackets == 0 => braces = braces.saturating_sub(1),
            _ => {}
        }
        if quote.is_some() || brackets > 0 || braces > 0 {
            name_start = None;
            continue;
        }
        match ch {
            '(' => {
                stack.push(Open {
                    name_start: name_start.take(),
                    paren: at,
                    arg_start: at + 1,
                    args: Vec::new(),
                    order,
                });
                order += 1;
            }
            ')' => {
                if let Some(open) = stack.pop() {
                    close(open, at, at + 1, &mut out);
                }
                name_start = None;
            }
            ch if ch == locale.arg_sep => {
                if let Some(open) = stack.last_mut().filter(|open| open.name_start.is_some()) {
                    open.args.push(text[open.arg_start..at].trim().to_string());
                    open.arg_start = at + 1;
                }
                name_start = None;
            }
            ch if ch.is_alphanumeric() || ch == '_' || ch == '.' => {
                // Names start with a letter: `2(` is not a call.
                if name_start.is_none() && (ch.is_alphabetic() || ch == '_') {
                    name_start = Some(at);
                }
            }
            _ => name_start = None,
        }
    }
    while let Some(open) = stack.pop() {
        close(open, text.len(), text.len(), &mut out);
    }
    out.sort_by_key(|(order, _)| *order);
    out.into_iter().map(|(_, call)| call).collect()
}

/// The innermost call a caret is in (on its name or between its
/// parentheses).
pub(crate) fn call_at(text: &str, caret: usize, locale: &Locale) -> Option<Call> {
    calls(text, locale)
        .into_iter()
        .filter(|call| call.range.start <= caret && caret <= call.range.end)
        .min_by_key(|call| call.range.len())
}

/// The function a call names, in the language of the workbook.
fn function_of(call: &Call, locale: &Locale) -> Option<&'static FunctionDef> {
    functions::lookup(&locale.fn_to_canonical(&call.name.to_uppercase()))
}

// ------------------------------------------------------------------- memory

/// What the dialogs remember from one use to the next, whatever the
/// workbook.
struct Memory {
    /// Functions last inserted, most recent first.
    recent: Vec<&'static str>,
    /// Category the Insert Function dialog was left on.
    category: usize,
}

/// The list Excel starts with.
const RECENT_AT_START: [&str; 10] = ["SUM", "AVERAGE", "IF", "HYPERLINK", "COUNT", "MAX", "SIN", "SUMIF", "PMT", "STDEV"];

static MEMORY: Mutex<Option<Memory>> = Mutex::new(None);

fn with_memory<R>(f: impl FnOnce(&mut Memory) -> R) -> R {
    let mut guard = MEMORY.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let memory = guard.get_or_insert_with(|| Memory {
        recent: RECENT_AT_START.iter().filter_map(|name| functions::lookup(name)).map(|def| def.name).collect(),
        category: 0,
    });
    f(memory)
}

fn remember(function: &'static str) {
    with_memory(|memory| {
        memory.recent.retain(|name| *name != function);
        memory.recent.insert(0, function);
        memory.recent.truncate(RECENT_AT_START.len());
    });
}

// ---------------------------------------------------------- Insert Function

/// The categories of the list, after "Most Recently Used" and "All".
const CATEGORIES: [Category; 9] = [
    Category::Financial,
    Category::DateTime,
    Category::Math,
    Category::Statistical,
    Category::Lookup,
    Category::Text,
    Category::Logical,
    Category::Information,
    Category::Engineering,
];

fn category_name(category: Category) -> &'static str {
    match category {
        Category::Financial => t_ctx("category", "Financial", "Finances"),
        Category::DateTime => t_ctx("category", "Date & Time", "Date et Heure"),
        Category::Math => t_ctx("category", "Math & Trig", "Math et Trigo"),
        Category::Statistical => t("Statistical", "Statistiques"),
        Category::Lookup => t_ctx("category", "Lookup & Reference", "Recherche & référence"),
        Category::Text => t("Text", "Texte"),
        Category::Logical => t("Logical", "Logique"),
        Category::Information => t("Information", "Informations"),
        Category::Engineering => t("Engineering", "Ingénierie"),
    }
}

/// The functions of an entry of the category list (0: most recently used,
/// 1: all), by their name in the language of the workbook.
fn functions_in(category: usize, locale: &Locale) -> Vec<&'static FunctionDef> {
    if category == 0 {
        return with_memory(|memory| memory.recent.iter().filter_map(|name| functions::lookup(name)).collect());
    }
    let wanted = CATEGORIES.get(category.wrapping_sub(2)).copied();
    let mut list: Vec<&'static FunctionDef> = functions::all()
        .into_iter()
        .filter(|def| wanted.is_none_or(|category| def.category == category))
        .collect();
    list.sort_by_key(|def| locale.fn_to_local(def.name));
    list
}

/// The functions answering a search: every word typed is found in their
/// name (in English or in the language of the workbook) or in what Excel
/// says about them.
pub(crate) fn search(query: &str, locale: &Locale) -> Vec<&'static FunctionDef> {
    let words: Vec<String> = query.split_whitespace().map(|word| word.to_lowercase()).collect();
    if words.is_empty() {
        return Vec::new();
    }
    let mut list: Vec<&'static FunctionDef> = functions::all()
        .into_iter()
        .filter(|def| {
            let mut haystack = format!("{} {}", def.name, locale.fn_to_local(def.name)).to_lowercase();
            if let Some(help) = fn_help::help(def.name) {
                haystack.push(' ');
                haystack.push_str(&help.description.to_lowercase());
            }
            words.iter().all(|word| haystack.contains(word))
        })
        .collect();
    list.sort_by_key(|def| locale.fn_to_local(def.name));
    list
}

/// The fx button: the arguments of the function the caret is in, else the
/// list of functions to choose one from.
pub(super) fn insert_function(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    if app.modal_open() {
        return;
    }
    if app.is_editing() {
        let original = app.edit_text(cx);
        if open_arguments(app, Restore::Text(original), window, cx) {
            return;
        }
    } else {
        // A cell holding a formula: its first function.
        let (row, col) = app.active();
        let first = app
            .wb
            .formula_text(app.sheet, row, col)
            .and_then(|text| calls(&text, app.wb.locale).into_iter().find(|call| function_of(call, app.wb.locale).is_some()));
        if let Some(call) = first {
            app.start_edit(None, EditMode::Edit, EditSource::Cell, window, cx);
            if !app.is_editing() {
                return;
            }
            let caret = call.range.start;
            app.editor().clone().update(cx, |input, cx| input.set_selection(caret..caret, cx));
            if open_arguments(app, Restore::CancelEdit, window, cx) {
                return;
            }
            app.cancel_edit(window, cx);
        }
    }
    let locale = app.wb.locale;
    app.open_modal(window, cx, move |modal, _, cx| InsertFunction::new(modal, locale, cx));
}

/// Put a function into the formula (starting one in a cell that is not
/// being edited) and open its arguments: what choosing a function in the
/// Insert Function dialog or in a menu of the Function Library does.
pub fn function_arguments(app: &mut ExcelApp, function: &'static str, window: &mut Window, cx: &mut Cx) {
    if app.modal_open() {
        return;
    }
    let restore = if app.is_editing() {
        let original = app.edit_text(cx);
        let name = app.wb.locale.fn_to_local(function);
        let text = format!("{}{name}()", if original.is_empty() { "=" } else { "" });
        app.editor().clone().update(cx, |input, cx| {
            input.insert(&text, window, cx);
            // Between the parentheses.
            let caret = input.selection().start.saturating_sub(1);
            input.set_selection(caret..caret, cx);
        });
        Restore::Text(original)
    } else {
        app.insert_function(function, window, cx);
        Restore::CancelEdit
    };
    if app.is_editing() && open_arguments(app, restore, window, cx) {
        remember(function);
    }
}

struct InsertFunction {
    modal: Modal,
    locale: &'static Locale,
    search: Entity<TextInput>,
    /// Entry of the category list; the list starts with "Recommended"
    /// once a search was made.
    category: usize,
    found: Option<Vec<&'static FunctionDef>>,
    selected: Option<usize>,
}

impl InsertFunction {
    fn new(modal: Modal, locale: &'static Locale, cx: &mut Context<Self>) -> Self {
        let search = widgets::input(cx, "");
        search.update(cx, |input, _| {
            input.placeholder = t(
                "Type a brief description of what you want to do and then click Go",
                "Tapez une brève description de ce que vous voulez faire, puis cliquez sur OK",
            )
            .into()
        });
        InsertFunction {
            modal,
            locale,
            search,
            category: with_memory(|memory| memory.category),
            found: None,
            selected: Some(0),
        }
    }

    /// The functions listed: the answers of the search while its category
    /// is chosen, else those of the category.
    fn listed(&self) -> Vec<&'static FunctionDef> {
        match &self.found {
            Some(found) if self.category == 0 => found.clone(),
            Some(_) => functions_in(self.category - 1, self.locale),
            None => functions_in(self.category, self.locale),
        }
    }

    fn go(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).text().to_string();
        let found = search(&query, self.locale);
        // "Recommended" joins the categories, and is chosen.
        self.category = 0;
        self.selected = (!found.is_empty()).then_some(0);
        self.found = Some(found);
        cx.notify();
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        // Return in the search box searches.
        if self.search.read(cx).focus_handle(cx).is_focused(window) && !self.search.read(cx).text().trim().is_empty() {
            self.go(cx);
            return;
        }
        let Some(def) = self.selected.and_then(|index| self.listed().get(index).copied()) else {
            return;
        };
        let category = if self.found.is_some() { self.category.saturating_sub(1) } else { self.category };
        with_memory(|memory| memory.category = category);
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| function_arguments(app, def.name, window, cx));
    }
}

impl Render for InsertFunction {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let locale = self.locale;
        let mut categories: Vec<SharedString> = vec![
            t("Most Recently Used", "Les dernières utilisées").into(),
            t_ctx("category", "All", "Toutes").into(),
        ];
        categories.extend(CATEGORIES.iter().map(|category| SharedString::from(category_name(*category))));
        if self.found.is_some() {
            categories.insert(0, t("Recommended", "Recommandé").into());
        }
        let listed = self.listed();
        let names: Vec<SharedString> = listed.iter().map(|def| SharedString::from(locale.fn_to_local(def.name))).collect();
        let chosen = self.selected.and_then(|index| listed.get(index).copied());
        let help = chosen.and_then(|def| fn_help::help(def.name));
        let syntax = chosen.map(|def| fn_help::syntax(def, help, locale)).unwrap_or_default();
        let description = help.map(|help| help.description.clone()).unwrap_or_default();

        dialog(&self.modal, t("Insert Function", "Insérer une fonction"))
            .width(470.0)
            .on_ok(cx.listener(Self::ok))
            .child(label(t("Search for a function:", "Recherchez une fonction :")))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(TextField::new(&self.search))
                    .child(Button::new("go", t("Go", "OK")).width(60.0).on_click(cx.listener(|this, _, _, cx| this.go(cx)))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(label(t("Or select a category:", "Ou sélectionnez une catégorie :")))
                    .child(
                        Select::new("category", categories, Some(self.category))
                            .width(220.0)
                            .on_change(cx.listener(|this, index: &usize, _, cx| {
                                this.category = *index;
                                this.selected = Some(0);
                                cx.notify();
                            })),
                    ),
            )
            .child(label(t("Select a function:", "Sélectionnez une fonction :")))
            .child(
                div().flex().child(
                    ListBox::new("functions", names, self.selected)
                        .height(140.0)
                        .on_change(cx.listener(|this, index: &usize, _, cx| {
                            this.selected = Some(*index);
                            cx.notify();
                        }))
                        .on_activate(cx.listener(|this, index: &usize, window, cx| {
                            this.selected = Some(*index);
                            this.ok(&(), window, cx);
                        })),
                ),
            )
            .child(
                div()
                    .h(px(76.))
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .overflow_hidden()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child(SharedString::from(syntax)))
                    .child(div().line_height(px(16.)).child(SharedString::from(description))),
            )
            .footer(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        Button::new("ok", t("OK", "OK"))
                            .primary()
                            .disabled(chosen.is_none())
                            .on_click(cx.listener(Self::ok)),
                    )
                    .child(Button::new("cancel", t("Cancel", "Annuler")).on_click(cx.listener(|this, _, window, cx| {
                        this.modal.close(window, cx);
                    }))),
            )
    }
}

// -------------------------------------------------------- Function Arguments

/// What Cancel does to the entry in progress.
#[derive(Clone)]
enum Restore {
    /// The dialog started the entry: the cell is left as it was.
    CancelEdit,
    /// The user was typing: the text goes back to this.
    Text(String),
}

/// Open Function Arguments on the call the caret of the entry in progress
/// is in. Returns false when it is in none (or in an unknown function).
fn open_arguments(app: &mut ExcelApp, restore: Restore, window: &mut Window, cx: &mut Cx) -> bool {
    let Some(edit) = &app.editing else {
        return false;
    };
    let cell = (edit.sheet, edit.row, edit.col);
    let locale = app.wb.locale;
    let editor = app.editor().clone();
    let text = editor.read(cx).text().to_string();
    let caret = editor.read(cx).selection().start;
    let Some(site) = call_site(&text, caret, locale) else {
        return false;
    };
    let init = ArgsInit {
        def: site.def,
        locale,
        prefix: site.prefix,
        suffix: site.suffix,
        args: site.args,
        cell,
        restore,
        shown: text,
    };
    app.open_modal(window, cx, move |modal, _, cx| FunctionArgs::new(modal, init, cx));
    true
}

/// The call of a known function a caret is in, and the formula around it.
struct CallSite {
    def: &'static FunctionDef,
    /// The formula before the call, from its `=`.
    prefix: String,
    args: Vec<String>,
    /// The formula after the call.
    suffix: String,
}

/// What Function Arguments works on in a formula being typed: the
/// innermost call of a function of the engine around the caret. `None`
/// for a text that is not a formula, a caret outside any call, and
/// functions the engine does not have.
fn call_site(text: &str, caret: usize, locale: &Locale) -> Option<CallSite> {
    if !text.starts_with('=') {
        return None;
    }
    let call = call_at(text, caret, locale)?;
    Some(CallSite {
        def: function_of(&call, locale)?,
        prefix: text[..call.range.start].to_string(),
        suffix: text[call.range.end..].to_string(),
        args: call.args,
    })
}

/// The formula Function Arguments writes back: the call with its
/// arguments (the empty ones at the end left out) between what came
/// before and after it.
fn compose(prefix: &str, def: &FunctionDef, args: &[String], suffix: &str, locale: &Locale) -> String {
    let used = args.iter().rposition(|arg| !arg.trim().is_empty()).map_or(0, |last| last + 1);
    let args: Vec<&str> = args[..used].iter().map(|arg| arg.trim()).collect();
    let sep = locale.arg_sep.to_string();
    format!("{prefix}{}({}){suffix}", locale.fn_to_local(def.name), args.join(&sep))
}

struct ArgsInit {
    def: &'static FunctionDef,
    locale: &'static Locale,
    prefix: String,
    suffix: String,
    args: Vec<String>,
    cell: (usize, u32, u32),
    restore: Restore,
    /// The formula as the cell shows it when the dialog opens.
    shown: String,
}

/// The value of an argument (or of the whole call) typed in the dialog,
/// as Excel writes it after the "=" sign; empty for an empty box.
pub(crate) fn preview(wb: &Workbook, cell: (usize, u32, u32), text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }
    match parse_formula(text, wb.locale) {
        Ok(expr) => names::literal_value(&Calc::eval_once(wb, cell.0, cell.1, cell.2, &expr), wb.locale),
        Err(_) => t("Invalid", "Non valide").to_string(),
    }
}

struct FunctionArgs {
    modal: Modal,
    def: &'static FunctionDef,
    help: Option<&'static FunctionHelp>,
    locale: &'static Locale,
    prefix: String,
    suffix: String,
    fields: Vec<Entity<TextInput>>,
    cell: (usize, u32, u32),
    restore: Restore,
    /// The formula last written into the cell.
    shown: String,
    /// The argument the keyboard was last in.
    current: usize,
    /// Text and value of each box and of the whole call, as last worked
    /// out: values are only calculated again when a text changes.
    previews: Vec<(String, String)>,
    result: (String, String),
}

impl FunctionArgs {
    fn new(modal: Modal, init: ArgsInit, cx: &mut Context<Self>) -> Self {
        let mut dialog = FunctionArgs {
            help: fn_help::help(init.def.name),
            locale: init.locale,
            modal,
            def: init.def,
            prefix: init.prefix,
            suffix: init.suffix,
            fields: Vec::new(),
            cell: init.cell,
            restore: init.restore,
            shown: init.shown,
            current: 0,
            previews: Vec::new(),
            result: (String::new(), String::new()),
        };
        for arg in init.args {
            dialog.add_field(arg, cx);
        }
        dialog.grow(cx);
        dialog
    }

    fn add_field(&mut self, text: String, cx: &mut Context<Self>) {
        let style = RefStyle {
            absolute: false,
            sheet: false,
            equals: false,
        };
        let field = self.modal.ref_input(style, text, cx);
        // Typing and picking cells both end here.
        cx.observe(&field, |this, _, cx| this.changed(cx)).detach();
        self.fields.push(field);
    }

    /// Show as many boxes as the function names, and always one to go on
    /// with while it takes more.
    fn grow(&mut self, cx: &mut Context<Self>) {
        let filled = self.fields.iter().rposition(|field| !field.read(cx).text().trim().is_empty()).map_or(0, |i| i + 1);
        let wanted = fn_help::shown_arguments(self.def, self.help, filled);
        while self.fields.len() < wanted {
            self.add_field(String::new(), cx);
        }
    }

    /// The call as the boxes spell it.
    fn call(&self, cx: &Context<Self>) -> String {
        let args: Vec<String> = self.fields.iter().map(|field| field.read(cx).text().to_string()).collect();
        compose("", self.def, &args, "", self.locale)
    }

    /// An argument changed: the formula in the cell follows.
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.grow(cx);
        let formula = format!("{}{}{}", self.prefix, self.call(cx), self.suffix);
        if formula != self.shown {
            self.show(formula, cx);
        }
        cx.notify();
    }

    /// Write a formula into the entry in progress, as if it were typed.
    fn show(&mut self, formula: String, cx: &mut Context<Self>) {
        self.shown = formula.clone();
        self.modal.update_app(cx, |app, cx| {
            app.editor().clone().update(cx, |input, cx| {
                input.set_text(formula, cx);
                cx.emit(InputEvent::Changed);
            });
        });
    }

    fn ok(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        self.changed(cx);
        remember(self.def.name);
        self.modal.close(window, cx);
        self.modal.update_app(cx, |app, cx| {
            app.commit_edit(None, window, cx);
        });
    }

    fn cancel(&mut self, _: &(), window: &mut Window, cx: &mut Context<Self>) {
        let restore = self.restore.clone();
        if let Restore::Text(original) = &restore {
            self.show(original.clone(), cx);
        }
        self.modal.close(window, cx);
        if matches!(restore, Restore::CancelEdit) {
            self.modal.update_app(cx, |app, cx| app.cancel_edit(window, cx));
        }
    }

    /// Bring the values shown up to date with the texts of the boxes.
    fn refresh_previews(&mut self, cx: &mut Context<Self>) {
        let texts: Vec<String> = self.fields.iter().map(|field| field.read(cx).text().to_string()).collect();
        let call = self.call(cx);
        let cell = self.cell;
        self.previews.truncate(texts.len());
        let stale: Vec<usize> = (0..texts.len())
            .filter(|i| self.previews.get(*i).is_none_or(|(text, _)| *text != texts[*i]))
            .collect();
        let result_stale = self.result.0 != call;
        if stale.is_empty() && !result_stale {
            return;
        }
        let fresh = self.modal.read_app(cx, |app| {
            let values: Vec<String> = stale.iter().map(|i| preview(&app.wb, cell, &texts[*i])).collect();
            let result = result_stale.then(|| preview(&app.wb, cell, &call));
            (values, result)
        });
        let Some((values, result)) = fresh else {
            return;
        };
        for (i, value) in stale.into_iter().zip(values) {
            let entry = (texts[i].clone(), value);
            match self.previews.get_mut(i) {
                Some(slot) => *slot = entry,
                None => self.previews.push(entry),
            }
        }
        if let Some(result) = result {
            self.result = (call, result);
        }
    }
}

/// A name with a capital, as the dialog labels the arguments.
fn capitalized(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

impl Render for FunctionArgs {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh_previews(cx);
        if let Some(focused) = self.fields.iter().position(|field| field.read(cx).focus_handle(cx).is_focused(window)) {
            self.current = focused;
        }
        let args = fn_help::arguments(self.def, self.help, self.fields.len());
        let mut rows: Vec<AnyElement> = Vec::new();
        for (i, (field, arg)) in self.fields.iter().zip(&args).enumerate() {
            let value = self.previews.get(i).map(|(_, value)| value.clone()).unwrap_or_default();
            rows.push(
                div()
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .w(px(130.))
                            .flex_none()
                            .flex()
                            .justify_end()
                            .when(!arg.optional, |d| d.font_weight(FontWeight::SEMIBOLD))
                            .child(div().truncate().child(SharedString::from(capitalized(&arg.name)))),
                    )
                    .child(RefEdit::new(field, &self.modal).width(190.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .child(SharedString::from(format!("= {value}"))),
                    )
                    .into_any_element(),
            );
        }
        let boxes: AnyElement = if rows.len() > 5 {
            ScrollArea::new("arguments", 5.0 * 28.0).children(rows).into_any_element()
        } else {
            div().flex().flex_col().children(rows).into_any_element()
        };
        let description = self.help.map(|help| help.description.clone()).unwrap_or_default();
        let about: AnyElement = match args.get(self.current) {
            Some(arg) if !arg.description.is_empty() => div()
                .flex()
                .gap(px(6.))
                .child(
                    div()
                        .w(px(130.))
                        .flex_none()
                        .flex()
                        .justify_end()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(SharedString::from(capitalized(&arg.name))),
                )
                .child(div().flex_1().line_height(px(16.)).child(SharedString::from(arg.description.clone())))
                .into_any_element(),
            // Without Excel's texts, the syntax says what goes where.
            _ if self.help.is_none() => div()
                .text_color(c(theme::TEXT_MUTED))
                .child(SharedString::from(fn_help::syntax(self.def, None, self.locale)))
                .into_any_element(),
            _ => div().into_any_element(),
        };
        let result = self.result.1.clone();

        dialog(&self.modal, t("Function Arguments", "Arguments de la fonction"))
            .width(560.0)
            .on_ok(cx.listener(Self::ok))
            .on_cancel(cx.listener(Self::cancel))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(SharedString::from(self.locale.fn_to_local(self.def.name))),
            )
            .child(boxes)
            .child(
                div()
                    .flex()
                    .justify_end()
                    .pr(px(4.))
                    .child(div().w(px(196.)).truncate().child(SharedString::from(format!("= {result}")))),
            )
            .child(div().min_h(px(32.)).line_height(px(16.)).child(SharedString::from(description)))
            .child(div().min_h(px(48.)).child(about))
            .child(label(format!("{} {result}", t("Formula result =", "Résultat ="))))
            .footer(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(Button::new("ok", t("OK", "OK")).primary().on_click(cx.listener(Self::ok)))
                    .child(Button::new("cancel", t("Cancel", "Annuler")).on_click(cx.listener(Self::cancel))),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(range: Range<usize>, name: &str, args: &[&str]) -> Call {
        Call {
            range,
            name: name.to_string(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
        }
    }

    #[test]
    fn calls_are_found_with_their_arguments() {
        let en = Locale::en();
        assert_eq!(calls("=SUM(A1:A3, B2)", en), [call(1..15, "SUM", &["A1:A3", "B2"])]);
        assert_eq!(calls("=RAND()", en), [call(1..7, "RAND", &[])]);
        assert_eq!(calls("=A1+2*(B1-C1)", en), []);
        assert_eq!(
            calls("=IF(SUM(A1,A2)>3,\"a,b)\",MAX({1,2;3,4}))+1", en),
            [
                call(1..39, "IF", &["SUM(A1,A2)>3", "\"a,b)\"", "MAX({1,2;3,4})"]),
                call(4..14, "SUM", &["A1", "A2"]),
                call(24..38, "MAX", &["{1,2;3,4}"]),
            ]
        );
        // Empty arguments are arguments; a union between parentheses is one.
        assert_eq!(calls("=IF(A1,,2)", en), [call(1..10, "IF", &["A1", "", "2"])]);
        assert_eq!(calls("=SUM((A1,B1),C1)", en), [call(1..16, "SUM", &["(A1,B1)", "C1"])]);
        // Sheet names, structured references and dotted names.
        assert_eq!(
            calls("=T.TEST('My (1), sheet'!A1,Table1[[a,b]:[c]])", en),
            [call(1..45, "T.TEST", &["'My (1), sheet'!A1", "Table1[[a,b]:[c]]"])]
        );
        assert_eq!(calls("=SUM(T[It's '[x'], y],1)", en), [call(1..24, "SUM", &["T[It's '[x'], y]", "1"])]);
    }

    #[test]
    fn calls_follow_the_language_and_unfinished_formulas() {
        let fr = Locale::fr();
        assert_eq!(calls("=SOMME(1,5;A1)", fr), [call(1..14, "SOMME", &["1,5", "A1"])]);
        // Still being typed.
        assert_eq!(calls("=SI(A1>2;SOMME(B1;", fr), [call(1..18, "SI", &["A1>2", "SOMME(B1;"]), call(9..18, "SOMME", &["B1", ""])]);
        assert_eq!(calls("=SOMME(", fr), [call(1..7, "SOMME", &[])]);
    }

    #[test]
    fn the_call_under_the_caret_is_the_innermost() {
        let en = Locale::en();
        let text = "=IF(SUM(A1,A2)>3,1,MAX(B1))+ABS(C1)";
        let name_at = |caret: usize| call_at(text, caret, en).map(|call| call.name);
        assert_eq!(name_at(2).as_deref(), Some("IF"));
        assert_eq!(name_at(5).as_deref(), Some("SUM"));
        assert_eq!(name_at(9).as_deref(), Some("SUM"));
        assert_eq!(name_at(15).as_deref(), Some("IF"));
        assert_eq!(name_at(24).as_deref(), Some("MAX"));
        assert_eq!(name_at(30).as_deref(), Some("ABS"));
        assert_eq!(name_at(0), None);
        assert_eq!(name_at(28).as_deref(), Some("ABS"));
        assert_eq!(call_at("=A1+B1", 3, en), None);
        assert_eq!(function_of(&call_at("=SOMME(A1)", 3, Locale::fr()).unwrap(), Locale::fr()).map(|def| def.name), Some("SUM"));
        assert!(function_of(&call(0..5, "NOPE", &[]), en).is_none());
    }

    #[test]
    fn the_dialog_takes_a_call_apart_and_puts_it_back() {
        let fr = Locale::fr();
        let text = "=A1+SOMME(B1;C1:C3)*2";
        let site = call_site(text, 12, fr).unwrap();
        assert_eq!((site.def.name, site.prefix.as_str(), site.suffix.as_str()), ("SUM", "=A1+", "*2"));
        assert_eq!(site.args, ["B1", "C1:C3"]);
        // Unchanged arguments give the formula back.
        assert_eq!(compose(&site.prefix, site.def, &site.args, &site.suffix, fr), text);
        // Empty boxes at the end are left out, those in the middle kept.
        let args = |list: &[&str]| -> Vec<String> { list.iter().map(|arg| arg.to_string()).collect() };
        let sum = site.def;
        assert_eq!(compose("=", sum, &args(&["A1", " ", ""]), "", fr), "=SOMME(A1)");
        assert_eq!(compose("=", sum, &args(&["", "B2 ", ""]), "", fr), "=SOMME(;B2)");
        assert_eq!(compose("=", sum, &args(&[]), "+1", Locale::en()), "=SUM()+1");
        // A call just started, the caret between its parentheses.
        let fresh = call_site("=RECHERCHEV()", 12, fr).unwrap();
        assert_eq!((fresh.def.name, fresh.args.len()), ("VLOOKUP", 0));
        // Not a formula, no call under the caret, a function nobody knows.
        assert!(call_site("SOMME(A1)", 3, fr).is_none());
        assert!(call_site("=A1+B1", 3, fr).is_none());
        assert!(call_site("=MACHIN(A1)", 4, fr).is_none());
        // Inside an unknown function, the known one around it is not
        // taken for it.
        assert!(call_site("=SOMME(MACHIN(A1))", 15, fr).is_none());
    }

    #[test]
    fn previews_show_values_as_formulas_write_them() {
        let mut wb = Workbook::new(Locale::en());
        for (row, text) in ["4", "5", "six"].iter().enumerate() {
            wb.set_cell_text(0, row as u32, 0, text).unwrap();
        }
        wb.commit("cells");
        let cell = (0, 5, 5);
        assert_eq!(preview(&wb, cell, "A1"), "4");
        assert_eq!(preview(&wb, cell, " A1:A3 "), "{4;5;\"six\"}");
        assert_eq!(preview(&wb, cell, "SUM(A1:A2)*2"), "18");
        assert_eq!(preview(&wb, cell, "\"a\"&A3"), "\"asix\"");
        assert_eq!(preview(&wb, cell, "1/0"), "#DIV/0!");
        assert_eq!(preview(&wb, cell, "A1>3"), "TRUE");
        assert_eq!(preview(&wb, cell, ""), "");
        assert_eq!(preview(&wb, cell, "SUM("), "Invalid");
    }

    #[test]
    fn functions_are_listed_by_category_and_searched() {
        let fr = Locale::fr();
        let all = functions_in(1, fr);
        assert!(all.len() > 250);
        let names: Vec<&str> = all.iter().map(|def| fr.fn_to_local(def.name)).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
        let logical = functions_in(2 + CATEGORIES.iter().position(|c| *c == Category::Logical).unwrap(), fr);
        assert!(logical.iter().all(|def| def.category == Category::Logical));
        assert!(logical.iter().any(|def| def.name == "IF"));
        // The list Excel starts with, as far as the engine has it.
        assert_eq!(functions_in(0, fr).first().map(|def| def.name), Some("SUM"));

        let found: Vec<&str> = search("somme si", fr).iter().map(|def| def.name).collect();
        assert!(found.contains(&"SUMIF") && found.contains(&"SUMIFS"), "{found:?}");
        assert!(search("   ", fr).is_empty());
        assert!(search("zzzzqqq", fr).is_empty());
    }

    #[test]
    fn labels_take_a_capital() {
        assert_eq!(capitalized("number1"), "Number1");
        assert_eq!(capitalized("été"), "Été");
        assert_eq!(capitalized(""), "");
    }
}
