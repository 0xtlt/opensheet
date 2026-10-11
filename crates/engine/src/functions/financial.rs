//! Financial functions.
//!
//! Cash-flow sign convention as in Excel: money paid out is negative, money
//! received is positive. `type` is 0 for payments at the end of each period
//! (the default) and non-zero for payments at the beginning.

use super::{Category, FunctionDef, Kind, VAR};
use crate::ast::Expr;
use crate::eval::EvalCtx;
use crate::value::{ErrorKind, R, Value};

/// A finite number, or `#NUM!`.
fn finite(x: f64) -> R<Value> {
    if x.is_finite() {
        Ok(Value::Number(x))
    } else {
        Err(ErrorKind::Num)
    }
}

/// Optional `type` argument: true when payments are due at the beginning.
fn due_arg(ctx: &EvalCtx, args: &[Expr], index: usize) -> R<bool> {
    Ok(ctx.num_or(args, index, 0.0)? != 0.0)
}

// ---- time value of money -------------------------------------------------

/// `(1 + rate)^nper`.
fn growth(rate: f64, nper: f64) -> f64 {
    (1.0 + rate).powf(nper)
}

fn pmt(rate: f64, nper: f64, pv: f64, fv: f64, due: bool) -> f64 {
    if rate == 0.0 {
        return -(pv + fv) / nper;
    }
    let g = growth(rate, nper);
    let due_factor = if due { 1.0 + rate } else { 1.0 };
    -(rate * (fv + pv * g)) / (due_factor * (g - 1.0))
}

fn fv(rate: f64, nper: f64, pmt: f64, pv: f64, due: bool) -> f64 {
    if rate == 0.0 {
        return -(pv + pmt * nper);
    }
    let g = growth(rate, nper);
    let due_factor = if due { 1.0 + rate } else { 1.0 };
    -(pv * g + pmt * due_factor * (g - 1.0) / rate)
}

fn pv(rate: f64, nper: f64, pmt: f64, fv: f64, due: bool) -> f64 {
    if rate == 0.0 {
        return -(fv + pmt * nper);
    }
    let g = growth(rate, nper);
    let due_factor = if due { 1.0 + rate } else { 1.0 };
    -(fv + pmt * due_factor * (g - 1.0) / rate) / g
}

/// Interest part of the payment for period `per` (1-based).
fn ipmt(rate: f64, per: f64, nper: f64, pv: f64, fv_target: f64, due: bool) -> f64 {
    let payment = pmt(rate, nper, pv, fv_target, due);
    if per == 1.0 {
        if due { 0.0 } else { -pv * rate }
    } else if due {
        (fv(rate, per - 2.0, payment, pv, true) - payment) * rate
    } else {
        fv(rate, per - 1.0, payment, pv, false) * rate
    }
}

fn f_pmt(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let rate = ctx.num(&args[0])?;
    let nper = ctx.num(&args[1])?;
    let pv = ctx.num(&args[2])?;
    let fv = ctx.num_or(args, 3, 0.0)?;
    let due = due_arg(ctx, args, 4)?;
    finite(pmt(rate, nper, pv, fv, due))
}

fn f_pv(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let rate = ctx.num(&args[0])?;
    let nper = ctx.num(&args[1])?;
    let payment = ctx.num_or(args, 2, 0.0)?;
    let fv = ctx.num_or(args, 3, 0.0)?;
    let due = due_arg(ctx, args, 4)?;
    finite(pv(rate, nper, payment, fv, due))
}

fn f_fv(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let rate = ctx.num(&args[0])?;
    let nper = ctx.num(&args[1])?;
    let payment = ctx.num_or(args, 2, 0.0)?;
    let pv = ctx.num_or(args, 3, 0.0)?;
    let due = due_arg(ctx, args, 4)?;
    finite(fv(rate, nper, payment, pv, due))
}

fn f_nper(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let rate = ctx.num(&args[0])?;
    let payment = ctx.num(&args[1])?;
    let pv = ctx.num(&args[2])?;
    let fv = ctx.num_or(args, 3, 0.0)?;
    let due = due_arg(ctx, args, 4)?;
    if rate == 0.0 {
        if payment == 0.0 {
            return Err(ErrorKind::Div0);
        }
        return finite(-(pv + fv) / payment);
    }
    let due_factor = if due { 1.0 + rate } else { 1.0 };
    let z = payment * due_factor / rate;
    let ratio = (z - fv) / (z + pv);
    if ratio.is_nan() || ratio <= 0.0 || rate <= -1.0 {
        return Err(ErrorKind::Num);
    }
    finite(ratio.ln() / rate.ln_1p())
}

