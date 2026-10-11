//! Built-in function library.
//!
//! Each family lives in its own file and exports `FUNCTIONS`, a table of
//! [`FunctionDef`]. Names are canonical (English, uppercase); localized
//! spellings are handled by `locale.rs` at the parse / print boundary.
//!
//! # Writing a function
//!
//! ```ignore
//! fn f_power(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
//!     let base = ctx.num(&args[0])?;      // `?` propagates error values
//!     let exp = ctx.num(&args[1])?;
//!     Ok(Value::Number(base.powf(exp)))
//! }
//! // name, min args, max args (VAR = unlimited), kind, category, fn
//! FunctionDef::new("POWER", 2, 2, Kind::Scalar, Category::Math, f_power)
//! ```
//!
//! * `Kind::Scalar`: every parameter is a single value. The dispatcher
//!   evaluates the arguments first and, when one is an array or range, calls
//!   the function once per element and returns an array ("lifting"). The
//!   function only ever sees scalars.
//! * `Kind::Raw`: the function receives the unevaluated argument
//!   expressions and decides what to evaluate (aggregates over ranges, lazy
//!   functions such as `IF`, functions needing references such as `ROW`).

use crate::ast::Expr;
use crate::eval::EvalCtx;
use crate::refs::Reference;
use crate::value::{R, Value};
use std::collections::HashMap;
use std::sync::OnceLock;

pub mod criteria;
pub mod date;
pub mod engineering;
pub mod financial;
pub mod info;
pub mod logical;
pub mod lookup;
pub mod math;
pub mod stat;
pub mod text;

pub type FnImpl = fn(&EvalCtx, &[Expr]) -> R<Value>;
pub type RefFn = fn(&EvalCtx, &[Expr]) -> R<Reference>;

/// Use as `max_args` for functions taking any number of arguments.
pub const VAR: u8 = u8::MAX;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Scalar,
    Raw,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    Math,
    Statistical,
    Logical,
    Text,
    DateTime,
    Lookup,
    Information,
    Financial,
    Engineering,
}

pub struct FunctionDef {
    pub name: &'static str,
    pub min_args: u8,
    pub max_args: u8,
    pub kind: Kind,
    pub category: Category,
    pub func: FnImpl,
    /// For functions that can return a reference (`OFFSET`, `INDIRECT`,
    /// `INDEX`): used when the caller needs cells rather than values.
    pub ref_func: Option<RefFn>,
}

impl FunctionDef {
    pub const fn new(
        name: &'static str,
        min_args: u8,
        max_args: u8,
        kind: Kind,
        category: Category,
        func: FnImpl,
    ) -> FunctionDef {
        FunctionDef {
            name,
            min_args,
            max_args,
            kind,
            category,
            func,
            ref_func: None,
        }
    }

    pub const fn with_ref(mut self, f: RefFn) -> FunctionDef {
        self.ref_func = Some(f);
        self
    }
}

fn registry() -> &'static HashMap<&'static str, &'static FunctionDef> {
    static REGISTRY: OnceLock<HashMap<&'static str, &'static FunctionDef>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut map = HashMap::new();
        let families: [&'static [FunctionDef]; 9] = [
            math::FUNCTIONS,
            stat::FUNCTIONS,
            logical::FUNCTIONS,
            text::FUNCTIONS,
            date::FUNCTIONS,
            lookup::FUNCTIONS,
            info::FUNCTIONS,
            financial::FUNCTIONS,
            engineering::FUNCTIONS,
        ];
        for family in families {
            for def in family {
                map.insert(def.name, def);
            }
        }
        map
    })
}

/// Find a function by canonical name.
pub fn lookup(name: &str) -> Option<&'static FunctionDef> {
    registry().get(name).copied()
}

/// All functions, sorted by name.
pub fn all() -> Vec<&'static FunctionDef> {
    let mut v: Vec<_> = registry().values().copied().collect();
    v.sort_by_key(|d| d.name);
    v
}

/// Case-insensitive wildcard match: `?` matches one character, `*` any run,
/// `~` escapes the next character.
pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    #[derive(Clone, Copy, PartialEq)]
    enum Tok {
        Any,
        One,
        Lit(char),
    }
    let mut toks = Vec::new();
    let mut chars = pattern.chars();
    while let Some(c) = chars.next() {
        match c {
            '*' => {
                if toks.last() != Some(&Tok::Any) {
                    toks.push(Tok::Any);
                }
            }
            '?' => toks.push(Tok::One),
            '~' => match chars.next() {
                Some(n) => toks.extend(n.to_lowercase().map(Tok::Lit)),
                None => toks.push(Tok::Lit('~')),
            },
            c => toks.extend(c.to_lowercase().map(Tok::Lit)),
        }
    }
    let text: Vec<char> = text.chars().flat_map(|c| c.to_lowercase()).collect();
    // Iterative glob matching with backtracking to the last `*`.
    let (mut t, mut p) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        match toks.get(p) {
            Some(Tok::Lit(c)) if *c == text[t] => {
                t += 1;
                p += 1;
            }
            Some(Tok::One) => {
                t += 1;
                p += 1;
            }
            Some(Tok::Any) => {
                star = Some((p, t));
                p += 1;
            }
            _ => match star {
                Some((sp, st)) => {
                    p = sp + 1;
                    t = st + 1;
                    star = Some((sp, st + 1));
                }
                None => return false,
            },
        }
    }
    while toks.get(p) == Some(&Tok::Any) {
        p += 1;
    }
    p == toks.len()
}

/// True when the pattern contains unescaped wildcard characters.
pub fn has_wildcards(pattern: &str) -> bool {
    pattern.contains(['*', '?', '~'])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards() {
        assert!(wildcard_match("a*c", "ABBBC"));
        assert!(wildcard_match("a?c", "abc"));
        assert!(!wildcard_match("a?c", "abbc"));
        assert!(wildcard_match("*", ""));
        assert!(wildcard_match("~*x", "*x"));
        assert!(!wildcard_match("~*x", "ax"));
        assert!(wildcard_match("*apple*", "Green Apple pie"));
    }
}
