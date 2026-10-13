//! From two states of a workbook to the script that leads from the first
//! to the second.
//!
//! The statements are written stage by stage — sheets, tables, contents,
//! formats, sizes, filters, names, panes, charts — and each stage is run
//! on the first workbook, through the script engine itself, before the
//! next one is written: a stage always describes what is really left to
//! do, whatever the statements before it brought along (a table that grew
//! over the row typed under it, a total row that came with its formula).
//! When the stages are through and the two workbooks still differ, there
//! is no script: the action is one the recorder cannot write.

use super::code::{Scope, Step, color, member, number, play, points, quote, rects_of, runs_of};
use super::compare::{Formats, Pair, content_at, differences, distance, same_chart, shown, side_by_side};
use opensheet_engine::chart::{Chart, ChartKind, ChartTitle, LegendPos};
use opensheet_engine::filter::{AutoFilter, Criteria, CustomOp, DynamicFilter, SortState};
use opensheet_engine::input::parse_input;
use opensheet_engine::ops::SortOn;
use opensheet_engine::refs::{MAX_COLS, MAX_ROWS, cell_name};
use opensheet_engine::style::{Border, BorderStyle, Borders, HAlign, Style, StyleId, Underline, VAlign};
use opensheet_engine::table::Table;
use opensheet_engine::workbook::DefinedName;
use opensheet_engine::{Color, Content, Locale, Rect, Sheet, Value, Workbook};
use opensheet_script::Outcome;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// What the window does after a script ran, before its result is looked
/// at: fit again, with its fonts, the columns the script asked to fit.
pub(crate) type Refit<'a> = &'a dyn Fn(&mut Workbook, &Outcome);

/// The most cells one action may write and still be recorded: beyond, the
/// script would be all data.
const MAX_CELLS: usize = 200_000;

/// Run some steps on the workbook being brought to its target.
pub(crate) fn run(work: &mut Workbook, scope: &Scope, steps: &[Step], refit: Refit) -> Result<(), String> {
    let front = scope.start.as_deref().and_then(|name| work.sheets.iter().position(|sheet| sheet.name == name)).unwrap_or(0);
    let outcome = play(work, front, steps)?;
    refit(work, &outcome);
    Ok(())
}

type Stage = fn(&Workbook, &Workbook, &Scope) -> Vec<Step>;

/// The stages after the sheets themselves, in the order they are run.
const STAGES: [Stage; 15] = [
    undone,
    new_tables,
    table_options,
    sorts,
    names,
    contents,
    table_sizes,
    line_formats,
    cell_formats,
    merges,
    sizes,
    filters,
    panes,
    new_charts,
    chart_options,
];

/// Write the steps that turn `work` into `target`, running them on
/// `work` as they are written. `Err` says what could not be reached;
/// `work` is then of no use.
pub(crate) fn reach(work: &mut Workbook, target: &Workbook, scope: &mut Scope, refit: Refit) -> Result<Vec<Step>, String> {
    let mut all = Vec::new();
    for round in 0..3 {
        if distance(work, target) == 0 {
            return Ok(all);
        }
        let (steps, start) = sheets(work, target, scope);
        if !steps.is_empty() {
            run(work, scope, &steps, refit)?;
            scope.start = start;
            all.extend(steps);
        }
        if work.sheets.len() != target.sheets.len() || work.sheets.iter().zip(&target.sheets).any(|(a, b)| a.name != b.name) {
            return Err("the sheets".to_string());
        }
        for (index, stage) in STAGES.iter().enumerate() {
            // A sort is not something to do twice.
            if round > 0 && index == 3 {
                continue;
            }
            let steps = stage(work, target, scope);
            if steps.is_empty() {
                continue;
            }
            run(work, scope, &steps, refit)?;
            all.extend(steps);
        }
    }
    let left = differences(work, target);
    if left.is_empty() { Ok(all) } else { Err(left.join("; ")) }
}

fn pairs<'a>(work: &'a Workbook, target: &'a Workbook) -> impl Iterator<Item = (&'a Sheet, &'a Sheet)> {
    work.sheets.iter().zip(&target.sheets)
}

// ------------------------------------------------------------------ sheets

/// Sheets renamed, deleted, added and moved, and what shows of each: the
/// colour of its tab, its gridlines and headings. Also gives the name
/// `selectedSheet` has once the steps are run.
fn sheets(work: &Workbook, target: &Workbook, scope: &Scope) -> (Vec<Step>, Option<String>) {
    let old: Vec<&str> = work.sheets.iter().map(|sheet| sheet.name.as_str()).collect();
    let new: Vec<&str> = target.sheets.iter().map(|sheet| sheet.name.as_str()).collect();
    let mut scope = scope.clone();
    let mut steps = Vec::new();
    let gone: Vec<&str> = old.iter().copied().filter(|name| !new.contains(name)).collect();
    let came: Vec<&str> = new.iter().copied().filter(|name| !old.contains(name)).collect();
    let mut order = old.clone();
    // The sheet each sheet of the target was, when it was there.
    let mut was: HashMap<&str, &Sheet> = work.sheets.iter().filter(|sheet| new.contains(&sheet.name.as_str())).map(|sheet| (sheet.name.as_str(), sheet)).collect();
    // As many names gone as come: the sheets were renamed.
    for (from, to) in gone.iter().zip(&came) {
        steps.push(Step::new(format!("Rename worksheet {from} to {to}"), format!("{}.setName({});", scope.sheet(from), quote(to))));
        if scope.start.as_deref() == Some(*from) {
            scope.start = Some(to.to_string());
        }
        if let Some(place) = order.iter_mut().find(|name| *name == from) {
            *place = to;
        }
        if let Some(sheet) = work.sheets.iter().find(|sheet| sheet.name == *from) {
            was.insert(to, sheet);
        }
    }
    for from in gone.iter().skip(came.len()) {
        steps.push(Step::new(format!("Delete worksheet {from}"), format!("{}.delete();", scope.sheet(from))));
        if scope.start.as_deref() == Some(*from) {
            scope.start = None;
        }
        order.retain(|name| name != from);
    }
    for to in came.iter().skip(gone.len()) {
        steps.push(Step::new("Add a new worksheet", format!("workbook.addWorksheet({});", quote(to))));
        order.push(to);
    }
    for (index, want) in new.iter().enumerate() {
        if order.get(index) != Some(want) {
            order.retain(|name| name != want);
            order.insert(index.min(order.len()), want);
            steps.push(Step::new(format!("Move worksheet {want}"), format!("{}.setPosition({index});", scope.sheet(want))));
        }
    }
    let blank = Sheet::new("");
    for sheet in &target.sheets {
        let before = was.get(sheet.name.as_str()).copied().unwrap_or(&blank);
        let expr = scope.sheet(&sheet.name);
        let on = scope.on(&sheet.name);
        if before.tab_color.map(Color::value) != sheet.tab_color.map(Color::value) {
            let text = sheet.tab_color.map_or(quote(""), color);
            steps.push(Step::new(format!("Set the tab color of {on}"), format!("{expr}.setTabColor({text});")));
        }
        if before.show_gridlines != sheet.show_gridlines {
            steps.push(Step::new(format!("Show gridlines on {on}: {}", sheet.show_gridlines), format!("{expr}.setShowGridlines({});", sheet.show_gridlines)));
        }
        if before.show_headers != sheet.show_headers {
            steps.push(Step::new(format!("Show headings on {on}: {}", sheet.show_headers), format!("{expr}.setShowHeadings({});", sheet.show_headers)));
        }
    }
    (steps, scope.start)
}