/// Arguments shared by `IPMT` and `PPMT`: (rate, per, nper, pv, fv, due).
fn period_args(ctx: &EvalCtx, args: &[Expr]) -> R<(f64, f64, f64, f64, f64, bool)> {
    let rate = ctx.num(&args[0])?;
    let per = ctx.num(&args[1])?;
    let nper = ctx.num(&args[2])?;
    let pv = ctx.num(&args[3])?;
    let fv = ctx.num_or(args, 4, 0.0)?;
    let due = due_arg(ctx, args, 5)?;
    if per < 1.0 || per > nper {
        return Err(ErrorKind::Num);
    }
    Ok((rate, per, nper, pv, fv, due))
}

fn f_ipmt(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let (rate, per, nper, pv, fv, due) = period_args(ctx, args)?;
    finite(ipmt(rate, per, nper, pv, fv, due))
}

fn f_ppmt(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let (rate, per, nper, pv, fv, due) = period_args(ctx, args)?;
    finite(pmt(rate, nper, pv, fv, due) - ipmt(rate, per, nper, pv, fv, due))
}

/// Shared by `CUMIPMT` and `CUMPRINC`: sum the interest (or principal)
/// parts of the payments for periods `start..=end`.
fn cumulative(ctx: &EvalCtx, args: &[Expr], principal: bool) -> R<Value> {
    let rate = ctx.num(&args[0])?;
    let nper = ctx.num(&args[1])?;
    let pv = ctx.num(&args[2])?;
    let start = ctx.num(&args[3])?.trunc();
    let end = ctx.num(&args[4])?.trunc();
    let kind = ctx.num(&args[5])?;
    if rate <= 0.0 || nper <= 0.0 || pv <= 0.0 {
        return Err(ErrorKind::Num);
    }
    if start < 1.0 || end < start || end > nper || (kind != 0.0 && kind != 1.0) {
        return Err(ErrorKind::Num);
    }
    if end - start > 10_000_000.0 {
        return Err(ErrorKind::Num);
    }
    let due = kind == 1.0;
    let payment = pmt(rate, nper, pv, 0.0, due);
    let mut total = 0.0;
    let mut per = start;
    while per <= end {
        let interest = ipmt(rate, per, nper, pv, 0.0, due);
        total += if principal {
            payment - interest
        } else {
            interest
        };
        per += 1.0;
    }
    finite(total)
}

fn f_cumipmt(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    cumulative(ctx, args, false)
}

fn f_cumprinc(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    cumulative(ctx, args, true)
}

// ---- root finding --------------------------------------------------------

/// Find `r > -1` with `f(r) = 0`: Newton's method from `guess`, falling
/// back to a scan for a sign change followed by bisection.
fn find_rate(f: &dyn Fn(f64) -> f64, guess: f64) -> Option<f64> {
    const LOW: f64 = -1.0 + 1e-9;
    let usable = |x: f64| x.is_finite() && x > -1.0;

    let mut x = guess;
    for _ in 0..100 {
        let fx = f(x);
        if !fx.is_finite() {
            break;
        }
        if fx == 0.0 {
            return Some(x);
        }
        // Numerical derivative (central where the domain allows).
        let h = 1e-6 * x.abs().max(1e-2);
        let (lo, hi) = (if x - h > -1.0 { x - h } else { x }, x + h);
        let slope = (f(hi) - f(lo)) / (hi - lo);
        if !slope.is_finite() || slope == 0.0 {
            break;
        }
        let next = x - fx / slope;
        if !usable(next) {
            break;
        }
        if (next - x).abs() <= 1e-12 * next.abs().max(1.0) {
            // The step vanished: accept only where the function really
            // crosses zero (a vanishing step also happens next to the
            // pole at -1, where the slope is enormous).
            let d = 1e-7 * next.abs().max(1.0);
            let (below, above) = (f((next - d).max(LOW)), f(next + d));
            if below.is_finite() && above.is_finite() && (below <= 0.0) != (above <= 0.0) {
                return Some(next);
            }
            break;
        }
        x = next;
    }

    // Scan a grid of rates for a sign change; prefer the bracket nearest
    // the guess, then bisect it.
    const GRID: &[f64] = &[
        LOW, -0.9999, -0.999, -0.99, -0.95, -0.9, -0.8, -0.7, -0.6, -0.5, -0.4, -0.3, -0.2, -0.1,
        -0.05, -0.01, 0.0, 0.01, 0.05, 0.1, 0.2, 0.3, 0.5, 0.75, 1.0, 2.0, 5.0, 10.0, 100.0, 1.0e4,
    ];
    let mut best: Option<(f64, f64, f64)> = None;
    let mut prev: Option<(f64, f64)> = None;
    for &x in GRID {
        let y = f(x);
        if !y.is_finite() {
            prev = None;
            continue;
        }
        if y == 0.0 {
            return Some(x);
        }
        // A sign change between two neighbouring grid points.
        if let Some((px, _)) = prev.filter(|(_, py)| (*py < 0.0) != (y < 0.0)) {
            let distance = if guess < px {
                px - guess
            } else if guess > x {
                guess - x
            } else {
                0.0
            };
            if best.is_none_or(|(_, _, d)| distance < d) {
                best = Some((px, x, distance));
            }
        }
        prev = Some((x, y));
    }
    let (mut lo, mut hi, _) = best?;
    let lo_negative = f(lo) < 0.0;
    for _ in 0..200 {
        let mid = 0.5 * (lo + hi);
        let y = f(mid);
        if !y.is_finite() {
            return None;
        }
        if y == 0.0 {
            return Some(mid);
        }
        if (y < 0.0) == lo_negative {
            lo = mid;
        } else {
            hi = mid;
        }
        if hi - lo <= 1e-14 * hi.abs().max(1.0) {
            break;
        }
    }
    Some(0.5 * (lo + hi))
}

