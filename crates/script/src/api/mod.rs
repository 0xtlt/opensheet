//! The ExcelScript object model: one table per class, each member with
//! its TypeScript signature, a line of description and the code that
//! runs it. The classes a script sees are built from this table and
//! [`crate::api`] hands the same table out: what is listed is what runs.

use serde_json::Value as J;

use crate::ErrorKind;
use crate::host::Host;

mod areas;
mod chart;
mod comment;
mod condfmt;
mod enums;
mod format;
mod pivot;
mod protection;
mod range;
mod shape;
mod table;
mod util;
mod validation;
mod workbook;
mod worksheet;

/// A class of the Office Scripts API as far as OpenSheet has it.
pub struct ApiClass {
    /// The name in the `ExcelScript` namespace: `Range`, `Worksheet`...
    pub name: &'static str,
    pub description: &'static str,
    pub members: &'static [ApiMember],
}

/// A method of a class.
pub struct ApiMember {
    pub name: &'static str,
    /// The TypeScript signature: `getCell(row: number, column: number): Range`.
    pub signature: &'static str,
    pub description: &'static str,
    pub(crate) run: Handler,
}

/// An enum of the `ExcelScript` namespace.
pub struct ApiEnum {
    pub name: &'static str,
    /// Each member with its value: `("center", "Center")`.
    pub members: &'static [(&'static str, &'static str)],
}

pub(crate) type Handler = fn(&mut Host, &Call<'_>) -> Res<Ret>;
pub(crate) type Res<T> = Result<T, Fail>;

/// Why a call did not go through.
#[derive(Clone, Debug)]
pub(crate) struct Fail {
    pub kind: ErrorKind,
    pub message: String,
}

/// What a call gives back to the script.
pub(crate) enum Ret {
    /// `undefined`.
    Nothing,
    /// Plain data.
    Value(J),
    /// An object of the model, several of them in an array, or `null` for
    /// what does not exist (the script sees `undefined`).
    Object(J),
    /// A `Date`: milliseconds since 1970, UTC.
    Date(f64),
}

impl Ret {
    pub fn object(class: &str, state: Vec<J>) -> Ret {
        Ret::Object(object(class, state))
    }

    pub fn maybe(found: Option<J>) -> Ret {
        Ret::Object(found.unwrap_or(J::Null))
    }
}

impl<T: Into<J>> From<T> for Ret {
    fn from(value: T) -> Ret {
        Ret::Value(value.into())
    }
}

/// An object of the model as it travels to the script: its class and
/// the state its methods are called with.
pub(crate) fn object(class: &str, state: Vec<J>) -> J {
    let index = API.iter().position(|c| c.name == class).unwrap_or_default();
    serde_json::json!({ "$": index, "s": state })
}

/// One call of a method: the object it is called on and its arguments.
pub(crate) struct Call<'a> {
    pub class: &'static str,
    pub member: &'static str,
    pub this: &'a [J],
    pub args: &'a [J],
}

const INVALID: &str = "The argument is invalid or missing or has an incorrect format.";

impl<'a> Call<'a> {
    /// A failure worded as Excel words them: `Range setValues: ...`.
    pub fn fail(&self, message: impl AsRef<str>) -> Fail {
        Fail {
            kind: ErrorKind::Runtime,
            message: format!("{} {}: {}", self.class, self.member, message.as_ref()),
        }
    }

    pub fn invalid(&self) -> Fail {
        self.fail(INVALID)
    }

    /// The object the call was made on, or what it needs, is gone.
    pub fn gone(&self) -> Fail {
        self.fail("The requested resource doesn't exist.")
    }

    /// Something of the API that OpenSheet does not have.
    pub fn unsupported(&self, what: impl AsRef<str>) -> Fail {
        Fail {
            kind: ErrorKind::Unsupported,
            message: format!("{} is not supported in OpenSheet", what.as_ref()),
        }
    }

    pub fn arg(&self, index: usize) -> &'a J {
        self.args.get(index).unwrap_or(&J::Null)
    }

    pub fn has(&self, index: usize) -> bool {
        !self.arg(index).is_null()
    }

    pub fn str(&self, index: usize) -> Res<&'a str> {
        self.arg(index).as_str().ok_or_else(|| self.invalid())
    }

    pub fn opt_str(&self, index: usize) -> Res<Option<&'a str>> {
        if self.has(index) {
            self.str(index).map(Some)
        } else {
            Ok(None)
        }
    }

    pub fn num(&self, index: usize) -> Res<f64> {
        self.arg(index).as_f64().ok_or_else(|| self.invalid())
    }

    /// A whole number.
    pub fn int(&self, index: usize) -> Res<i64> {
        let n = self.num(index)?;
        if n.fract() == 0.0 && n.abs() < 1e15 {
            Ok(n as i64)
        } else {
            Err(self.invalid())
        }
    }

    pub fn opt_int(&self, index: usize) -> Res<Option<i64>> {
        if self.has(index) {
            self.int(index).map(Some)
        } else {
            Ok(None)
        }
    }

    pub fn bool(&self, index: usize) -> Res<bool> {
        self.arg(index).as_bool().ok_or_else(|| self.invalid())
    }

    pub fn opt_bool(&self, index: usize) -> Res<Option<bool>> {
        if self.has(index) {
            self.bool(index).map(Some)
        } else {
            Ok(None)
        }
    }

    pub fn array(&self, index: usize) -> Res<&'a [J]> {
        self.arg(index)
            .as_array()
            .map(Vec::as_slice)
            .ok_or_else(|| self.invalid())
    }

    /// A number of the state of the object the call is made on.
    pub fn this_int(&self, index: usize) -> i64 {
        self.this.get(index).and_then(J::as_i64).unwrap_or(-1)
    }
}