// ------------------------------------------------------------------ tables

/// The table of `tables` that is the table `table` of the other
/// workbook: the same number, or the same place.
fn twin<'a>(tables: &'a [Table], table: &Table) -> Option<&'a Table> {
    tables.iter().find(|other| other.id == table.id).or_else(|| tables.iter().find(|other| (other.range.r0, other.range.c0, other.range.c1) == (table.range.r0, table.range.c0, table.range.c1)))
}

fn table_expr(name: &str) -> String {
    format!("workbook.getTable({})", quote(name))
}

/// What must go before anything else is written: tables that are no
/// longer tables, a filter taken off, cells no longer merged.
fn undone(work: &Workbook, target: &Workbook, scope: &Scope) -> Vec<Step> {
    let mut steps = Vec::new();
    for (ws, ts) in pairs(work, target) {
        let on = scope.on(&ts.name);
        for table in &ws.tables {
            if twin(&ts.tables, table).is_none() {
                steps.push(Step::new(format!("Convert table {} to a range", table.name), format!("{}.convertToRange();", table_expr(&table.name))));
            }
        }
        if ws.auto_filter.is_some() && ts.auto_filter.is_none() {
            steps.push(Step::new(format!("Remove the AutoFilter of {on}"), format!("{}.getAutoFilter().remove();", scope.sheet(&ts.name))));
        }
        for merge in &ws.merges {
            if !ts.merges.contains(merge) {
                steps.push(Step::new(format!("Unmerge range {} on {on}", merge.name()), format!("{}.unmerge();", scope.range(&ts.name, *merge))));
            }
        }
    }
    steps
}

/// What a cell shows when it holds a constant, for the header of a table.
fn plain_text(sheet: &Sheet, row: u32, col: u32) -> Option<String> {
    match content_at(sheet, (row, col)) {
        Content::Value(Value::Text(text)) => Some(text.to_string()),
        _ => None,
    }
}

fn new_tables(work: &Workbook, target: &Workbook, scope: &Scope) -> Vec<Step> {
    let mut steps = Vec::new();
    for (ws, ts) in pairs(work, target) {
        for table in &ts.tables {
            if twin(&ws.tables, table).is_some() {
                continue;
            }
            let mut range = table.range;
            range.r1 -= (table.totals_row && range.r1 > range.r0) as u32;
            // The headers were there when the cells of the first row
            // already read as the columns are named; else the table was
            // made without, and its data moved down a row.
            let headed = table.header_row && table.columns.iter().enumerate().all(|(index, column)| plain_text(ws, range.r0, range.c0 + index as u32).as_deref() == Some(column.name.as_str()));
            if !headed && range.r1 > range.r0 {
                range.r1 -= 1;
            }
            steps.push(Step::new(
                format!("Add a new table at range {} on {}", range.name(), scope.on(&ts.name)),
                format!("workbook.addTable({}, {headed});", scope.range(&ts.name, range)),
            ));
        }
    }
    steps
}

/// The name, the style and the check boxes of each table, and its size.
fn table_options(work: &Workbook, target: &Workbook, _: &Scope) -> Vec<Step> {
    let mut steps = Vec::new();
    for (ws, ts) in pairs(work, target) {
        for table in &ts.tables {
            let Some(now) = twin(&ws.tables, table) else {
                continue;
            };
            let mut name = now.name.as_str();
            if now.name != table.name {
                steps.push(Step::new(format!("Rename table {} to {}", now.name, table.name), format!("{}.setName({});", table_expr(name), quote(&table.name))));
                name = &table.name;
            }
            let expr = table_expr(name);
            if now.style != table.style {
                let style = table.style.as_deref().unwrap_or_default();
                steps.push(Step::new(format!("Change the style of table {name}"), format!("{expr}.setPredefinedTableStyle({});", quote(style))));
            }
            let options: [(&str, &str, bool, bool); 7] = [
                ("header row", "setShowHeaders", now.header_row, table.header_row),
                ("total row", "setShowTotals", now.totals_row, table.totals_row),
                ("banded rows", "setShowBandedRows", now.banded_rows, table.banded_rows),
                ("banded columns", "setShowBandedColumns", now.banded_cols, table.banded_cols),
                ("first column", "setHighlightFirstColumn", now.first_col, table.first_col),
                ("last column", "setHighlightLastColumn", now.last_col, table.last_col),
                ("filter button", "setShowFilterButton", now.auto_filter.is_some(), table.auto_filter.is_some()),
            ];
            for (what, setter, was, is) in options {
                if was != is {
                    steps.push(Step::new(format!("Show the {what} of table {name}: {is}"), format!("{expr}.{setter}({is});")));
                }
            }
        }
    }
    steps
}

/// The tables that are not yet the size they must be. This comes after
/// the contents: a table grows by itself over what is typed under it.
fn table_sizes(work: &Workbook, target: &Workbook, _: &Scope) -> Vec<Step> {
    let mut steps = Vec::new();
    for (ws, ts) in pairs(work, target) {
        for table in &ts.tables {
            let Some(now) = twin(&ws.tables, table).filter(|now| now.name == table.name) else {
                continue;
            };
            if now.range != table.range && (now.header_row, now.totals_row) == (table.header_row, table.totals_row) {
                steps.push(Step::new(
                    format!("Resize table {} to range {}", table.name, table.range.name()),
                    format!("{}.resize({});", table_expr(&table.name), quote(&table.range.name())),
                ));
            }
        }
    }
    steps
}

// ------------------------------------------------------------------- sorts

/// The levels of a sort as `SortField` objects; `None` when a level is
/// one a script cannot ask for.
fn sort_fields(sort: &SortState) -> Option<String> {
    let fields: Option<Vec<String>> = sort
        .levels
        .iter()
        .map(|level| {
            if level.custom_list.is_some() {
                return None;
            }
            let on = match level.on {
                SortOn::Values => String::new(),
                SortOn::CellColor(Some(tint)) => format!(", sortOn: {}, color: {}", member("SortOn", "cellColor"), color(tint)),
                SortOn::FontColor(Some(tint)) => format!(", sortOn: {}, color: {}", member("SortOn", "fontColor"), color(tint)),
                SortOn::CellColor(None) | SortOn::FontColor(None) => return None,
            };
            Some(format!("{{ key: {}, ascending: {}{on} }}", level.key, !level.descending))
        })
        .collect();
    fields.filter(|fields| !fields.is_empty()).map(|fields| format!("[{}]", fields.join(", ")))
}

