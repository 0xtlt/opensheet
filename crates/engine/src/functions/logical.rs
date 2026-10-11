//! Logical functions.

use super::{Category, FunctionDef, Kind, VAR};
use crate::ast::Expr;
use crate::eval::{EvalCtx, compare_values, is_reference};
use crate::value::{Array, ErrorKind, R, Value};
use std::cmp::Ordering;

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

/// True for a value that must be handled element by element.
fn is_multi(v: &Value) -> bool {
    matches!(v, Value::Array(a) if a.len() != 1)
}

/// Common shape of several values under broadcasting.
fn broadcast_dims<'a>(values: impl IntoIterator<Item = &'a Value>) -> (usize, usize) {
    let (mut rows, mut cols) = (1, 1);
    for v in values {
        if is_multi(v) {
            let (r, c) = v.dims();
            rows = rows.max(r);
            cols = cols.max(c);
        }
    }
    (rows, cols)
}

/// `IFS(condition1, value1, ...)`: the value of the first true condition,
/// `#N/A` when none is. Lazy; once a condition is an array the remaining
/// pairs are evaluated and combined element-wise.
fn f_ifs(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    // Conditions and values come in pairs.
    if args.len() % 2 == 1 {
        return Err(ErrorKind::NA);
    }
    for (i, pair) in args.chunks(2).enumerate() {
        let cond = ctx.eval(&pair[0]);
        if is_multi(&cond) {
            return Ok(ifs_arrays(ctx, &args[i * 2..], cond));
        }
        if ctx.to_bool(&cond.into_scalar())? {
            return Ok(ctx.eval(&pair[1]));
        }
    }
    Err(ErrorKind::NA)
}

fn ifs_arrays(ctx: &EvalCtx, args: &[Expr], first: Value) -> Value {
    let mut conds = vec![first];
    conds.extend(args.chunks(2).skip(1).map(|pair| ctx.eval(&pair[0])));
    let (rows, cols) = broadcast_dims(&conds);
    // Result values are only evaluated when some element selects them.
    let mut results: Vec<Option<Value>> = vec![None; conds.len()];
    let mut out = Array::new(rows, cols);
    for r in 0..rows {
        for c in 0..cols {
            let mut picked = Value::Error(ErrorKind::NA);
            for (i, cond) in conds.iter().enumerate() {
                match ctx.to_bool(&cond.get_broadcast(r, c)) {
                    Ok(false) => {}
                    Ok(true) => {
                        let value = results[i].get_or_insert_with(|| ctx.eval(&args[i * 2 + 1]));
                        picked = value.get_broadcast(r, c);
                        break;
                    }
                    Err(e) => {
                        picked = Value::Error(e);
                        break;
                    }
                }
            }
            out.set(r, c, picked);
        }
    }
    Value::array(out)
}

/// Shared by `IFERROR` and `IFNA`: the fallback is only evaluated when an
/// error it handles is present; arrays are processed element-wise.
fn replace_errors(ctx: &EvalCtx, args: &[Expr], handles: fn(ErrorKind) -> bool) -> R<Value> {
    let value = ctx.eval(&args[0]);
    let caught = |v: &Value| matches!(v, Value::Error(e) if handles(*e));
    match value {
        Value::Array(a) if a.len() != 1 => {
            if !a.iter().any(caught) {
                return Ok(Value::Array(a));
            }
            let fallback = ctx.eval(&args[1]);
            let (fr, fc) = fallback.dims();
            let (rows, cols) = (a.rows.max(fr), a.cols.max(fc));
            let mut out = Array::new(rows, cols);
            for r in 0..rows {
                for c in 0..cols {
                    let v = a.get_broadcast(r, c);
                    let v = if caught(&v) {
                        fallback.get_broadcast(r, c)
                    } else {
                        v
                    };
                    out.set(r, c, v);
                }
            }
            Ok(Value::array(out))
        }
        value => {
            let value = value.into_scalar();
            if caught(&value) {
                Ok(ctx.eval(&args[1]))
            } else {
                Ok(value)
            }
        }
    }
}

fn f_iferror(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    replace_errors(ctx, args, |_| true)
}

fn f_ifna(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    replace_errors(ctx, args, |e| e == ErrorKind::NA)
}

