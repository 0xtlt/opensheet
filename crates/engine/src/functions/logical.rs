//! Logical functions.

use super::{Category, FunctionDef, Kind};
use crate::ast::Expr;
use crate::eval::EvalCtx;
use crate::value::{Array, R, Value};

/// `IF` is lazy: only the chosen branch is evaluated. With an array
/// condition both branches are evaluated and combined element-wise.
fn f_if(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let cond = ctx.eval(&args[0]);
    let branch = |pick: bool| -> Value {
        match args.get(if pick { 1 } else { 2 }) {
            Some(Expr::Missing) => Value::Number(0.0),
            Some(e) => ctx.eval(e),
            None => Value::Bool(pick),
        }
    };
    match cond {
        Value::Array(a) if a.len() != 1 => {
            let yes = branch(true);
            let no = branch(false);
            let mut out = Array::new(a.rows, a.cols);
            for r in 0..a.rows {
                for c in 0..a.cols {
                    let v = match ctx.to_bool(a.get(r, c)) {
                        Ok(true) => yes.get_broadcast(r, c),
                        Ok(false) => no.get_broadcast(r, c),
                        Err(e) => Value::Error(e),
                    };
                    out.set(r, c, v);
                }
            }
            Ok(Value::array(out))
        }
        cond => Ok(branch(ctx.to_bool(&cond.into_scalar())?)),
    }
}

pub const FUNCTIONS: &[FunctionDef] = &[
    FunctionDef::new("IF", 2, 3, Kind::Raw, Category::Logical, f_if),
];
