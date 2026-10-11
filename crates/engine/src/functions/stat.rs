//! Statistical functions.

use super::{Category, FunctionDef, Kind, VAR};
use crate::ast::Expr;
use crate::eval::EvalCtx;
use crate::value::{ErrorKind, R, Value};

fn f_average(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let nums = ctx.numbers(args)?;
    if nums.is_empty() {
        return Err(ErrorKind::Div0);
    }
    Ok(Value::Number(nums.iter().sum::<f64>() / nums.len() as f64))
}

fn f_count(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let n = ctx
        .flatten(args)
        .iter()
        .filter(|v| matches!(v, Value::Number(_)))
        .count();
    Ok(Value::Number(n as f64))
}

fn f_max(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let nums = ctx.numbers(args)?;
    Ok(Value::Number(nums.into_iter().fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.max(x)))).unwrap_or(0.0)))
}

fn f_min(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let nums = ctx.numbers(args)?;
    Ok(Value::Number(nums.into_iter().fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.min(x)))).unwrap_or(0.0)))
}

pub const FUNCTIONS: &[FunctionDef] = &[
    FunctionDef::new("AVERAGE", 1, VAR, Kind::Raw, Category::Statistical, f_average),
    FunctionDef::new("COUNT", 1, VAR, Kind::Raw, Category::Statistical, f_count),
    FunctionDef::new("MAX", 1, VAR, Kind::Raw, Category::Statistical, f_max),
    FunctionDef::new("MIN", 1, VAR, Kind::Raw, Category::Statistical, f_min),
];