/// The sorts a filter remembers: a table or a filtered range sorted from
/// its buttons, or from the Sort dialog.
fn sorts(work: &Workbook, target: &Workbook, scope: &Scope) -> Vec<Step> {
    let mut steps = Vec::new();
    let changed = |now: Option<&AutoFilter>, then: Option<&AutoFilter>| -> Option<SortState> {
        let (now, then) = (now?, then?);
        (now.range == then.range && now.sort != then.sort).then(|| then.sort.clone()).flatten()
    };
    for (ws, ts) in pairs(work, target) {
        for table in &ts.tables {
            let Some(now) = twin(&ws.tables, table).filter(|now| now.name == table.name) else {
                continue;
            };
            if let Some(fields) = changed(now.auto_filter.as_ref(), table.auto_filter.as_ref()).and_then(|sort| Some((sort_fields(&sort)?, sort.case_sensitive))) {
                steps.push(Step::new(format!("Sort table {}", table.name), format!("{}.getSort().apply({}, {});", table_expr(&table.name), fields.0, fields.1)));
            }
        }
        if let Some(sort) = changed(ws.auto_filter.as_ref(), ts.auto_filter.as_ref()) {
            if let (Some(fields), Some(filter)) = (sort_fields(&sort), &ts.auto_filter) {
                steps.push(Step::new(
                    format!("Sort the range {} on {}", filter.range.name(), scope.on(&ts.name)),
                    format!("{}.getSort().apply({fields}, {}, true);", scope.range(&ts.name, filter.range), sort.case_sensitive),
                ));
            }
        }
    }
    steps
}

// ------------------------------------------------------------------- names

fn name_steps(steps: &mut Vec<Step>, holder: &str, now: &BTreeMap<String, DefinedName>, then: &BTreeMap<String, DefinedName>) {
    let en = Locale::en();
    for (key, name) in now {
        if !then.contains_key(key) {
            steps.push(Step::new(format!("Delete the name {}", name.name), format!("{holder}.getNamedItem({}).delete();", quote(&name.name))));
        }
    }
    for (key, name) in then {
        let formula = quote(&name.refers_to(en, 0, 0));
        match now.get(key) {
            None => {
                let comment = if name.comment.is_empty() { String::new() } else { format!(", {}", quote(&name.comment)) };
                steps.push(Step::new(format!("Define the name {}", name.name), format!("{holder}.addNamedItem({}, {formula}{comment});", quote(&name.name))));
            }
            Some(old) => {
                let item = format!("{holder}.getNamedItem({})", quote(&old.name));
                if old.expr != name.expr {
                    steps.push(Step::new(format!("Change what the name {} refers to", name.name), format!("{item}.setFormula({formula});")));
                }
                if old.comment != name.comment {
                    steps.push(Step::new(format!("Change the comment of the name {}", name.name), format!("{item}.setComment({});", quote(&name.comment))));
                }
            }
        }
    }
}

fn names(work: &Workbook, target: &Workbook, scope: &Scope) -> Vec<Step> {
    let mut steps = Vec::new();
    name_steps(&mut steps, "workbook", &work.names, &target.names);
    for (ws, ts) in pairs(work, target) {
        name_steps(&mut steps, &scope.sheet(&ts.name), &ws.names, &ts.names);
    }
    steps
}

// ---------------------------------------------------------------- contents

/// A text as it must be typed to be taken as that text: `'007` for the
/// text 007. A cell formatted as text takes anything as it is.
fn as_typed(text: &str, verbatim: bool) -> String {
    if verbatim {
        return text.to_string();
    }
    match parse_input(text, Locale::en()) {
        Ok(parsed) if parsed.num_fmt.is_none() && parsed.content == Content::Value(Value::text(text)) => text.to_string(),
        _ => format!("'{text}"),
    }
}

/// A value as `setValue` takes it.
fn literal(content: &Content, verbatim: bool) -> String {
    match content {
        Content::Value(Value::Number(n)) => number(*n),
        Content::Value(Value::Bool(b)) => b.to_string(),
        Content::Value(Value::Text(text)) => quote(&as_typed(text, verbatim)),
        Content::Value(Value::Error(error)) => quote(error.as_str()),
        Content::Formula(formula) => quote(&format!("={}", formula.to_text(Locale::en()))),
        _ => quote(""),
    }
}

/// What a cell holds as it is typed in English, for `setFormulas`.
fn typed(content: &Content, verbatim: bool) -> String {
    match content {
        Content::Value(Value::Number(n)) => number(*n),
        Content::Value(Value::Bool(b)) => b.to_string().to_uppercase(),
        Content::Value(Value::Text(text)) => as_typed(text, verbatim),
        Content::Value(Value::Error(error)) => error.as_str().to_string(),
        Content::Formula(formula) => format!("={}", formula.to_text(Locale::en())),
        _ => String::new(),
    }
}

/// The statement that writes the contents the target has in `rect`.
fn write_step(work: &Workbook, ws: &Sheet, ts: &Sheet, rect: Rect, scope: &Scope) -> Step {
    let at = |row: u32, col: u32| content_at(ts, (row, col));
    let verbatim = |row: u32, col: u32| work.styles.get(ws.style_id_at(row, col)).num_fmt.as_ref() == "@";
    let cells = || (rect.r0..=rect.r1).flat_map(|row| (rect.c0..=rect.c1).map(move |col| (row, col)));
    let range = scope.range(&ts.name, rect);
    let comment = format!("Set range {} on {}", rect.name(), scope.on(&ts.name));
    let first = at(rect.r0, rect.c0);
    let first_verbatim = verbatim(rect.r0, rect.c0);
    match first {
        // One value for every cell, as Ctrl+Enter enters it.
        Content::Value(_) if cells().all(|(row, col)| at(row, col) == first && verbatim(row, col) == first_verbatim) => {
            return Step::new(comment, format!("{range}.setValue({});", literal(first, first_verbatim)));
        }
        // One formula filled over the cells.
        Content::Formula(formula)
            if cells().all(|(row, col)| matches!(at(row, col), Content::Formula(other) if **other == formula.shifted((row - rect.r0) as i64, (col - rect.c0) as i64))) =>
        {
            return Step::new(comment, format!("{range}.setFormula({});", literal(first, false)));
        }
        _ => {}
    }
    let formulas = cells().any(|(row, col)| matches!(at(row, col), Content::Formula(_)));
    let rows: Vec<String> = (rect.r0..=rect.r1)
        .map(|row| {
            let items: Vec<String> = (rect.c0..=rect.c1)
                .map(|col| if formulas { quote(&typed(at(row, col), verbatim(row, col))) } else { literal(at(row, col), verbatim(row, col)) })
                .collect();
            format!("[{}]", items.join(", "))
        })
        .collect();
    Step::new(comment, format!("{range}.{}([{}]);", if formulas { "setFormulas" } else { "setValues" }, rows.join(", ")))
}

