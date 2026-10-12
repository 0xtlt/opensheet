//! OpenSheet calculation engine.
//!
//! Pure Rust, no UI dependencies: workbook model, formula parser and
//! evaluator, function library, number formats and editing operations.

pub mod ast;
pub mod cell_style;
pub mod chart;
pub mod chart_layout;
pub mod chart_trend;
pub mod comment;
pub mod condfmt;
pub mod consolidate;
pub mod datetime;
pub mod deps;
pub mod drawing;
pub mod eval;
pub mod features;
pub mod filter;
pub mod find;
pub mod flashfill;
pub mod format;
pub mod functions;
pub mod hyperlink;
pub mod input;
pub mod locale;
pub mod metrics;
pub mod names;
pub mod ops;
pub mod outline;
pub mod page;
pub mod parser;
pub mod paste_special;
pub mod pivot;
pub mod protect;
pub mod recalc;
pub mod refs;
pub mod sparkline;
pub mod structured;
pub mod subtotal;
pub mod style;
pub mod table;
pub mod text_columns;
pub mod validation;
pub mod value;
pub mod whatif;
pub mod workbook;

pub use ast::Expr;
pub use eval::{EvalCtx, EvalSession};
pub use locale::Locale;
pub use recalc::{CalcMode, IterativeCalc};
pub use refs::{Rect, Reference, MAX_COLS, MAX_ROWS};
pub use style::{Color, Style, StyleId};
pub use value::{Array, ErrorKind, Value, R};
pub use workbook::{Cell, Content, Sheet, Workbook};

#[cfg(test)]
mod tests;