fn f_rate(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let nper = ctx.num(&args[0])?;
    let payment = ctx.num(&args[1])?;
    let pv = ctx.num(&args[2])?;
    let fv = ctx.num_or(args, 3, 0.0)?;
    let due = due_arg(ctx, args, 4)?;
    let guess = ctx.num_or(args, 5, 0.1)?;
    if nper <= 0.0 || guess <= -1.0 {
        return Err(ErrorKind::Num);
    }
    // Future value of all flows at the end of the term.
    let balance = |r: f64| -> f64 {
        if r.abs() < 1e-12 {
            return pv + payment * nper + fv;
        }
        // (1+r)^n - 1, accurate for small r.
        let gm1 = (nper * r.ln_1p()).exp_m1();
        pv * (gm1 + 1.0) + payment * (if due { 1.0 + r } else { 1.0 }) * gm1 / r + fv
    };
    find_rate(&balance, guess)
        .map(Value::Number)
        .ok_or(ErrorKind::Num)
}

// ---- cash-flow series ----------------------------------------------------

fn npv(rate: f64, values: &[f64]) -> f64 {
    let mut total = 0.0;
    let mut discount = 1.0;
    for v in values {
        discount *= 1.0 + rate;
        total += v / discount;
    }
    total
}

fn f_npv(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let rate = ctx.num(&args[0])?;
    let values = ctx.numbers(&args[1..])?;
    if rate == -1.0 {
        return Err(ErrorKind::Div0);
    }
    finite(npv(rate, &values))
}

fn has_both_signs(values: &[f64]) -> bool {
    values.iter().any(|v| *v > 0.0) && values.iter().any(|v| *v < 0.0)
}

fn f_irr(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let values = ctx.numbers(&args[..1])?;
    let guess = ctx.num_or(args, 1, 0.1)?;
    if guess <= -1.0 || !has_both_signs(&values) {
        return Err(ErrorKind::Num);
    }
    // Net present value at time 0 (the first flow is not discounted).
    let f = |r: f64| npv(r, &values) * (1.0 + r);
    find_rate(&f, guess)
        .map(Value::Number)
        .ok_or(ErrorKind::Num)
}

fn f_mirr(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let values = ctx.numbers(&args[..1])?;
    let finance = ctx.num(&args[1])?;
    let reinvest = ctx.num(&args[2])?;
    let n = values.len() as f64;
    let positive: Vec<f64> = values.iter().map(|v| v.max(0.0)).collect();
    let negative: Vec<f64> = values.iter().map(|v| v.min(0.0)).collect();
    if !has_both_signs(&values) || finance == -1.0 || reinvest == -1.0 {
        return Err(ErrorKind::Div0);
    }
    let gains = npv(reinvest, &positive) * growth(reinvest, n);
    let costs = npv(finance, &negative) * (1.0 + finance);
    finite((-gains / costs).powf(1.0 / (n - 1.0)) - 1.0)
}

/// Numbers of a range or array for `XNPV` / `XIRR`: every element must be
/// numeric (blank cells count as 0).
fn strict_numbers(ctx: &EvalCtx, e: &Expr) -> R<Vec<f64>> {
    let mut out = Vec::new();
    for v in ctx.flatten(std::slice::from_ref(e)) {
        out.push(match v {
            Value::Number(n) => n,
            Value::Empty => 0.0,
            Value::Error(err) => return Err(err),
            Value::Text(_) => ctx.to_number(&v)?,
            Value::Bool(_) | Value::Array(_) => return Err(ErrorKind::Value),
        });
    }
    Ok(out)
}

/// Cash flows paired with their offset in years (365-day) from the first
/// date.
fn dated_flows(ctx: &EvalCtx, values: &Expr, dates: &Expr) -> R<Vec<(f64, f64)>> {
    let values = strict_numbers(ctx, values)?;
    let dates = strict_numbers(ctx, dates)?;
    if values.len() != dates.len() || values.is_empty() {
        return Err(ErrorKind::Num);
    }
    let first = dates[0].floor();
    let mut out = Vec::with_capacity(values.len());
    for (v, d) in values.iter().zip(&dates) {
        let d = d.floor();
        if !(0.0..=2_958_465.0).contains(&d) || d < first {
            return Err(ErrorKind::Num);
        }
        out.push((*v, (d - first) / 365.0));
    }
    Ok(out)
}