/// The state of an object of the model of that class, as it comes back
/// from the script.
pub(crate) fn state_of<'a>(value: &'a J, class: &str) -> Option<&'a [J]> {
    let index = value.get("$")?.as_u64()? as usize;
    (API.get(index)?.name == class).then_some(())?;
    value.get("s")?.as_array().map(Vec::as_slice)
}

/// Declares the table of a class: each member is its name, the rest of
/// its TypeScript signature, a line of description and its code.
macro_rules! class {
    ($name:literal, $doc:literal, { $( $member:ident $signature:literal $about:literal => $run:expr; )* }) => {
        $crate::api::ApiClass {
            name: $name,
            description: $doc,
            members: &[ $( $crate::api::ApiMember {
                name: stringify!($member),
                signature: concat!(stringify!($member), $signature),
                description: $about,
                run: $run,
            } ),* ],
        }
    };
}
pub(crate) use class;

/// Every class, `Workbook` first: it is the one `main` receives.
pub(crate) static API: &[ApiClass] = &[
    workbook::WORKBOOK,
    workbook::APPLICATION,
    workbook::NAMED_ITEM,
    worksheet::WORKSHEET,
    worksheet::FREEZE_PANES,
    protection::WORKSHEET_PROTECTION,
    worksheet::AUTO_FILTER,
    range::RANGE,
    range::RANGE_SORT,
    range::REMOVE_DUPLICATES_RESULT,
    format::RANGE_FORMAT,
    format::RANGE_FILL,
    format::RANGE_FONT,
    format::RANGE_BORDER,
    format::FORMAT_PROTECTION,
    table::TABLE,
    table::TABLE_COLUMN,
    table::FILTER,
    table::TABLE_SORT,
    chart::CHART,
    chart::CHART_TITLE,
    chart::CHART_LEGEND,
    areas::RANGE_AREAS,
    areas::RANGE_VIEW,
    condfmt::CONDITIONAL_FORMAT,
    condfmt::CELL_VALUE,
    condfmt::COLOR_SCALE,
    condfmt::DATA_BAR,
    condfmt::BAR_POSITIVE,
    condfmt::BAR_NEGATIVE,
    condfmt::ICON_SET,
    condfmt::PRESET,
    condfmt::TEXT,
    condfmt::TOP_BOTTOM,
    condfmt::CUSTOM,
    condfmt::FORMULA_RULE,
    condfmt::RANGE_FORMAT,
    condfmt::RANGE_FILL,
    condfmt::RANGE_FONT,
    condfmt::RANGE_BORDER,
    validation::DATA_VALIDATION,
    comment::COMMENT,
    comment::COMMENT_REPLY,
    comment::NOTE,
    pivot::PIVOT_TABLE,
    pivot::PIVOT_HIERARCHY,
    pivot::ROW_COLUMN_HIERARCHY,
    pivot::FILTER_HIERARCHY,
    pivot::DATA_HIERARCHY,
    pivot::PIVOT_FIELD,
    pivot::PIVOT_ITEM,
    pivot::PIVOT_LAYOUT,
    shape::SHAPE,
    shape::SHAPE_FILL,
    shape::SHAPE_LINE,
    shape::TEXT_FRAME,
    shape::TEXT_RANGE,
    shape::SHAPE_FONT,
    shape::IMAGE,
];

pub(crate) use enums::ENUMS;

/// The value of a member of an enum given as an argument: scripts pass
/// `ExcelScript.ClearApplyTo.contents`, which is the text `"Contents"`.
pub(crate) fn enum_value<'a>(call: &Call<'a>, index: usize, name: &str) -> Res<Option<&'a str>> {
    let Some(text) = call.opt_str(index)? else {
        return Ok(None);
    };
    let known = ENUMS
        .iter()
        .find(|e| e.name == name)
        .is_some_and(|e| e.members.iter().any(|(_, value)| *value == text));
    if known {
        Ok(Some(text))
    } else {
        Err(call.invalid())
    }
}

/// The members that answered a call in this process, for the test that
/// proves every listed member runs.
static CALLED: std::sync::Mutex<std::collections::BTreeSet<(&'static str, &'static str)>> =
    std::sync::Mutex::new(std::collections::BTreeSet::new());

pub(crate) fn note_call(class: &'static str, member: &'static str) {
    if let Ok(mut called) = CALLED.lock() {
        called.insert((class, member));
    }
}

pub(crate) fn called() -> Vec<(&'static str, &'static str)> {
    CALLED
        .lock()
        .map(|called| called.iter().copied().collect())
        .unwrap_or_default()
}