#[derive(Clone, Copy)]
enum Fold {
    And,
    Or,
    Xor,
}

/// `AND` / `OR` / `XOR`. Inside ranges and arrays only numbers and
/// booleans count (text and blanks are skipped); a direct text argument
/// must spell TRUE or FALSE; with no logical value at all the result is
/// `#VALUE!`. Every argument is evaluated, so any error propagates.
fn fold_logical(ctx: &EvalCtx, args: &[Expr], fold: Fold) -> R<Value> {
    let mut acc: Option<bool> = None;
    let mut push = |b: bool| {
        acc = Some(match (acc, fold) {
            (None, _) => b,
            (Some(a), Fold::And) => a && b,
            (Some(a), Fold::Or) => a || b,
            (Some(a), Fold::Xor) => a ^ b,
        });
    };
    for arg in args {
        match ctx.eval(arg) {
            Value::Array(a) => {
                for v in a.iter() {
                    match v {
                        Value::Number(n) => push(*n != 0.0),
                        Value::Bool(b) => push(*b),
                        Value::Error(e) => return Err(*e),
                        _ => {}
                    }
                }
            }
            Value::Number(n) => push(n != 0.0),
            Value::Bool(b) => push(b),
            Value::Error(e) => return Err(e),
            Value::Empty => {
                // An omitted argument is FALSE; a blank cell is skipped.
                if ctx.is_missing(arg) {
                    push(false);
                }
            }
            Value::Text(s) => {
                if !is_reference(arg) {
                    push(ctx.locale().parse_bool(&s).ok_or(ErrorKind::Value)?);
                }
            }
        }
    }
    acc.map(Value::Bool).ok_or(ErrorKind::Value)
}

fn f_and(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    fold_logical(ctx, args, Fold::And)
}

fn f_or(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    fold_logical(ctx, args, Fold::Or)
}

fn f_xor(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    fold_logical(ctx, args, Fold::Xor)
}

fn f_not(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    Ok(Value::Bool(!ctx.boolean(&args[0])?))
}

fn f_true(_ctx: &EvalCtx, _args: &[Expr]) -> R<Value> {
    Ok(Value::Bool(true))
}

fn f_false(_ctx: &EvalCtx, _args: &[Expr]) -> R<Value> {
    Ok(Value::Bool(false))
}

/// `SWITCH(expression, value1, result1, ..., [default])`: the result paired
/// with the first value equal to the expression. Lazy for a scalar
/// expression; element-wise for an array.
fn f_switch(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let cases = &args[1..];
    let pairs = cases.len() / 2;
    let default = if cases.len() % 2 == 1 {
        cases.last()
    } else {
        None
    };
    let target = ctx.eval(&args[0]);

    if is_multi(&target) {
        let values: Vec<Value> = (0..pairs).map(|i| ctx.eval_scalar(&cases[i * 2])).collect();
        let mut results: Vec<Option<Value>> = vec![None; pairs];
        let mut fallback: Option<Value> = None;
        let (rows, cols) = target.dims();
        let mut out = Array::new(rows, cols);
        for r in 0..rows {
            for c in 0..cols {
                let t = target.get_broadcast(r, c);
                let v = match switch_pick(&t, &values) {
                    Err(e) => Value::Error(e),
                    Ok(Some(i)) => results[i]
                        .get_or_insert_with(|| ctx.eval(&cases[i * 2 + 1]))
                        .get_broadcast(r, c),
                    Ok(None) => match default {
                        Some(e) => fallback
                            .get_or_insert_with(|| ctx.eval(e))
                            .get_broadcast(r, c),
                        None => Value::Error(ErrorKind::NA),
                    },
                };
                out.set(r, c, v);
            }
        }
        return Ok(Value::array(out));
    }

    let target = target.into_scalar();
    if let Value::Error(e) = target {
        return Err(e);
    }
    for i in 0..pairs {
        let value = ctx.eval_scalar(&cases[i * 2]);
        if let Value::Error(e) = value {
            return Err(e);
        }
        if compare_values(&target, &value) == Ordering::Equal {
            return Ok(ctx.eval(&cases[i * 2 + 1]));
        }
    }
    match default {
        Some(e) => Ok(ctx.eval(e)),
        None => Err(ErrorKind::NA),
    }
}