fn xnpv(rate: f64, flows: &[(f64, f64)]) -> f64 {
    flows.iter().map(|(v, t)| v / (1.0 + rate).powf(*t)).sum()
}

fn f_xnpv(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let rate = ctx.num(&args[0])?;
    let flows = dated_flows(ctx, &args[1], &args[2])?;
    if rate <= -1.0 {
        return Err(ErrorKind::Num);
    }
    finite(xnpv(rate, &flows))
}

fn f_xirr(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let flows = dated_flows(ctx, &args[0], &args[1])?;
    let guess = ctx.num_or(args, 2, 0.1)?;
    let values: Vec<f64> = flows.iter().map(|(v, _)| *v).collect();
    if guess <= -1.0 || !has_both_signs(&values) {
        return Err(ErrorKind::Num);
    }
    let f = |r: f64| xnpv(r, &flows);
    find_rate(&f, guess)
        .map(Value::Number)
        .ok_or(ErrorKind::Num)
}

// ---- depreciation --------------------------------------------------------

fn f_sln(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let cost = ctx.num(&args[0])?;
    let salvage = ctx.num(&args[1])?;
    let life = ctx.num(&args[2])?;
    if life == 0.0 {
        return Err(ErrorKind::Div0);
    }
    finite((cost - salvage) / life)
}

fn f_syd(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let cost = ctx.num(&args[0])?;
    let salvage = ctx.num(&args[1])?;
    let life = ctx.num(&args[2])?;
    let per = ctx.num(&args[3])?;
    if salvage < 0.0 || life <= 0.0 || per < 1.0 || per > life {
        return Err(ErrorKind::Num);
    }
    finite((cost - salvage) * (life - per + 1.0) * 2.0 / (life * (life + 1.0)))
}

/// Fixed-declining balance. The rate is rounded to three decimals; the
/// first year covers `month` months and an extra final period the rest.
fn f_db(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let cost = ctx.num(&args[0])?;
    let salvage = ctx.num(&args[1])?;
    let life = ctx.num(&args[2])?;
    let period = ctx.num(&args[3])?;
    let month = ctx.num_or(args, 4, 12.0)?.trunc();
    if cost < 0.0 || salvage < 0.0 || life <= 0.0 || period < 1.0 || !(1.0..=12.0).contains(&month)
    {
        return Err(ErrorKind::Num);
    }
    let last = if month < 12.0 { life + 1.0 } else { life };
    if period > last {
        return Err(ErrorKind::Num);
    }
    if cost == 0.0 {
        return Ok(Value::Number(0.0));
    }
    let rate = ((1.0 - (salvage / cost).powf(1.0 / life)) * 1000.0).round() / 1000.0;
    let period = period.floor();
    let first = cost * rate * month / 12.0;
    if period == 1.0 {
        return finite(first);
    }
    let mut total = first;
    let mut depreciation = first;
    let mut p = 2.0;
    while p <= period.min(life) {
        depreciation = (cost - total) * rate;
        total += depreciation;
        p += 1.0;
    }
    if period > life {
        depreciation = (cost - total) * rate * (12.0 - month) / 12.0;
    }
    finite(depreciation)
}

/// Double-declining balance (or another `factor`), never depreciating
/// below the salvage value.
fn f_ddb(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let cost = ctx.num(&args[0])?;
    let salvage = ctx.num(&args[1])?;
    let life = ctx.num(&args[2])?;
    let period = ctx.num(&args[3])?;
    let factor = ctx.num_or(args, 4, 2.0)?;
    if cost < 0.0 || salvage < 0.0 || life <= 0.0 || period < 1.0 || period > life || factor <= 0.0
    {
        return Err(ErrorKind::Num);
    }
    let rate = factor / life;
    let (before, after) = if rate >= 1.0 {
        // Everything is written off in the first period.
        (if period == 1.0 { cost } else { 0.0 }, 0.0)
    } else {
        (
            cost * (1.0 - rate).powf(period - 1.0),
            cost * (1.0 - rate).powf(period),
        )
    };
    finite((before - after.max(salvage)).max(0.0))
}

// ---- interest rate conversion --------------------------------------------

fn f_effect(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let nominal = ctx.num(&args[0])?;
    let npery = ctx.num(&args[1])?.trunc();
    if nominal <= 0.0 || npery < 1.0 {
        return Err(ErrorKind::Num);
    }
    finite((1.0 + nominal / npery).powf(npery) - 1.0)
}

fn f_nominal(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let effect = ctx.num(&args[0])?;
    let npery = ctx.num(&args[1])?.trunc();
    if effect <= 0.0 || npery < 1.0 {
        return Err(ErrorKind::Num);
    }
    finite(((1.0 + effect).powf(1.0 / npery) - 1.0) * npery)
}

