//! The TypeScript the recorder writes: how values, addresses and sheets
//! read in it, the script Excel's recorder would have made of the same
//! actions, and the running of a piece of it on a workbook.

use opensheet_engine::refs::Rect;
use opensheet_engine::{Color, Workbook};
use opensheet_script::{Context, Outcome};
use std::collections::BTreeSet;

/// One thing a recorded action does: a comment and the statements under
/// it, as Excel's recorder writes them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Step {
    pub comment: String,
    pub lines: Vec<String>,
}

impl Step {
    pub fn new(comment: impl Into<String>, line: impl Into<String>) -> Step {
        Step {
            comment: comment.into(),
            lines: vec![line.into()],
        }
    }
}

/// How the sheets read in the script: the one that was in front when the
/// recording started is `selectedSheet`, the others are found by name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Scope {
    /// The name `selectedSheet` has now; `None` once it was deleted.
    pub start: Option<String>,
}

impl Scope {
    /// The expression that gives a sheet.
    pub fn sheet(&self, name: &str) -> String {
        if self.start.as_deref() == Some(name) { "selectedSheet".to_string() } else { format!("workbook.getWorksheet({})", quote(name)) }
    }

    /// The sheet in the words of a comment.
    pub fn on(&self, name: &str) -> String {
        if self.start.as_deref() == Some(name) { "selectedSheet".to_string() } else { name.to_string() }
    }

    /// A range of a sheet: `selectedSheet.getRange("A1:B2")`.
    pub fn range(&self, name: &str, rect: Rect) -> String {
        format!("{}.getRange({})", self.sheet(name), quote(&rect.name()))
    }
}

/// A text as TypeScript writes it.
pub(crate) fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // Control characters, and the two line ends a string of a
            // script cannot hold as they are.
            ch if (ch as u32) < 0x20 || ch == '\u{2028}' || ch == '\u{2029}' => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// A number as TypeScript writes it: no more digits than it takes to
/// read the same number back.
pub(crate) fn number(n: f64) -> String {
    if n == n.trunc() && n.abs() < 1e15 { format!("{}", n as i64) } else { format!("{n}") }
}

/// A size in points from pixels, short enough to read.
pub(crate) fn points(pixels: f32) -> String {
    number((pixels as f64 * 0.75 * 10_000.0).round() / 10_000.0)
}

pub(crate) fn color(color: Color) -> String {
    quote(&format!("#{:06X}", color.value()))
}

/// `ExcelScript.HorizontalAlignment.center`.
pub(crate) fn member(of: &str, name: &str) -> String {
    format!("ExcelScript.{of}.{name}")
}

/// The fewest rectangles that hold exactly these cells, top to bottom:
/// the runs of each row, joined with the same run of the rows below.
pub(crate) fn rects_of(cells: &BTreeSet<(u32, u32)>) -> Vec<Rect> {
    let mut done: Vec<Rect> = Vec::new();
    // The runs of the row before, still growing.
    let mut open: Vec<Rect> = Vec::new();
    let mut row_runs: Vec<Rect> = Vec::new();
    let flush = |row_runs: &mut Vec<Rect>, open: &mut Vec<Rect>, done: &mut Vec<Rect>| {
        let mut next = Vec::new();
        for run in row_runs.drain(..) {
            match open.iter().position(|o| o.c0 == run.c0 && o.c1 == run.c1 && o.r1 + 1 == run.r0) {
                Some(index) => {
                    let mut grown = open.swap_remove(index);
                    grown.r1 = run.r0;
                    next.push(grown);
                }
                None => next.push(run),
            }
        }
        done.append(open);
        *open = next;
    };
    let mut current: Option<Rect> = None;
    for &(row, col) in cells {
        match &mut current {
            Some(run) if run.r0 == row && run.c1 + 1 == col => run.c1 = col,
            _ => {
                let finished = current.replace(Rect::cell(row, col));
                if let Some(run) = finished {
                    let new_row = run.r0 != row;
                    row_runs.push(run);
                    if new_row {
                        flush(&mut row_runs, &mut open, &mut done);
                    }
                }
            }
        }
    }
    if let Some(run) = current {
        row_runs.push(run);
    }
    flush(&mut row_runs, &mut open, &mut done);
    done.append(&mut open);
    done.sort_by_key(|rect| (rect.r0, rect.c0));
    done
}