fn contents(work: &Workbook, target: &Workbook, scope: &Scope) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut count = 0;
    for (ws, ts) in pairs(work, target) {
        let (mut cleared, mut written) = (BTreeSet::new(), BTreeSet::new());
        for Pair { pos, contents, .. } in side_by_side(ws, ts) {
            if contents.0 != contents.1 {
                if matches!(contents.1, Content::Empty) { cleared.insert(pos) } else { written.insert(pos) };
            }
        }
        count += cleared.len() + written.len();
        if count > MAX_CELLS {
            return Vec::new();
        }
        for rect in rects_of(&cleared) {
            steps.push(Step::new(
                format!("Clear contents of range {} on {}", rect.name(), scope.on(&ts.name)),
                format!("{}.clear({});", scope.range(&ts.name, rect), member("ClearApplyTo", "contents")),
            ));
        }
        for rect in rects_of(&written) {
            steps.push(write_step(work, ws, ts, rect, scope));
        }
    }
    steps
}

// ----------------------------------------------------------------- formats

/// One property of a format to change: what it is in the words of a
/// comment, and what is called on the range.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Change {
    order: u8,
    what: String,
    call: String,
}

fn h_align(align: HAlign) -> &'static str {
    match align {
        HAlign::General => "general",
        HAlign::Left => "left",
        HAlign::Center => "center",
        HAlign::Right => "right",
        HAlign::Fill => "fill",
        HAlign::Justify => "justify",
        HAlign::CenterAcross => "centerAcrossSelection",
    }
}

/// The properties to change to go from one format to another, borders
/// apart. Both formats are given as they show (see `shown`).
fn changes(now: &Style, then: &Style) -> Vec<Change> {
    let mut out = Vec::new();
    let mut add = |order: u8, differs: bool, what: String, call: String| {
        if differs {
            out.push(Change { order, what, call });
        }
    };
    let hex = |tint: Option<Color>| format!("{:06X}", tint.map_or(0, Color::value));
    let (a, b) = (&now.font, &then.font);
    add(0, now.num_fmt != then.num_fmt, "Set number format".to_string(), format!(".setNumberFormat({});", quote(&then.num_fmt)));
    match then.fill {
        Some(tint) => add(1, now.fill != then.fill, format!("Set fill color to {}", hex(Some(tint))), format!(".getFormat().getFill().setColor({});", color(tint))),
        None => add(1, now.fill.is_some(), "Clear fill color".to_string(), ".getFormat().getFill().clear();".to_string()),
    }
    add(2, a.name != b.name, format!("Set font name to {}", quote(&b.name)), format!(".getFormat().getFont().setName({});", quote(&b.name)));
    add(3, a.size != b.size, format!("Set font size to {}", number(b.size_pt() as f64)), format!(".getFormat().getFont().setSize({});", number(b.size_pt() as f64)));
    add(4, a.bold != b.bold, format!("Set font bold to {}", b.bold), format!(".getFormat().getFont().setBold({});", b.bold));
    add(5, a.italic != b.italic, format!("Set font italic to {}", b.italic), format!(".getFormat().getFont().setItalic({});", b.italic));
    let underline = match b.underline {
        Underline::None => "none",
        Underline::Single => "single",
        Underline::Double => "double",
    };
    add(
        6,
        a.underline != b.underline,
        format!("Set font underline to {}", member("RangeUnderlineStyle", underline)),
        format!(".getFormat().getFont().setUnderline({});", member("RangeUnderlineStyle", underline)),
    );
    add(7, a.strike != b.strike, format!("Set font strikethrough to {}", b.strike), format!(".getFormat().getFont().setStrikethrough({});", b.strike));
    add(8, a.color != b.color, format!("Set font color to {}", hex(b.color)), format!(".getFormat().getFont().setColor({});", color(b.color.unwrap_or(Color::BLACK))));
    let across = member("HorizontalAlignment", h_align(then.h_align));
    add(9, now.h_align != then.h_align, format!("Set horizontal alignment to {across}"), format!(".getFormat().setHorizontalAlignment({across});"));
    let upright = member(
        "VerticalAlignment",
        match then.v_align {
            VAlign::Top => "top",
            VAlign::Center => "center",
            VAlign::Bottom => "bottom",
        },
    );
    add(10, now.v_align != then.v_align, format!("Set vertical alignment to {upright}"), format!(".getFormat().setVerticalAlignment({upright});"));
    add(11, now.wrap != then.wrap, format!("Set wrap text to {}", then.wrap), format!(".getFormat().setWrapText({});", then.wrap));
    add(12, now.shrink_to_fit != then.shrink_to_fit, format!("Set shrink to fit to {}", then.shrink_to_fit), format!(".getFormat().setShrinkToFit({});", then.shrink_to_fit));
    add(13, now.indent != then.indent, format!("Set indent level to {}", then.indent), format!(".getFormat().setIndentLevel({});", then.indent));
    add(14, now.rotation != then.rotation, format!("Set text orientation to {}", then.rotation), format!(".getFormat().setTextOrientation({});", then.rotation));
    add(15, now.locked != then.locked, format!("Set locked to {}", then.locked), format!(".getFormat().getProtection().setLocked({});", then.locked));
    add(16, now.hidden != then.hidden, format!("Set formula hidden to {}", then.hidden), format!(".getFormat().getProtection().setFormulaHidden({});", then.hidden));
    out
}

/// The formats kept on whole columns and rows.
fn line_formats(work: &Workbook, target: &Workbook, scope: &Scope) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut formats = Formats::new(work, target);
    for (ws, ts) in pairs(work, target) {
        let on = scope.on(&ts.name);
        let mut lines = |now: &BTreeMap<u32, StyleId>, then: &BTreeMap<u32, StyleId>, columns: bool| {
            // Runs of lines that go from one same format to one same other.
            let mut groups: BTreeMap<(StyleId, StyleId), Vec<u32>> = BTreeMap::new();
            let all: BTreeSet<u32> = now.keys().chain(then.keys()).copied().collect();
            for line in all {
                let pair = (now.get(&line).copied().unwrap_or_default(), then.get(&line).copied().unwrap_or_default());
                if !formats.same(pair.0, pair.1) {
                    groups.entry(pair).or_default().push(line);
                }
            }
            for ((a, b), lines) in groups {
                let (a, b) = (shown(work.styles.get(a)), shown(target.styles.get(b)));
                for (first, last) in runs_of(lines) {
                    let rect = if columns { Rect::new(0, first, MAX_ROWS - 1, last) } else { Rect::new(first, 0, last, MAX_COLS - 1) };
                    let range = scope.range(&ts.name, rect);
                    for change in changes(&a, &b) {
                        steps.push(Step::new(format!("{} for range {} on {on}", change.what, rect.name()), format!("{range}{}", change.call)));
                    }
                }
            }
        };
        lines(&ws.col_styles, &ts.col_styles, true);
        lines(&ws.row_styles, &ts.row_styles, false);
    }
    steps
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Side {
    Top,
    Bottom,
    Left,
    Right,
}

fn side(borders: &Borders, side: Side) -> Border {
    match side {
        Side::Top => borders.top,
        Side::Bottom => borders.bottom,
        Side::Left => borders.left,
        Side::Right => borders.right,
    }
}