#[rustfmt::skip]
pub const FUNCTIONS: &[FunctionDef] = &[
    FunctionDef::new("PMT", 3, 5, Kind::Scalar, Category::Financial, f_pmt),
    FunctionDef::new("PV", 3, 5, Kind::Scalar, Category::Financial, f_pv),
    FunctionDef::new("FV", 3, 5, Kind::Scalar, Category::Financial, f_fv),
    FunctionDef::new("NPV", 2, VAR, Kind::Raw, Category::Financial, f_npv),
    FunctionDef::new("IRR", 1, 2, Kind::Raw, Category::Financial, f_irr),
    FunctionDef::new("XNPV", 3, 3, Kind::Raw, Category::Financial, f_xnpv),
    FunctionDef::new("XIRR", 2, 3, Kind::Raw, Category::Financial, f_xirr),
    FunctionDef::new("MIRR", 3, 3, Kind::Raw, Category::Financial, f_mirr),
    FunctionDef::new("RATE", 3, 6, Kind::Scalar, Category::Financial, f_rate),
    FunctionDef::new("NPER", 3, 5, Kind::Scalar, Category::Financial, f_nper),
    FunctionDef::new("IPMT", 4, 6, Kind::Scalar, Category::Financial, f_ipmt),
    FunctionDef::new("PPMT", 4, 6, Kind::Scalar, Category::Financial, f_ppmt),
    FunctionDef::new("CUMIPMT", 6, 6, Kind::Scalar, Category::Financial, f_cumipmt),
    FunctionDef::new("CUMPRINC", 6, 6, Kind::Scalar, Category::Financial, f_cumprinc),
    FunctionDef::new("SLN", 3, 3, Kind::Scalar, Category::Financial, f_sln),
    FunctionDef::new("DB", 4, 5, Kind::Scalar, Category::Financial, f_db),
    FunctionDef::new("DDB", 4, 5, Kind::Scalar, Category::Financial, f_ddb),
    FunctionDef::new("SYD", 4, 4, Kind::Scalar, Category::Financial, f_syd),
    FunctionDef::new("EFFECT", 2, 2, Kind::Scalar, Category::Financial, f_effect),
    FunctionDef::new("NOMINAL", 2, 2, Kind::Scalar, Category::Financial, f_nominal),
];

#[cfg(test)]
mod tests {
    use crate::tests::{eval, num, val, wb};
    use crate::value::{ErrorKind, Value};

    fn err(formula: &str) -> ErrorKind {
        match eval(formula) {
            Value::Error(e) => e,
            other => panic!("{formula} gave {other:?}, expected an error"),
        }
    }

    /// Compare with a relative tolerance (absolute near zero).
    fn close(formula: &str, expected: f64) {
        let got = num(formula);
        let tol = 1e-6 * expected.abs().max(1e-3);
        assert!(
            (got - expected).abs() <= tol,
            "{formula} gave {got}, expected {expected}"
        );
    }

    fn cell_close(w: &crate::workbook::Workbook, cell: &str, expected: f64) {
        match val(w, cell) {
            Value::Number(got) => {
                let tol = 1e-6 * expected.abs().max(1e-3);
                assert!(
                    (got - expected).abs() <= tol,
                    "{cell} gave {got}, expected {expected}"
                );
            }
            other => panic!("{cell} gave {other:?}, expected {expected}"),
        }
    }

    #[test]
    fn pmt() {
        close("PMT(0.05/12,60,10000)", -188.712336);
        close("PMT(8%/12,10,10000)", -1037.032089);
        close("PMT(8%/12,10,10000,0,1)", -1030.164327);
        close("PMT(6%/12,18*12,0,50000)", -129.081161);
        close("PMT(0,10,1000)", -100.0);
        close("PMT(0,10,1000,500)", -150.0);
        // Any non-zero type means "beginning of period".
        close("PMT(8%/12,10,10000,0,2)", -1030.164327);
        close("PMT(8%/12,10,10000,,)", -1037.032089);
        assert_eq!(err("PMT(0.05,0,1000)"), ErrorKind::Num);
        assert_eq!(err("PMT(0,0,1000)"), ErrorKind::Num);
        assert_eq!(err("PMT(\"x\",10,1000)"), ErrorKind::Value);
        assert_eq!(err("PMT(1/0,10,1000)"), ErrorKind::Div0);
    }