/// Runs of numbers that follow one another: `[2, 3, 4, 7]` is 2..=4 and 7.
pub(crate) fn runs_of(numbers: impl IntoIterator<Item = u32>) -> Vec<(u32, u32)> {
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for n in numbers {
        match runs.last_mut() {
            Some(run) if run.1 + 1 == n => run.1 = n,
            _ => runs.push((n, n)),
        }
    }
    runs
}

const INDENT: &str = "\t";

/// The script of some steps, as Excel's recorder lays one out.
pub(crate) fn script(steps: &[Step]) -> String {
    let mut out = String::from("function main(workbook: ExcelScript.Workbook) {\n");
    out.push_str(INDENT);
    out.push_str("let selectedSheet = workbook.getActiveWorksheet();\n");
    for step in steps {
        out.push_str(&text_of(step, INDENT));
    }
    out.push_str("}\n");
    out
}

/// One step as it reads in a script, each line after `indent`.
pub(crate) fn text_of(step: &Step, indent: &str) -> String {
    let mut out = format!("{indent}// {}\n", step.comment);
    for line in &step.lines {
        out.push_str(indent);
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Run some steps on a workbook, `selectedSheet` being the sheet at
/// `front`. On an error the workbook is left as it was.
pub(crate) fn play(wb: &mut Workbook, front: usize, steps: &[Step]) -> Result<Outcome, String> {
    let context = Context {
        active_sheet: front,
        ..Context::default()
    };
    wb.snapshot();
    let outcome = opensheet_script::run(&script(steps), wb, &context);
    match &outcome.error {
        Some(error) => {
            wb.rollback();
            Err(error.message.clone())
        }
        None => {
            wb.commit("");
            // A copy that is only played on needs no way back.
            wb.clear_history();
            Ok(outcome)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cells_are_gathered_into_rectangles() {
        let cells = |rects: &[Rect]| -> BTreeSet<(u32, u32)> {
            rects.iter().flat_map(|r| (r.r0..=r.r1).flat_map(move |row| (r.c0..=r.c1).map(move |col| (row, col)))).collect()
        };
        let names = |cells: &BTreeSet<(u32, u32)>| rects_of(cells).iter().map(Rect::name).collect::<Vec<_>>();
        assert_eq!(names(&cells(&[Rect::new(0, 0, 3, 2)])), ["A1:C4"]);
        assert_eq!(names(&cells(&[Rect::cell(4, 4)])), ["E5"]);
        assert_eq!(names(&cells(&[Rect::new(0, 0, 1, 1), Rect::new(0, 3, 1, 3), Rect::new(5, 0, 5, 4)])), ["A1:B2", "D1:D2", "A6:E6"]);
        // An L: the wide row cannot join the narrow ones.
        assert_eq!(names(&cells(&[Rect::new(0, 0, 2, 0), Rect::new(2, 1, 2, 2)])), ["A1:A2", "A3:C3"]);
        assert!(rects_of(&BTreeSet::new()).is_empty());
        assert_eq!(runs_of([2, 3, 4, 7]), [(2, 4), (7, 7)]);
    }

    #[test]
    fn values_read_as_typescript() {
        assert_eq!(quote("a \"b\"\n"), r#""a \"b\"\n""#);
        assert_eq!(number(3.0), "3");
        assert_eq!(number(-0.5), "-0.5");
        assert_eq!(number(1e21), "1000000000000000000000");
        assert_eq!(points(64.0), "48");
        assert_eq!(points(21.0), "15.75");
        assert_eq!(color(Color(0xFF00)), "\"#00FF00\"");
        let scope = Scope { start: Some("Data".to_string()) };
        assert_eq!(scope.range("Data", Rect::new(0, 0, 1, 1)), "selectedSheet.getRange(\"A1:B2\")");
        assert_eq!(scope.sheet("It's"), "workbook.getWorksheet(\"It's\")");
        let step = Step::new("Set range A1 on selectedSheet", "selectedSheet.getRange(\"A1\").setValue(1);");
        assert_eq!(
            script(&[step]),
            "function main(workbook: ExcelScript.Workbook) {\n\tlet selectedSheet = workbook.getActiveWorksheet();\n\t// Set range A1 on selectedSheet\n\tselectedSheet.getRange(\"A1\").setValue(1);\n}\n"
        );
    }
}