/// The statements that draw one border of a range as `line`.
fn border_lines(range: &str, index: &str, line: Border, recolour: bool) -> Vec<String> {
    let border = format!("{range}.getFormat().getRangeBorder({})", member("BorderIndex", index));
    let style = |name: &str| format!("{border}.setStyle({});", member("BorderLineStyle", name));
    let weight = |name: &str| format!("{border}.setWeight({});", member("BorderWeight", name));
    let mut lines = match line.style {
        BorderStyle::None => return vec![style("none")],
        BorderStyle::Hair => vec![style("continuous"), weight("hairline")],
        BorderStyle::Thin => vec![style("continuous"), weight("thin")],
        BorderStyle::Medium => vec![style("continuous"), weight("medium")],
        BorderStyle::Thick => vec![style("continuous"), weight("thick")],
        BorderStyle::Dashed => vec![style("dash")],
        BorderStyle::Dotted => vec![style("dot")],
        BorderStyle::Double => vec![style("double")],
    };
    if recolour {
        lines.push(format!("{border}.setColor({});", color(line.color.unwrap_or(Color::BLACK))));
    }
    lines
}

/// The borders of the cells of one rectangle, all of which have a border
/// to change.
fn border_steps(steps: &mut Vec<Step>, name: &str, rect: Rect, now: &dyn Fn(u32, u32) -> Borders, then: &dyn Fn(u32, u32) -> Borders, scope: &Scope) {
    let on = scope.on(name);
    let cells = |rect: Rect| (rect.r0..=rect.r1).flat_map(move |row| (rect.c0..=rect.c1).map(move |col| (row, col)));
    // A border must be coloured when it is not black, or when the line
    // it replaces was of another colour.
    let recolour = |line: Border, part: &[(Rect, Side)]| {
        line.color.is_some_and(|tint| tint.value() != 0)
            || part.iter().any(|(rect, which)| cells(*rect).any(|(row, col)| {
                let old = side(&now(row, col), *which);
                !old.is_none() && old.color != line.color
            }))
    };
    let differs = |part: &[(Rect, Side)]| part.iter().any(|(rect, which)| cells(*rect).any(|(row, col)| side(&now(row, col), *which) != side(&then(row, col), *which)));
    let uniform = |part: &[(Rect, Side)]| -> Option<Border> {
        let (first, which) = part.first()?;
        let line = side(&then(first.r0, first.c0), *which);
        part.iter().all(|(rect, which)| cells(*rect).all(|(row, col)| side(&then(row, col), *which) == line)).then_some(line)
    };
    // The six borders a range has: when each is one same line, they are
    // written as the Borders menu and Format Cells set them.
    let mut frame: Vec<(&str, Vec<(Rect, Side)>)> = vec![
        ("edgeTop", vec![(Rect::new(rect.r0, rect.c0, rect.r0, rect.c1), Side::Top)]),
        ("edgeBottom", vec![(Rect::new(rect.r1, rect.c0, rect.r1, rect.c1), Side::Bottom)]),
        ("edgeLeft", vec![(Rect::new(rect.r0, rect.c0, rect.r1, rect.c0), Side::Left)]),
        ("edgeRight", vec![(Rect::new(rect.r0, rect.c1, rect.r1, rect.c1), Side::Right)]),
    ];
    if rect.cols() > 1 {
        frame.push(("insideVertical", vec![(Rect::new(rect.r0, rect.c0, rect.r1, rect.c1 - 1), Side::Right), (Rect::new(rect.r0, rect.c0 + 1, rect.r1, rect.c1), Side::Left)]));
    }
    if rect.rows() > 1 {
        frame.push(("insideHorizontal", vec![(Rect::new(rect.r0, rect.c0, rect.r1 - 1, rect.c1), Side::Bottom), (Rect::new(rect.r0 + 1, rect.c0, rect.r1, rect.c1), Side::Top)]));
    }
    if frame.iter().all(|(_, part)| uniform(part).is_some()) {
        let range = scope.range(name, rect);
        let mut lines = Vec::new();
        for (index, part) in &frame {
            if let Some(line) = uniform(part).filter(|_| differs(part)) {
                lines.extend(border_lines(&range, index, line, recolour(line, part)));
            }
        }
        if !lines.is_empty() {
            steps.push(Step {
                comment: format!("Set border for range {} on {on}", rect.name()),
                lines,
            });
        }
        return;
    }
    // Else side by side: each run of cells whose top (bottom, left, right)
    // becomes one same line.
    for (which, index) in [(Side::Top, "edgeTop"), (Side::Bottom, "edgeBottom"), (Side::Left, "edgeLeft"), (Side::Right, "edgeRight")] {
        let mut groups: Vec<(Border, BTreeSet<(u32, u32)>)> = Vec::new();
        for (row, col) in cells(rect) {
            let line = side(&then(row, col), which);
            if side(&now(row, col), which) != line {
                match groups.iter_mut().find(|(other, _)| *other == line) {
                    Some((_, cells)) => {
                        cells.insert((row, col));
                    }
                    None => groups.push((line, BTreeSet::from([(row, col)]))),
                }
            }
        }
        for (line, group) in groups {
            for block in rects_of(&group) {
                // An edge is one row or one column of cells.
                let strips: Vec<Rect> = match which {
                    Side::Top | Side::Bottom => (block.r0..=block.r1).map(|row| Rect::new(row, block.c0, row, block.c1)).collect(),
                    Side::Left | Side::Right => (block.c0..=block.c1).map(|col| Rect::new(block.r0, col, block.r1, col)).collect(),
                };
                for strip in strips {
                    let part = [(strip, which)];
                    steps.push(Step {
                        comment: format!("Set border for range {} on {on}", strip.name()),
                        lines: border_lines(&scope.range(name, strip), index, line, recolour(line, &part)),
                    });
                }
            }
        }
    }
}

fn cell_formats(work: &Workbook, target: &Workbook, scope: &Scope) -> Vec<Step> {
    let mut steps = Vec::new();
    let mut formats = Formats::new(work, target);
    // What each pair of formats met takes to go from one to the other.
    let mut known: HashMap<(StyleId, StyleId), (Vec<Change>, bool)> = HashMap::new();
    for (ws, ts) in pairs(work, target) {
        let on = scope.on(&ts.name);
        let mut groups: BTreeMap<Change, BTreeSet<(u32, u32)>> = BTreeMap::new();
        let mut bordered: BTreeSet<(u32, u32)> = BTreeSet::new();
        for Pair { pos, styles: pair, .. } in side_by_side(ws, ts) {
            if formats.same(pair.0, pair.1) {
                continue;
            }
            let (list, borders) = known.entry(pair).or_insert_with(|| {
                let (a, b) = (shown(work.styles.get(pair.0)), shown(target.styles.get(pair.1)));
                (changes(&a, &b), a.borders != b.borders)
            });
            for change in list.iter() {
                groups.entry(change.clone()).or_default().insert(pos);
            }
            if *borders {
                bordered.insert(pos);
            }
        }
        for (change, cells) in groups {
            for rect in rects_of(&cells) {
                steps.push(Step::new(format!("{} for range {} on {on}", change.what, rect.name()), format!("{}{}", scope.range(&ts.name, rect), change.call)));
            }
        }
        let now = |row: u32, col: u32| shown(work.styles.get(ws.style_id_at(row, col))).borders;
        let then = |row: u32, col: u32| shown(target.styles.get(ts.style_id_at(row, col))).borders;
        for rect in rects_of(&bordered) {
            border_steps(&mut steps, &ts.name, rect, &now, &then, scope);
        }
    }
    steps
}