    #[test]
    fn pv_and_fv() {
        close("PV(0.08/12,12*20,500,,0)", -59777.145851);
        close("PV(0.05,10,-100)", 772.173493);
        close("PV(0.05,10,-100,0,1)", 810.782168);
        close("PV(0,10,-100)", 1000.0);
        close("PV(0.1,5,0,1000)", -620.921323);
        close("FV(0.06/12,10,-200,-500,1)", 2581.403374);
        close("FV(0.12/12,12,-1000)", 12682.503013);
        close("FV(0.11/12,35,-2000,,1)", 82846.246372);
        close("FV(0.06/12,12,-100,-1000,1)", 2301.401830);
        close("FV(0,12,-100,-1000)", 2200.0);
        // PV and FV are inverse to each other.
        close("FV(0.07,9,-150,PV(0.07,9,-150,2500))", 2500.0);
        assert_eq!(err("FV(\"x\",1,1)"), ErrorKind::Value);
    }

    #[test]
    fn nper_and_rate() {
        close("NPER(0.12/12,-100,-1000,10000,1)", 59.673866);
        close("NPER(0.12/12,-100,-1000,10000)", 60.082123);
        close("NPER(0.12/12,-100,-1000)", -9.578594);
        close("NPER(0,-100,1000)", 10.0);
        assert_eq!(err("NPER(0,0,1000)"), ErrorKind::Div0);
        // The payment never covers the interest.
        assert_eq!(err("NPER(0.1,-50,1000)"), ErrorKind::Num);
        close("RATE(48,-200,8000)", 0.007701472);
        close("RATE(48,-200,8000)*12", 0.092417669);
        close("RATE(10,-1000,6500)", 0.087113756);
        close("RATE(10,0,-1000,2000)", 0.071773463);
        close("RATE(12,-100,1200)", 0.0);
        close("RATE(60,-188.712336443,10000)", 0.05 / 12.0);
        close("RATE(10,-1030.164327,10000,0,1)", 0.08 / 12.0);
        close("RATE(48,-200,8000,0,0,0.5)", 0.007701472);
        assert_eq!(err("RATE(0,-200,8000)"), ErrorKind::Num);
        // All flows have the same sign: no rate balances them.
        assert_eq!(err("RATE(10,100,1000)"), ErrorKind::Num);
    }

    #[test]
    fn ipmt_and_ppmt() {
        close("IPMT(0.1/12,1,36,8000)", -66.666667);
        close("IPMT(0.1,3,3,8000)", -292.447130);
        close("PPMT(0.1/12,1,24,2000)", -75.623186);
        close("PPMT(0.08,10,10,200000)", -27598.053462);
        // Interest and principal add up to the payment.
        close(
            "IPMT(0.07/12,5,48,15000)+PPMT(0.07/12,5,48,15000)",
            num("PMT(0.07/12,48,15000)"),
        );
        close(
            "IPMT(0.07/12,5,48,15000,0,1)+PPMT(0.07/12,5,48,15000,0,1)",
            num("PMT(0.07/12,48,15000,0,1)"),
        );
        // Payments at the beginning: no interest in the first period.
        assert_eq!(num("IPMT(0.1/12,1,36,8000,0,1)"), 0.0);
        close("IPMT(0.1/12,2,36,8000,0,1)", -64.533299);
        close("IPMT(0,3,10,1000)", 0.0);
        close("PPMT(0,3,10,1000)", -100.0);
        assert_eq!(err("IPMT(0.1,0,3,8000)"), ErrorKind::Num);
        assert_eq!(err("IPMT(0.1,4,3,8000)"), ErrorKind::Num);
        assert_eq!(err("PPMT(0.1,0.5,3,8000)"), ErrorKind::Num);
    }

    #[test]
    fn cumulative() {
        close("CUMIPMT(0.09/12,30*12,125000,13,24,0)", -11135.232131);
        close("CUMIPMT(0.09/12,30*12,125000,1,1,0)", -937.5);
        close("CUMPRINC(0.09/12,30*12,125000,13,24,0)", -934.107123);
        close("CUMPRINC(0.09/12,30*12,125000,1,1,0)", -68.278271);
        // The whole loan is repaid over its life.
        close("CUMPRINC(0.09/12,360,125000,1,360,0)", -125000.0);
        close("CUMPRINC(0.09/12,360,125000,1,360,1)", -125000.0);
        close(
            "CUMIPMT(0.09/12,360,125000,1,360,1)+CUMPRINC(0.09/12,360,125000,1,360,1)",
            num("PMT(0.09/12,360,125000,0,1)*360"),
        );
        assert_eq!(err("CUMIPMT(0,360,125000,1,12,0)"), ErrorKind::Num);
        assert_eq!(err("CUMIPMT(0.01,360,125000,0,12,0)"), ErrorKind::Num);
        assert_eq!(err("CUMIPMT(0.01,360,125000,13,12,0)"), ErrorKind::Num);
        assert_eq!(err("CUMIPMT(0.01,360,125000,1,361,0)"), ErrorKind::Num);
        assert_eq!(err("CUMPRINC(0.01,360,125000,1,12,2)"), ErrorKind::Num);
        assert_eq!(err("CUMPRINC(0.01,360,-5,1,12,0)"), ErrorKind::Num);
    }

