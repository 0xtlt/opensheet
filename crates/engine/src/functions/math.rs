//! Math & trigonometry functions.

use super::{Category, FunctionDef, Kind, VAR};
use crate::ast::Expr;
use crate::eval::EvalCtx;
use crate::value::{R, Value};

fn f_sum(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    Ok(Value::Number(ctx.numbers(args)?.iter().sum()))
}

fn f_abs(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    Ok(Value::Number(ctx.num(&args[0])?.abs()))
}

pub const FUNCTIONS: &[FunctionDef] = &[
    FunctionDef::new("SUM", 1, VAR, Kind::Raw, Category::Math, f_sum),
    FunctionDef::new("ABS", 1, 1, Kind::Scalar, Category::Math, f_abs),
];
