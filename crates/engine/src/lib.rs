//! OpenSheet calculation engine.
//!
//! Pure Rust, no UI dependencies: workbook model, formula parser and
//! evaluator, function library, number formats and editing operations.

pub mod ast;
pub mod chart;
pub mod comment;
pub mod condfmt;
pub mod datetime;
pub mod drawing;
pub mod eval;
pub mod features;
pub mod filter;
pub mod format;
pub mod functions;
pub mod input;
pub mod locale;
pub mod names;
pub mod ops;
pub mod page;
pub mod parser;
pub mod pivot;
pub mod protect;
pub mod refs;
pub mod structured;
pub mod style;
pub mod table;
pub mod validation;
pub mod value;
pub mod workbook;

pub use ast::Expr;
pub use eval::EvalCtx;
pub use locale::Locale;
pub use refs::{Rect, Reference, MAX_COLS, MAX_ROWS};
pub use style::{Color, Style, StyleId};
pub use value::{Array, ErrorKind, Value, R};
pub use workbook::{Cell, Content, Sheet, Workbook};

#[cfg(test)]
mod tests;