/// Index of the first case value equal to `target`.
fn switch_pick(target: &Value, values: &[Value]) -> R<Option<usize>> {
    if let Value::Error(e) = target {
        return Err(*e);
    }
    for (i, value) in values.iter().enumerate() {
        if let Value::Error(e) = value {
            return Err(*e);
        }
        if compare_values(target, value) == Ordering::Equal {
            return Ok(Some(i));
        }
    }
    Ok(None)
}

#[rustfmt::skip]
pub const FUNCTIONS: &[FunctionDef] = &[
    FunctionDef::new("IF", 2, 3, Kind::Raw, Category::Logical, f_if),
    FunctionDef::new("IFS", 2, VAR, Kind::Raw, Category::Logical, f_ifs),
    FunctionDef::new("IFERROR", 2, 2, Kind::Raw, Category::Logical, f_iferror),
    FunctionDef::new("IFNA", 2, 2, Kind::Raw, Category::Logical, f_ifna),
    FunctionDef::new("AND", 1, VAR, Kind::Raw, Category::Logical, f_and),
    FunctionDef::new("OR", 1, VAR, Kind::Raw, Category::Logical, f_or),
    FunctionDef::new("XOR", 1, VAR, Kind::Raw, Category::Logical, f_xor),
    FunctionDef::new("NOT", 1, 1, Kind::Scalar, Category::Logical, f_not),
    FunctionDef::new("TRUE", 0, 0, Kind::Raw, Category::Logical, f_true),
    FunctionDef::new("FALSE", 0, 0, Kind::Raw, Category::Logical, f_false),
    FunctionDef::new("SWITCH", 3, VAR, Kind::Raw, Category::Logical, f_switch),
];

#[cfg(test)]
mod tests {
    use crate::tests::{eval, num, val, wb};
    use crate::value::{ErrorKind, Value};

    fn err(e: ErrorKind) -> Value {
        Value::Error(e)
    }

    #[test]
    fn if_basics() {
        assert_eq!(num("IF(1>0,10,20)"), 10.0);
        assert_eq!(num("IF(0,10,20)"), 20.0);
        assert_eq!(num("IF(TRUE,1,1/0)"), 1.0);
        assert_eq!(num("IF(FALSE,1/0,2)"), 2.0);
        assert_eq!(eval("IF(FALSE,1)"), Value::Bool(false));
        assert_eq!(num("IF(TRUE,,5)"), 0.0);
        assert_eq!(eval("IF(\"x\",1,2)"), err(ErrorKind::Value));
        assert_eq!(eval("IF(1/0,1,2)"), err(ErrorKind::Div0));
        let w = wb(&[("A1", "1"), ("A2", "0"), ("C1", "=IF(A1:A2,\"y\",\"n\")")]);
        assert_eq!(val(&w, "C1"), Value::text("y"));
        assert_eq!(val(&w, "C2"), Value::text("n"));
    }

    #[test]
    fn ifs() {
        assert_eq!(eval("IFS(FALSE,\"a\",TRUE,\"b\")"), Value::text("b"));
        assert_eq!(eval("IFS(1,\"a\",TRUE,\"b\")"), Value::text("a"));
        // Lazy: branches after the first match are not evaluated.
        assert_eq!(num("IFS(TRUE,1,1/0,2)"), 1.0);
        assert_eq!(num("IFS(FALSE,1/0,TRUE,2)"), 2.0);
        assert_eq!(eval("IFS(FALSE,1,FALSE,2)"), err(ErrorKind::NA));
        assert_eq!(eval("IFS(FALSE,1,1/0,2)"), err(ErrorKind::Div0));
        assert_eq!(eval("IFS(\"x\",1)"), err(ErrorKind::Value));
        assert_eq!(eval("IFS(TRUE,1,TRUE)"), err(ErrorKind::NA));
        let w = wb(&[
            ("A1", "95"),
            ("A2", "72"),
            ("A3", "10"),
            ("B1", "=IFS(A1>=90,\"A\",A1>=70,\"C\",TRUE,\"F\")"),
            ("C1", "=IFS(A1:A3>=90,\"A\",A1:A3>=70,\"C\",TRUE,\"F\")"),
            ("E1", "=IFS(A1:A3>=90,\"A\",A1:A3>=70,\"C\")"),
        ]);
        assert_eq!(val(&w, "B1"), Value::text("A"));
        assert_eq!(val(&w, "C1"), Value::text("A"));
        assert_eq!(val(&w, "C2"), Value::text("C"));
        assert_eq!(val(&w, "C3"), Value::text("F"));
        assert_eq!(val(&w, "E3"), err(ErrorKind::NA));
    }