fn merges(work: &Workbook, target: &Workbook, scope: &Scope) -> Vec<Step> {
    let mut steps = Vec::new();
    for (ws, ts) in pairs(work, target) {
        for merge in &ts.merges {
            if !ws.merges.contains(merge) {
                steps.push(Step::new(format!("Merge range {} on {}", merge.name(), scope.on(&ts.name)), format!("{}.merge(false);", scope.range(&ts.name, *merge))));
            }
        }
    }
    steps
}

// ------------------------------------------------------------------- sizes

fn sizes(work: &Workbook, target: &Workbook, scope: &Scope) -> Vec<Step> {
    let mut steps = Vec::new();
    for (ws, ts) in pairs(work, target) {
        let on = scope.on(&ts.name);
        // Lines whose size changed, by the size they take.
        let resized = |now: &BTreeMap<u32, f32>, then: &BTreeMap<u32, f32>, standard: f32| {
            let mut groups: BTreeMap<String, Vec<u32>> = BTreeMap::new();
            let all: BTreeSet<u32> = now.keys().chain(then.keys()).copied().collect();
            for line in all {
                let (a, b) = (now.get(&line).copied().unwrap_or(standard), then.get(&line).copied().unwrap_or(standard));
                if (a - b).abs() > 0.02 {
                    groups.entry(points(b)).or_default().push(line);
                }
            }
            groups
        };
        for (size, columns) in resized(&ws.col_widths, &ts.col_widths, ts.default_col_width) {
            for (first, last) in runs_of(columns) {
                let rect = Rect::new(0, first, MAX_ROWS - 1, last);
                steps.push(Step::new(
                    format!("Set width of column(s) at range {} on {on} to {size}", rect.name()),
                    format!("{}.getFormat().setColumnWidth({size});", scope.range(&ts.name, rect)),
                ));
            }
        }
        for (size, rows) in resized(&ws.row_heights, &ts.row_heights, ts.default_row_height) {
            for (first, last) in runs_of(rows) {
                let rect = Rect::new(first, 0, last, MAX_COLS - 1);
                steps.push(Step::new(
                    format!("Set height of row(s) at range {} on {on} to {size}", rect.name()),
                    format!("{}.getFormat().setRowHeight({size});", scope.range(&ts.name, rect)),
                ));
            }
        }
        // Lines hidden and shown again.
        let mut hidden = |now: &BTreeSet<u32>, then: &BTreeSet<u32>, columns: bool| {
            for (hide, lines) in [(true, then.difference(now)), (false, now.difference(then))] {
                for (first, last) in runs_of(lines.copied()) {
                    let rect = if columns { Rect::new(0, first, MAX_ROWS - 1, last) } else { Rect::new(first, 0, last, MAX_COLS - 1) };
                    let (what, setter) = if columns { ("column(s)", "setColumnHidden") } else { ("row(s)", "setRowHidden") };
                    steps.push(Step::new(
                        format!("{} {what} at range {} on {on}", if hide { "Hide" } else { "Unhide" }, rect.name()),
                        format!("{}.{setter}({hide});", scope.range(&ts.name, rect)),
                    ));
                }
            }
        };
        hidden(&ws.hidden_cols, &ts.hidden_cols, true);
        hidden(&ws.hidden_rows, &ts.hidden_rows, false);
    }
    steps
}

// ----------------------------------------------------------------- filters

/// A criterion as a `FilterCriteria` object; `None` for one a script
/// cannot ask for.
fn criteria_text(criteria: &Criteria) -> Option<String> {
    let on = |name: &str| format!("filterOn: {}", member("FilterOn", name));
    Some(match criteria {
        Criteria::Values(filter) => {
            if !filter.dates.is_empty() {
                return None;
            }
            let mut values: Vec<String> = filter.values.iter().map(|value| quote(value)).collect();
            if filter.blanks {
                values.push(quote(""));
            }
            format!("{{ {}, values: [{}] }}", on("values"), values.join(", "))
        }
        Criteria::Custom(custom) => {
            let condition = |condition: &opensheet_engine::filter::Condition| {
                let op = match condition.op {
                    CustomOp::Equal => "=",
                    CustomOp::NotEqual => "<>",
                    CustomOp::Greater => ">",
                    CustomOp::GreaterOrEqual => ">=",
                    CustomOp::Less => "<",
                    CustomOp::LessOrEqual => "<=",
                };
                quote(&format!("{op}{}", condition.value))
            };
            match &custom.second {
                Some(second) => format!(
                    "{{ {}, criterion1: {}, criterion2: {}, operator: {} }}",
                    on("custom"),
                    condition(&custom.first),
                    condition(second),
                    member("FilterOperator", if custom.and { "and" } else { "or" })
                ),
                None => format!("{{ {}, criterion1: {} }}", on("custom"), condition(&custom.first)),
            }
        }
        Criteria::Top10(top) => {
            let kind = match (top.top, top.percent) {
                (true, false) => "topItems",
                (false, false) => "bottomItems",
                (true, true) => "topPercent",
                (false, true) => "bottomPercent",
            };
            format!("{{ {}, criterion1: {} }}", on(kind), quote(&number(top.value)))
        }
        Criteria::Dynamic(dynamic) => {
            const MONTHS: [&str; 12] = ["January", "Februray", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
            let name = match dynamic {
                DynamicFilter::AboveAverage => "aboveAverage".to_string(),
                DynamicFilter::BelowAverage => "belowAverage".to_string(),
                DynamicFilter::Today => "today".to_string(),
                DynamicFilter::Yesterday => "yesterday".to_string(),
                DynamicFilter::Tomorrow => "tomorrow".to_string(),
                DynamicFilter::ThisWeek => "thisWeek".to_string(),
                DynamicFilter::LastWeek => "lastWeek".to_string(),
                DynamicFilter::NextWeek => "nextWeek".to_string(),
                DynamicFilter::ThisMonth => "thisMonth".to_string(),
                DynamicFilter::LastMonth => "lastMonth".to_string(),
                DynamicFilter::NextMonth => "nextMonth".to_string(),
                DynamicFilter::ThisQuarter => "thisQuarter".to_string(),
                DynamicFilter::LastQuarter => "lastQuarter".to_string(),
                DynamicFilter::NextQuarter => "nextQuarter".to_string(),
                DynamicFilter::ThisYear => "thisYear".to_string(),
                DynamicFilter::LastYear => "lastYear".to_string(),
                DynamicFilter::NextYear => "nextYear".to_string(),
                DynamicFilter::YearToDate => "yearToDate".to_string(),
                DynamicFilter::Quarter(quarter) => format!("allDatesInPeriodQuarter{quarter}"),
                DynamicFilter::Month(month) => format!("allDatesInPeriod{}", MONTHS.get(*month as usize - 1)?),
            };
            format!("{{ {}, dynamicCriteria: {} }}", on("dynamic"), member("DynamicFilterCriteria", &name))
        }
        Criteria::Color(filter) => format!("{{ {}, color: {} }}", on(if filter.font { "fontColor" } else { "cellColor" }), color(filter.color?)),
    })
}

/// The words of a comment for a criterion.
fn criteria_words(criteria: &Criteria) -> &'static str {
    match criteria {
        Criteria::Values(_) => "values filter",
        Criteria::Custom(_) => "custom filter",
        Criteria::Top10(_) => "top or bottom filter",
        Criteria::Dynamic(_) => "dynamic filter",
        Criteria::Color(_) => "color filter",
    }
}

