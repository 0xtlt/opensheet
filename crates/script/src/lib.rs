//! Office Scripts for OpenSheet.
//!
//! A script is TypeScript with a `main(workbook: ExcelScript.Workbook)`
//! function, as Excel's Automate tab writes them. [`run`] executes one on
//! a workbook; [`check`] reports what is wrong with its text without
//! running it; [`Script`] reads and writes the `.osts` files Excel keeps
//! scripts in.
//!
//! The contract with the application:
//!
//! * the caller takes `wb.snapshot()` before [`run`];
//! * when [`Outcome::error`] is `None` the changes of the script are in
//!   the workbook and the caller closes them as one undo step with
//!   `wb.commit(label)`;
//! * when it is `Some`, the caller drops them with `wb.rollback()`: a
//!   script that fails changes nothing;
//! * when it is `Some`, [`run`] has already dropped them and the
//!   workbook is as it was; the caller's `wb.rollback()` is still right
//!   (it then finds nothing to drop);
//! * [`run`] never panics and never runs longer than
//!   [`Context::time_limit`];
//! * a script gets the language and the `ExcelScript` object model,
//!   nothing else: no file, no network, no process, no environment.
//!
//! What a script can call is the table [`api`] gives (with [`enums`]);
//! anything else of the Office Scripts API fails with
//! [`ErrorKind::Unsupported`] and the name of what is missing. Types are
//! removed, not checked: a script with a type error runs.

use std::time::Duration;

mod api;
mod compile;
mod host;
mod runtime;

pub use api::{ApiClass, ApiEnum, ApiMember};

use opensheet_engine::refs::Rect;
use opensheet_engine::workbook::Workbook;

/// A script as the Automate tab lists it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Script {
    /// The name shown in the gallery; in a folder of scripts, the name of
    /// the file without `.osts`.
    pub name: String,
    pub description: String,
    /// The TypeScript text.
    pub body: String,
}

/// The version of the `.osts` format written.
const OSTS_VERSION: &str = "0.3.0";

impl Script {
    /// A new script holding the text Excel starts one with.
    pub fn new(name: &str) -> Script {
        Script {
            name: name.to_string(),
            description: String::new(),
            body: "function main(workbook: ExcelScript.Workbook) {\n  // Your code here\n}\n".to_string(),
        }
    }

    /// Read the text of an `.osts` file; `name` is the name of the file
    /// without its extension.
    pub fn from_osts(name: &str, text: &str) -> Result<Script, String> {
        let value: serde_json::Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let field = |key: &str| value.get(key).and_then(|v| v.as_str()).unwrap_or_default().to_string();
        if value.get("body").and_then(|v| v.as_str()).is_none() {
            return Err("no script body".to_string());
        }
        Ok(Script {
            name: name.to_string(),
            description: field("description"),
            body: field("body"),
        })
    }

    /// The text of the `.osts` file of this script.
    pub fn to_osts(&self) -> String {
        serde_json::json!({
            "version": OSTS_VERSION,
            "body": self.body,
            "description": self.description,
            "noCodeMetadata": "",
            "parameterInfo": "{\"version\":1,\"originalParameterOrder\":[],\"parameterSchema\":{\"type\":\"object\",\"default\":{},\"x-ms-visibility\":\"internal\"},\"returnSchema\":{\"type\":\"object\",\"properties\":{}},\"signature\":{\"comment\":\"\",\"parameters\":[{\"name\":\"workbook\",\"comment\":\"\"}]}}",
            "apiInfo": "{\"variant\":\"synchronous\",\"variantVersion\":2}",
        })
        .to_string()
    }
}

/// What the script sees of the window it runs in.
#[derive(Clone, Debug, PartialEq)]
pub struct Context {
    /// The sheet in front.
    pub active_sheet: usize,
    /// The selected range and the active cell (row, column) on it.
    pub selection: Rect,
    pub active_cell: (u32, u32),
    /// A script still running after this long is stopped with an error.
    pub time_limit: Duration,
}

impl Default for Context {
    fn default() -> Context {
        Context {
            active_sheet: 0,
            selection: Rect::new(0, 0, 0, 0),
            active_cell: (0, 0),
            time_limit: Duration::from_secs(120),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Log,
    Warn,
    Error,
}

/// One `console.log` (or `warn`, `error`) of the script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Log {
    pub level: LogLevel,
    pub text: String,
    /// The line of the script it came from (1-based), when known.
    pub line: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// The text is not valid TypeScript, or has no `main` function.
    Syntax,
    /// The script threw, or called something with the wrong arguments.
    Runtime,
    /// The script used a part of the Office Scripts API that OpenSheet
    /// does not have.
    Unsupported,
    /// The script ran past [`Context::time_limit`].
    Timeout,
}

/// Why a script did not run to its end. Lines and columns are 1-based
/// positions in the TypeScript text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptError {
    pub kind: ErrorKind,
    pub message: String,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

/// What running a script gave.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    pub logs: Vec<Log>,
    /// `None` when the script ran to its end.
    pub error: Option<ScriptError>,
    /// What `main` returned, as JSON text, when it returned something.
    pub returned: Option<String>,
    /// The sheet the script activated, and the range it selected (sheet,
    /// range), for the window to follow.
    pub activated: Option<usize>,
    pub selected: Option<(usize, Rect)>,
    /// The columns and rows the script asked to fit to their content.
    /// They were given the sizes the engine estimates without fonts; a
    /// window fits them again with the fonts it draws with, before it
    /// commits.
    pub autofit: Vec<Autofit>,
}

/// Columns (or rows) `first..=last` of a sheet, to fit to their content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Autofit {
    pub sheet: usize,
    /// Columns rather than rows.
    pub columns: bool,
    pub first: u32,
    pub last: u32,
}

/// What is wrong with the text of a script, without running it: syntax
/// errors and a missing `main`. Empty when it can be run. Types are not
/// checked.
pub fn check(source: &str) -> Vec<ScriptError> {
    compile::compile(source).err().unwrap_or_default()
}

/// Run the `main` of a script on a workbook. See the contract at the top
/// of this file.
pub fn run(source: &str, wb: &mut Workbook, context: &Context) -> Outcome {
    match compile::compile(source) {
        Ok(compiled) => runtime::run(compiled, wb, context),
        Err(mut errors) => Outcome {
            error: Some(errors.swap_remove(0)),
            ..Outcome::default()
        },
    }
}

/// The classes of the Office Scripts API a script can use, with every
/// member that works: the table the bindings themselves are made of.
pub fn api() -> &'static [ApiClass] {
    api::API
}

/// The enums of the `ExcelScript` namespace a script can use.
pub fn enums() -> &'static [ApiEnum] {
    api::ENUMS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_survives_its_osts_file() {
        let mut script = Script::new("Tidy");
        script.description = "Sorts \"things\"".to_string();
        script.body.push_str("// é\n");
        let back = Script::from_osts("Tidy", &script.to_osts()).expect("an osts file");
        assert_eq!(back, script);
        assert!(Script::from_osts("x", "{}").is_err());
        assert!(Script::from_osts("x", "not json").is_err());
    }
}