    #[test]
    fn iferror_and_ifna() {
        assert_eq!(num("IFERROR(1/0,7)"), 7.0);
        assert_eq!(num("IFERROR(5,7)"), 5.0);
        assert_eq!(eval("IFERROR(NOSUCHFN(),\"n\")"), Value::text("n"));
        assert_eq!(eval("IFERROR(#N/A,\"na\")"), Value::text("na"));
        // The fallback is only evaluated when needed, and may itself fail.
        assert_eq!(num("IFERROR(5,1/0)"), 5.0);
        assert_eq!(eval("IFERROR(1/0,#REF!)"), err(ErrorKind::Ref));
        assert_eq!(num("IFERROR(1/0,)"), 0.0);
        assert_eq!(eval("IFNA(#N/A,\"na\")"), Value::text("na"));
        assert_eq!(eval("IFNA(1/0,\"na\")"), err(ErrorKind::Div0));
        assert_eq!(num("IFNA(3,1/0)"), 3.0);
        assert_eq!(num("SUM(IFERROR({1,#DIV/0!,3},10))"), 14.0);
        assert_eq!(num("SUM(IFNA({1,#N/A,3},10))"), 14.0);
        let w = wb(&[
            ("A1", "4"),
            ("A2", "0"),
            ("A3", "=NA()"),
            ("C1", "=IFERROR(8/A1:A3,-1)"),
            ("D1", "=IFNA(A1:A3,\"none\")"),
            ("E1", "=IFNA(8/A1:A3,0)"),
        ]);
        assert_eq!(val(&w, "C1"), Value::Number(2.0));
        assert_eq!(val(&w, "C2"), Value::Number(-1.0));
        assert_eq!(val(&w, "C3"), Value::Number(-1.0));
        assert_eq!(val(&w, "D1"), Value::Number(4.0));
        assert_eq!(val(&w, "D3"), Value::text("none"));
        assert_eq!(val(&w, "E2"), err(ErrorKind::Div0));
        assert_eq!(val(&w, "E3"), Value::Number(0.0));
    }

    #[test]
    fn and_or_xor() {
        assert_eq!(eval("AND(TRUE,1,2>1)"), Value::Bool(true));
        assert_eq!(eval("AND(TRUE,0)"), Value::Bool(false));
        assert_eq!(eval("OR(FALSE,0)"), Value::Bool(false));
        assert_eq!(eval("OR(FALSE,3)"), Value::Bool(true));
        assert_eq!(eval("XOR(TRUE,TRUE)"), Value::Bool(false));
        assert_eq!(eval("XOR(TRUE,FALSE,FALSE)"), Value::Bool(true));
        assert_eq!(eval("XOR(TRUE,TRUE,TRUE)"), Value::Bool(true));
        // Direct text must spell a boolean.
        assert_eq!(eval("AND(\"true\",TRUE)"), Value::Bool(true));
        assert_eq!(eval("OR(\"FALSE\")"), Value::Bool(false));
        assert_eq!(eval("AND(\"abc\",TRUE)"), err(ErrorKind::Value));
        assert_eq!(eval("OR(\"1\")"), err(ErrorKind::Value));
        // No short-circuit: errors always propagate.
        assert_eq!(eval("AND(FALSE,1/0)"), err(ErrorKind::Div0));
        assert_eq!(eval("OR(TRUE,#N/A)"), err(ErrorKind::NA));
        assert_eq!(eval("AND(TRUE,)"), Value::Bool(false));
        assert_eq!(eval("AND({1,1,0})"), Value::Bool(false));
        assert_eq!(eval("OR({0,\"x\",2})"), Value::Bool(true));
        assert_eq!(eval("XOR({1,1},TRUE)"), Value::Bool(true));
        let w = wb(&[
            ("A1", "TRUE"),
            ("A2", "some text"),
            ("A4", "1"),
            ("B1", "=AND(A1:A4)"),
            ("B2", "=OR(A2:A3)"),
            ("B3", "=AND(A2)"),
            ("B4", "=AND(A3,A1)"),
            ("B5", "=XOR(A1:A4)"),
            ("B6", "=OR(A2,A3,A4)"),
            ("B7", "=AND(A1:A4,0)"),
        ]);
        // Text and blanks inside ranges are ignored.
        assert_eq!(val(&w, "B1"), Value::Bool(true));
        assert_eq!(val(&w, "B2"), err(ErrorKind::Value));
        assert_eq!(val(&w, "B3"), err(ErrorKind::Value));
        assert_eq!(val(&w, "B4"), Value::Bool(true));
        assert_eq!(val(&w, "B5"), Value::Bool(false));
        assert_eq!(val(&w, "B6"), Value::Bool(true));
        assert_eq!(val(&w, "B7"), Value::Bool(false));
    }