fn filters(work: &Workbook, target: &Workbook, scope: &Scope) -> Vec<Step> {
    let mut steps = Vec::new();
    let none = BTreeMap::new();
    for (ws, ts) in pairs(work, target) {
        let on = scope.on(&ts.name);
        // The filter of the sheet.
        if let Some(filter) = &ts.auto_filter {
            let apply = format!("{}.getAutoFilter().apply({}", scope.sheet(&ts.name), quote(&filter.range.name()));
            let placed = ws.auto_filter.as_ref().filter(|now| now.range == filter.range);
            if placed.is_none() {
                steps.push(Step::new(format!("Apply the AutoFilter on range {} of {on}", filter.range.name()), format!("{apply});")));
            }
            let now = placed.map_or(&none, |now| &now.columns);
            for column in now.keys().chain(filter.columns.keys()).copied().collect::<BTreeSet<u32>>() {
                match filter.columns.get(&column) {
                    Some(criteria) if now.get(&column) != Some(criteria) => {
                        if let Some(text) = criteria_text(criteria) {
                            steps.push(Step::new(format!("Apply {} on column {} of the AutoFilter of {on}", criteria_words(criteria), column + 1), format!("{apply}, {column}, {text});")));
                        }
                    }
                    None => steps.push(Step::new(
                        format!("Clear the filter of column {} of the AutoFilter of {on}", column + 1),
                        format!("{}.getAutoFilter().clearColumnCriteria({column});", scope.sheet(&ts.name)),
                    )),
                    _ => {}
                }
            }
        }
        // The filters of the tables, column by column.
        for table in &ts.tables {
            let Some(now) = twin(&ws.tables, table).filter(|now| now.name == table.name) else {
                continue;
            };
            let (Some(old), Some(new)) = (&now.auto_filter, &table.auto_filter) else {
                continue;
            };
            for column in old.columns.keys().chain(new.columns.keys()).copied().collect::<BTreeSet<u32>>() {
                let Some(header) = table.columns.get(column as usize) else {
                    continue;
                };
                let filter = format!("{}.getColumnByName({}).getFilter()", table_expr(&table.name), quote(&header.name));
                match new.columns.get(&column) {
                    Some(criteria) if old.columns.get(&column) != Some(criteria) => {
                        if let Some(text) = criteria_text(criteria) {
                            steps.push(Step::new(format!("Apply {} on column {} of table {}", criteria_words(criteria), header.name, table.name), format!("{filter}.apply({text});")));
                        }
                    }
                    None => steps.push(Step::new(format!("Clear the filter of column {} of table {}", header.name, table.name), format!("{filter}.clear();"))),
                    _ => {}
                }
            }
        }
    }
    steps
}

// ------------------------------------------------------------------- panes

fn panes(work: &Workbook, target: &Workbook, scope: &Scope) -> Vec<Step> {
    let mut steps = Vec::new();
    for (ws, ts) in pairs(work, target) {
        if (ws.frozen_rows, ws.frozen_cols) == (ts.frozen_rows, ts.frozen_cols) {
            continue;
        }
        let on = scope.on(&ts.name);
        let panes = format!("{}.getFreezePanes()", scope.sheet(&ts.name));
        steps.push(match (ts.frozen_rows, ts.frozen_cols) {
            (0, 0) => Step::new(format!("Unfreeze the panes of {on}"), format!("{panes}.unfreeze();")),
            (rows, 0) => Step::new(format!("Freeze the first {rows} row(s) of {on}"), format!("{panes}.freezeRows({rows});")),
            (0, cols) => Step::new(format!("Freeze the first {cols} column(s) of {on}"), format!("{panes}.freezeColumns({cols});")),
            (rows, cols) => {
                let range = format!("A1:{}", cell_name(rows - 1, cols - 1));
                Step::new(format!("Freeze the panes of {on} at range {range}"), format!("{panes}.freezeAt({});", quote(&range)))
            }
        });
    }
    steps
}

// ------------------------------------------------------------------ charts

const CHART_TYPES: &[(&str, ChartKind)] = &[
    ("columnClustered", ChartKind::Column),
    ("columnStacked", ChartKind::StackedColumn),
    ("columnStacked100", ChartKind::PercentStackedColumn),
    ("_3DColumnClustered", ChartKind::Column3D),
    ("_3DColumnStacked", ChartKind::StackedColumn3D),
    ("_3DColumnStacked100", ChartKind::PercentStackedColumn3D),
    ("line", ChartKind::Line),
    ("lineStacked", ChartKind::StackedLine),
    ("lineStacked100", ChartKind::PercentStackedLine),
    ("lineMarkers", ChartKind::LineMarkers),
    ("lineMarkersStacked", ChartKind::StackedLineMarkers),
    ("lineMarkersStacked100", ChartKind::PercentStackedLineMarkers),
    ("_3DLine", ChartKind::Line3D),
    ("pie", ChartKind::Pie),
    ("_3DPie", ChartKind::Pie3D),
    ("doughnut", ChartKind::Doughnut),
    ("barClustered", ChartKind::Bar),
    ("barStacked", ChartKind::StackedBar),
    ("barStacked100", ChartKind::PercentStackedBar),
    ("_3DBarClustered", ChartKind::Bar3D),
    ("_3DBarStacked", ChartKind::StackedBar3D),
    ("_3DBarStacked100", ChartKind::PercentStackedBar3D),
    ("area", ChartKind::Area),
    ("areaStacked", ChartKind::StackedArea),
    ("areaStacked100", ChartKind::PercentStackedArea),
    ("_3DArea", ChartKind::Area3D),
    ("_3DAreaStacked", ChartKind::StackedArea3D),
    ("_3DAreaStacked100", ChartKind::PercentStackedArea3D),
    ("xyscatter", ChartKind::Scatter),
    ("xyscatterSmooth", ChartKind::ScatterSmoothMarkers),
    ("xyscatterSmoothNoMarkers", ChartKind::ScatterSmooth),
    ("xyscatterLines", ChartKind::ScatterLinesMarkers),
    ("xyscatterLinesNoMarkers", ChartKind::ScatterLines),
    ("bubble", ChartKind::Bubble),
    ("radar", ChartKind::Radar),
    ("radarMarkers", ChartKind::RadarMarkers),
    ("radarFilled", ChartKind::RadarFilled),
];