    #[test]
    fn npv() {
        close("NPV(0.1,-10000,3000,4200,6800)", 1188.443412);
        close("NPV(0.08,8000,9200,10000,12000,14500)-40000", 1922.061555);
        close("NPV(0,1,2,3)", 6.0);
        close("NPV(10%,{100,200},300)", 481.592787);
        assert_eq!(err("NPV(-1,100)"), ErrorKind::Div0);
        assert_eq!(err("NPV(0.1,\"x\")"), ErrorKind::Value);
        let w = wb(&[
            ("A1", "-10000"),
            ("A2", "3000"),
            ("A3", "note"),
            ("A4", "4200"),
            ("A6", "6800"),
            ("B1", "=NPV(0.1,A1:A6)"),
        ]);
        // Text and blanks inside the range are skipped, not counted as 0.
        cell_close(&w, "B1", 1188.443412);
    }

    #[test]
    fn irr() {
        let w = wb(&[
            ("A1", "-70000"),
            ("A2", "12000"),
            ("A3", "15000"),
            ("A4", "18000"),
            ("A5", "21000"),
            ("A6", "26000"),
            ("B1", "=IRR(A1:A6)"),
            ("B2", "=IRR(A1:A5)"),
            ("B3", "=IRR(A1:A3,-10%)"),
            ("B4", "=IRR(A2:A6)"),
            ("B5", "=NPV(B1,A1:A6)"),
            ("B6", "=MIRR(A1:A6,10%,12%)"),
        ]);
        cell_close(&w, "B1", 0.086630948);
        cell_close(&w, "B2", -0.021244848);
        cell_close(&w, "B3", -0.443506941);
        assert_eq!(val(&w, "B4"), Value::Error(ErrorKind::Num));
        cell_close(&w, "B5", 0.0);
        close("IRR({-100,110})", 0.1);
        close("IRR({-100,50,50,50})", 0.233751928);
        close("IRR({-1000,100,100,100})", -0.424417444);
        // Far-off guesses still converge.
        close("IRR({-100,110},5)", 0.1);
        close("IRR({-100,110},-0.9)", 0.1);
        close("IRR({-100,0,0,0,0,400})", 0.319507911);
        close("IRR({100,-110})", 0.1);
        assert_eq!(err("IRR({1,2,3})"), ErrorKind::Num);
        assert_eq!(err("IRR(5)"), ErrorKind::Num);
        assert_eq!(err("IRR({-100,#N/A,50})"), ErrorKind::NA);
    }

    #[test]
    fn solver_robustness() {
        // Roots far from the default guess.
        close("IRR({-1,100})", 99.0);
        close("IRR({-1000,1})", -0.999);
        close(
            "IRR({-1000,200,200,200,200,200,200,200,200,200,200})",
            0.150984145,
        );
        // Two sign changes: the root nearest the guess is found.
        close("IRR({-100,230,-132},0.05)", 0.1);
        close("IRR({-100,230,-132},0.25)", 0.2);
        close("RATE(12,-50,1000)", -0.071001947);
        close("RATE(12,-100,1000)", 0.029228541);
        close("RATE(24,-50,0,2000,1)", 0.038837802);
        close("RATE(360,PMT(0.005,360,100000),100000)", 0.005);
        assert_eq!(err("IRR({-100,110},-1)"), ErrorKind::Num);
        assert_eq!(err("RATE(12,-100,1000,0,0,-2)"), ErrorKind::Num);
    }

    #[test]
    fn mirr() {
        close(
            "MIRR({-120000,39000,30000,21000,37000,46000},10%,12%)",
            0.126094130,
        );
        close("MIRR({-120000,39000,30000,21000},10%,12%)", -0.048044655);
        close(
            "MIRR({-120000,39000,30000,21000,37000,46000},10%,14%)",
            0.134759111,
        );
        assert_eq!(err("MIRR({1,2,3},10%,12%)"), ErrorKind::Div0);
        assert_eq!(err("MIRR({-1,-2},10%,12%)"), ErrorKind::Div0);
    }