    #[test]
    fn not_true_false() {
        assert_eq!(eval("NOT(TRUE)"), Value::Bool(false));
        assert_eq!(eval("NOT(0)"), Value::Bool(true));
        assert_eq!(eval("NOT(5)"), Value::Bool(false));
        assert_eq!(eval("NOT(\"false\")"), Value::Bool(true));
        assert_eq!(eval("NOT(\"abc\")"), err(ErrorKind::Value));
        assert_eq!(eval("NOT(#N/A)"), err(ErrorKind::NA));
        assert_eq!(eval("TRUE()"), Value::Bool(true));
        assert_eq!(eval("FALSE()"), Value::Bool(false));
        assert_eq!(eval("TRUE(1)"), err(ErrorKind::Value));
        assert_eq!(num("TRUE()+TRUE"), 2.0);
        let w = wb(&[("A1", "1"), ("A2", "0"), ("C1", "=NOT(A1:A3)")]);
        assert_eq!(val(&w, "C1"), Value::Bool(false));
        assert_eq!(val(&w, "C2"), Value::Bool(true));
    }

    #[test]
    fn switch() {
        assert_eq!(eval("SWITCH(2,1,\"one\",2,\"two\")"), Value::text("two"));
        assert_eq!(
            eval("SWITCH(3,1,\"one\",2,\"two\",\"other\")"),
            Value::text("other")
        );
        assert_eq!(eval("SWITCH(3,1,\"one\",2,\"two\")"), err(ErrorKind::NA));
        assert_eq!(num("SWITCH(\"B\",\"a\",1,\"b\",2)"), 2.0);
        // No implicit conversion between types.
        assert_eq!(
            eval("SWITCH(\"1\",1,\"num\",\"text\")"),
            Value::text("text")
        );
        assert_eq!(
            eval("SWITCH(TRUE,1,\"num\",TRUE,\"bool\")"),
            Value::text("bool")
        );
        // Lazy: only the matching result (or the default) is evaluated.
        assert_eq!(num("SWITCH(1,1,10,2,1/0)"), 10.0);
        assert_eq!(num("SWITCH(1,1,10,1/0)"), 10.0);
        assert_eq!(num("SWITCH(2,1,1/0,2,20)"), 20.0);
        assert_eq!(eval("SWITCH(5,1,10,1/0)"), err(ErrorKind::Div0));
        assert_eq!(eval("SWITCH(1/0,1,10,99)"), err(ErrorKind::Div0));
        assert_eq!(eval("SWITCH(#N/A,1,10)"), err(ErrorKind::NA));
        assert_eq!(num("SWITCH(WEEKDAY(DATE(2024,10,6)),1,100,7,700,0)"), 100.0);
        let w = wb(&[
            ("A1", "1"),
            ("A2", "2"),
            ("A3", "9"),
            ("C1", "=SWITCH(A1:A3,1,\"one\",2,\"two\",\"?\")"),
            ("D1", "=SWITCH(A1:A3,1,\"one\",2,\"two\")"),
        ]);
        assert_eq!(val(&w, "C1"), Value::text("one"));
        assert_eq!(val(&w, "C2"), Value::text("two"));
        assert_eq!(val(&w, "C3"), Value::text("?"));
        assert_eq!(val(&w, "D3"), err(ErrorKind::NA));
    }
}