fn chart_type(kind: ChartKind) -> Option<String> {
    CHART_TYPES.iter().find(|(_, other)| *other == kind).map(|(name, _)| member("ChartType", name))
}

/// The cells a chart draws, when they are one block of its own sheet:
/// what `addChart` is given.
fn chart_source(sheet: &Sheet, chart: &Chart) -> Option<Rect> {
    let mut block: Option<Rect> = None;
    let mut alone = true;
    chart.clone().map_refs(&mut |reference| {
        if reference.sheet.as_deref().is_some_and(|name| !name.eq_ignore_ascii_case(&sheet.name)) {
            alone = false;
        }
        let rect = reference.rect();
        block = Some(block.map_or(rect, |block| block.union(&rect)));
        Some(reference.clone())
    });
    block.filter(|_| alone)
}

fn new_charts(work: &Workbook, target: &Workbook, scope: &Scope) -> Vec<Step> {
    let mut steps = Vec::new();
    for (index, (ws, ts)) in pairs(work, target).enumerate() {
        let on = scope.on(&ts.name);
        for chart in &ws.charts {
            if !ts.charts.iter().any(|other| other.id == chart.id) {
                steps.push(Step::new(format!("Delete chart {} on {on}", chart.name), format!("{}.getChart({}).delete();", scope.sheet(&ts.name), quote(&chart.name))));
            }
        }
        for chart in &ts.charts {
            if ws.charts.iter().any(|other| other.id == chart.id) {
                continue;
            }
            let (Some(kind), Some(source)) = (chart_type(chart.kind), chart_source(ts, chart)) else {
                continue;
            };
            // The series run the way the chart made of the same cells has
            // them; said only when that is not the way a new chart takes.
            let by = [(None, ""), (Some(true), "rows"), (Some(false), "columns")]
                .into_iter()
                .find(|(by_rows, _)| target.chart_from_block(index, source, chart.kind, *by_rows).series == chart.series)
                .map_or(String::new(), |(_, name)| if name.is_empty() { String::new() } else { format!(", {}", member("ChartSeriesBy", name)) });
            steps.push(Step::new(
                format!("Insert chart on sheet {on}"),
                format!("{}.addChart({kind}, {}{by});", scope.sheet(&ts.name), scope.range(&ts.name, source)),
            ));
        }
    }
    steps
}

/// The chart of `charts` that is `chart`: the same number, or, for a
/// chart a script just made, the same place in the list.
fn chart_twin<'a>(charts: &'a [Chart], all: &[Chart], chart: &Chart) -> Option<&'a Chart> {
    charts.iter().find(|other| other.id == chart.id).or_else(|| {
        let place = all.iter().position(|other| other.id == chart.id)?;
        charts.get(place).filter(|other| !all.iter().any(|known| known.id == other.id))
    })
}

fn chart_options(work: &Workbook, target: &Workbook, scope: &Scope) -> Vec<Step> {
    let mut steps = Vec::new();
    for (ws, ts) in pairs(work, target) {
        let on = scope.on(&ts.name);
        for chart in &ts.charts {
            let Some(now) = chart_twin(&ws.charts, &ts.charts, chart) else {
                continue;
            };
            if same_chart(ws, now, ts, chart) {
                continue;
            }
            let mut name = now.name.as_str();
            let sheet = scope.sheet(&ts.name);
            if now.name != chart.name {
                steps.push(Step::new(format!("Rename chart {} to {}", now.name, chart.name), format!("{sheet}.getChart({}).setName({});", quote(name), quote(&chart.name))));
                name = &chart.name;
            }
            let expr = format!("{sheet}.getChart({})", quote(name));
            if now.kind != chart.kind {
                if let Some(kind) = chart_type(chart.kind) {
                    steps.push(Step::new(format!("Change the type of chart {name} on {on}"), format!("{expr}.setChartType({kind});")));
                }
            }
            if now.title != chart.title {
                match &chart.title {
                    ChartTitle::Text(text) => steps.push(Step::new(format!("Set the title of chart {name} on {on}"), format!("{expr}.getTitle().setText({});", quote(text)))),
                    ChartTitle::None => steps.push(Step::new(format!("Hide the title of chart {name} on {on}"), format!("{expr}.getTitle().setVisible(false);"))),
                    ChartTitle::Auto => steps.push(Step::new(format!("Show the title of chart {name} on {on}"), format!("{expr}.getTitle().setVisible(true);"))),
                }
            }
            if now.title_overlay != chart.title_overlay {
                steps.push(Step::new(format!("Set the title of chart {name} over the plot: {}", chart.title_overlay), format!("{expr}.getTitle().setOverlay({});", chart.title_overlay)));
            }
            if now.legend != chart.legend {
                let place = match chart.legend {
                    LegendPos::None => None,
                    LegendPos::Bottom => Some("bottom"),
                    LegendPos::Right => Some("right"),
                    LegendPos::Top => Some("top"),
                    LegendPos::Left => Some("left"),
                    LegendPos::TopRight => Some("corner"),
                };
                match place {
                    None => steps.push(Step::new(format!("Hide the legend of chart {name} on {on}"), format!("{expr}.getLegend().setVisible(false);"))),
                    Some(place) => steps.push(Step::new(
                        format!("Set the legend of chart {name} on {on}"),
                        format!("{expr}.getLegend().setPosition({});", member("ChartLegendPosition", place)),
                    )),
                }
            }
            if now.legend_overlay != chart.legend_overlay {
                steps.push(Step::new(format!("Set the legend of chart {name} over the plot: {}", chart.legend_overlay), format!("{expr}.getLegend().setOverlay({});", chart.legend_overlay)));
            }
            // Where it is and how large, in points from the corner of
            // the sheet.
            let corner = |sheet: &Sheet, chart: &Chart| {
                let x: f64 = (0..chart.anchor.1).map(|col| sheet.col_width(col) as f64).sum();
                let y: f64 = (0..chart.anchor.0).map(|row| sheet.row_height(row) as f64).sum();
                ((x + chart.offset.0 as f64) as f32, (y + chart.offset.1 as f64) as f32)
            };
            let (was, is) = (corner(ws, now), corner(ts, chart));
            let mut lines = Vec::new();
            if (was.0 - is.0).abs() > 0.25 {
                lines.push(format!("{expr}.setLeft({});", points(is.0)));
            }
            if (was.1 - is.1).abs() > 0.25 {
                lines.push(format!("{expr}.setTop({});", points(is.1)));
            }
            if (now.size.0 - chart.size.0).abs() > 0.25 {
                lines.push(format!("{expr}.setWidth({});", points(chart.size.0)));
            }
            if (now.size.1 - chart.size.1).abs() > 0.25 {
                lines.push(format!("{expr}.setHeight({});", points(chart.size.1)));
            }
            if !lines.is_empty() {
                steps.push(Step {
                    comment: format!("Resize and move chart {name} on {on}"),
                    lines,
                });
            }
        }
    }
    steps
}