    #[test]
    fn xnpv_and_xirr() {
        let w = wb(&[
            ("A1", "-10000"),
            ("A2", "2750"),
            ("A3", "4250"),
            ("A4", "3250"),
            ("A5", "2750"),
            ("B1", "2008-01-01"),
            ("B2", "2008-03-01"),
            ("B3", "2008-10-30"),
            ("B4", "2009-02-15"),
            ("B5", "2009-04-01"),
            ("C1", "=XNPV(0.09,A1:A5,B1:B5)"),
            ("C2", "=XIRR(A1:A5,B1:B5)"),
            ("C3", "=XIRR(A1:A5,B1:B5,0.1)"),
            ("C4", "=XNPV(C2,A1:A5,B1:B5)"),
            ("C5", "=XNPV(0.09,A1:A5,B1:B4)"),
            ("C6", "=XIRR(A2:A5,B2:B5)"),
            ("C7", "=XIRR(A1:A5,B1:B5,-0.5)"),
            ("C8", "=XNPV(0,A1:A5,B1:B5)"),
            ("D1", "2010-01-01"),
            ("D2", "2009-01-01"),
            ("C9", "=XNPV(0.09,A1:A2,D1:D2)"),
            ("C10", "=XNPV(-1,A1:A5,B1:B5)"),
        ]);
        cell_close(&w, "C1", 2086.647602);
        cell_close(&w, "C2", 0.373362535);
        cell_close(&w, "C3", 0.373362535);
        cell_close(&w, "C4", 0.0);
        assert_eq!(val(&w, "C5"), Value::Error(ErrorKind::Num));
        assert_eq!(val(&w, "C6"), Value::Error(ErrorKind::Num));
        cell_close(&w, "C7", 0.373362535);
        cell_close(&w, "C8", 3000.0);
        // A date before the first one is rejected.
        assert_eq!(val(&w, "C9"), Value::Error(ErrorKind::Num));
        assert_eq!(val(&w, "C10"), Value::Error(ErrorKind::Num));
        // One year apart exactly: the plain rate of return.
        close("XIRR({-1000,1100},{43831,44196})", 0.1);
        close("XNPV(0.1,{-1000,1100},{43831,44196})", 0.0);
        assert_eq!(err("XIRR({-1000,1100},{43831,\"x\"})"), ErrorKind::Value);
    }

    #[test]
    fn depreciation() {
        close("SLN(30000,7500,10)", 2250.0);
        assert_eq!(err("SLN(30000,7500,0)"), ErrorKind::Div0);
        close("SYD(30000,7500,10,1)", 4090.909091);
        close("SYD(30000,7500,10,10)", 409.090909);
        assert_eq!(err("SYD(30000,7500,10,11)"), ErrorKind::Num);
        assert_eq!(err("SYD(30000,7500,10,0)"), ErrorKind::Num);
        close("DB(1000000,100000,6,1,7)", 186083.333333);
        close("DB(1000000,100000,6,2,7)", 259639.416667);
        close("DB(1000000,100000,6,3,7)", 176814.442750);
        close("DB(1000000,100000,6,4,7)", 120410.635513);
        close("DB(1000000,100000,6,5,7)", 81999.642784);
        close("DB(1000000,100000,6,6,7)", 55841.756736);
        close("DB(1000000,100000,6,7,7)", 15845.098474);
        close("DB(10000,1000,5,1)", 3690.0);
        close("DB(10000,1000,5,5)", 584.983751);
        assert_eq!(err("DB(1000000,100000,6,8,7)"), ErrorKind::Num);
        assert_eq!(err("DB(10000,1000,5,6)"), ErrorKind::Num);
        assert_eq!(err("DB(10000,1000,5,1,13)"), ErrorKind::Num);
        assert_eq!(err("DB(10000,1000,5,0)"), ErrorKind::Num);
        close("DDB(2400,300,10*365,1)", 1.315068493);
        close("DDB(2400,300,10*12,1,2)", 40.0);
        close("DDB(2400,300,10,1,2)", 480.0);
        close("DDB(2400,300,10,2,1.5)", 306.0);
        close("DDB(2400,300,10,10)", 22.122547);
        // Never below the salvage value.
        close("DDB(1000,900,5,1)", 100.0);
        close("DDB(1000,900,5,2)", 0.0);
        assert_eq!(err("DDB(2400,300,10,11)"), ErrorKind::Num);
        assert_eq!(err("DDB(2400,300,10,0)"), ErrorKind::Num);
        assert_eq!(err("DDB(2400,-1,10,1)"), ErrorKind::Num);
    }

    #[test]
    fn effect_and_nominal() {
        close("EFFECT(0.0525,4)", 0.053542667);
        close("EFFECT(0.1,12)", 0.104713067);
        close("EFFECT(0.1,1)", 0.1);
        close("EFFECT(0.0525,4.9)", 0.053542667);
        close("NOMINAL(0.053543,4)", 0.052500319);
        close("NOMINAL(EFFECT(0.07,12),12)", 0.07);
        assert_eq!(err("EFFECT(0,4)"), ErrorKind::Num);
        assert_eq!(err("EFFECT(0.05,0.5)"), ErrorKind::Num);
        assert_eq!(err("NOMINAL(-0.05,4)"), ErrorKind::Num);
        assert_eq!(err("NOMINAL(0.05,0)"), ErrorKind::Num);
        assert_eq!(err("EFFECT(\"x\",4)"), ErrorKind::Value);
    }

    #[test]
    fn lifted_over_ranges() {
        let w = wb(&[("A1", "0.05"), ("A2", "0.1"), ("C1", "=FV(A1:A2,1,0,-100)")]);
        cell_close(&w, "C1", 105.0);
        cell_close(&w, "C2", 110.0);
    }
}
