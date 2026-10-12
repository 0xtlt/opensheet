//! Math & trigonometry functions.

use super::criteria::{Grid, if_function, ifs_function, lift, matrix};
use super::stat;
use super::{Category, FunctionDef, Kind, VAR};
use crate::ast::Expr;
use crate::eval::{EvalCtx, is_reference};
use crate::refs::{MAX_COLS, MAX_ROWS};
use crate::value::{Array, ErrorKind, R, Value};
use std::cell::Cell;
use std::f64::consts::PI;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};

/// 2^53: integers up to here are exact in an `f64`.
const MAX_EXACT: f64 = 9_007_199_254_740_992.0;

/// Largest array a function may generate (`SEQUENCE`, `RANDARRAY`, ...).
const MAX_GENERATED_CELLS: f64 = 10_000_000.0;

// ---- shared numeric helpers ------------------------------------------------

/// Wrap a float result: non-finite values become `#NUM!`, `-0` becomes `0`.
pub(super) fn number(x: f64) -> R<Value> {
    if x.is_finite() {
        Ok(Value::Number(x + 0.0))
    } else {
        Err(ErrorKind::Num)
    }
}

/// Round to 15 significant decimal digits, the precision spreadsheets work
/// at. Removes binary noise such as `0.7/0.1 = 6.999999999999999`.
pub(super) fn snap15(x: f64) -> f64 {
    // Whole numbers are exact already (and may have more than 15 digits).
    if x.fract() == 0.0 || !x.is_finite() {
        return x;
    }
    format!("{x:.14e}").parse().unwrap_or(x)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Rounding {
    /// Nearest, halves away from zero (`ROUND`).
    HalfAway,
    /// Away from zero (`ROUNDUP`).
    Up,
    /// Toward zero (`ROUNDDOWN`, `TRUNC`).
    Down,
}

/// Round `x` to `digits` decimal places (negative: left of the decimal
/// point). The rounding is done on the 15-significant-digit decimal form
/// of the number, so values such as 2.675 or 1.005, which are stored a
/// hair below their decimal spelling, round the way they read. Whole
/// numbers are rounded on their exact digits.
pub(super) fn round_to(x: f64, digits: i32, mode: Rounding) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    let digits = digits.clamp(-400, 400) as i64;
    let whole = x.fract() == 0.0;
    let text = format!("{:.*e}", if whole { 16 } else { 14 }, x.abs());
    let Some((mantissa, exp)) = text.split_once('e') else {
        return x;
    };
    let Ok(exp) = exp.parse::<i64>() else {
        return x;
    };
    let mut digs: Vec<u8> = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .map(|b| b - b'0')
        .collect();
    // digs[i] has place value 10^(exp - i); keep those >= 10^(-digits).
    let keep = exp + digits + 1;
    if keep >= digs.len() as i64 {
        // Nothing to drop; fractions still lose their binary noise.
        return snap15(x);
    }
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    if keep <= 0 {
        // Everything is dropped: the result is 0 or one unit.
        let up = match mode {
            Rounding::HalfAway => keep == 0 && digs[0] >= 5,
            Rounding::Up => true,
            Rounding::Down => false,
        };
        let unit: f64 = format!("1e{}", -digits).parse().unwrap_or(f64::INFINITY);
        return if up { sign * unit } else { 0.0 };
    }
    let keep = keep as usize;
    let up = match mode {
        Rounding::HalfAway => digs[keep] >= 5,
        Rounding::Up => digs[keep..].iter().any(|d| *d != 0),
        Rounding::Down => false,
    };
    digs.truncate(keep);
    if up {
        let mut i = keep;
        loop {
            if i == 0 {
                digs.insert(0, 1);
                break;
            }
            i -= 1;
            if digs[i] == 9 {
                digs[i] = 0;
            } else {
                digs[i] += 1;
                break;
            }
        }
    }
    // The kept digits form an integer scaled by 10^(exp - keep + 1).
    let mut out: String = digs.iter().map(|d| (b'0' + d) as char).collect();
    out.push_str(&format!("e{}", exp - keep as i64 + 1));
    sign * out.parse::<f64>().unwrap_or(x.abs())
}

/// `k * m` for an integer `k`, multiplied in decimal so that `7 * 0.1` is
/// `0.7` rather than `0.7000000000000001`.
fn mul_exact(k: f64, m: f64) -> f64 {
    let plain = k * m;
    if k == 0.0 || m == 0.0 || !plain.is_finite() || k.abs() >= MAX_EXACT || k.fract() != 0.0 {
        return plain;
    }
    // Shortest decimal form of `m`: "d.ddde-N".
    let text = format!("{:e}", m.abs());
    let Some((mantissa, exp)) = text.split_once('e') else {
        return plain;
    };
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let frac_len = mantissa.split_once('.').map_or(0, |(_, f)| f.len()) as i64;
    let (Ok(mantissa), Ok(exp)) = (digits.parse::<u128>(), exp.parse::<i64>()) else {
        return plain;
    };
    let product = mantissa * k.abs() as u128;
    let sign = if (k < 0.0) != (m < 0.0) { -1.0 } else { 1.0 };
    format!("{product}e{}", exp - frac_len)
        .parse::<f64>()
        .map_or(plain, |v| sign * v)
}

/// `x` moved to a multiple of `m` (non-zero) by applying `f` to `x / m`.
fn to_multiple(x: f64, m: f64, f: fn(f64) -> f64) -> f64 {
    mul_exact(f(snap15(x / m)), m)
}

fn round_half_away(x: f64) -> f64 {
    (x.abs() + 0.5).floor().copysign(x)
}

/// Binomial coefficient; exact while the result fits in 2^53.
pub(super) fn combin(n: f64, k: f64) -> f64 {
    if k < 0.0 || k > n {
        return 0.0;
    }
    let k = k.min(n - k);
    let mut r = 1.0f64;
    let mut i = 1.0;
    while i <= k {
        r = r * (n - k + i) / i;
        if !r.is_finite() {
            return f64::INFINITY;
        }
        i += 1.0;
    }
    if r < MAX_EXACT { r.round() } else { r }
}

/// `n!` for a non-negative integer `n` (infinite above 170).
pub(super) fn factorial(n: f64) -> f64 {
    if n > 170.0 {
        return f64::INFINITY;
    }
    let mut r = 1.0;
    let mut i = 2.0;
    while i <= n {
        r *= i;
        i += 1.0;
    }
    r
}

fn gcd(mut a: f64, mut b: f64) -> f64 {
    while b != 0.0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Arguments of the functions inherited from the Analysis ToolPak (`GCD`,
/// `LCM`, `MULTINOMIAL`): numbers and numeric text typed as arguments;
/// booleans and other text are `#VALUE!`, blank cells are skipped. Values
/// are truncated to integers and must lie in `0 .. 2^53`.
fn whole_numbers(ctx: &EvalCtx, args: &[Expr]) -> R<Vec<f64>> {
    let mut out = Vec::new();
    for arg in args {
        match ctx.eval(arg) {
            Value::Array(a) => {
                for v in a.iter() {
                    match v {
                        Value::Number(n) => out.push(*n),
                        Value::Empty => {}
                        Value::Error(e) => return Err(*e),
                        _ => return Err(ErrorKind::Value),
                    }
                }
            }
            Value::Empty => {
                if ctx.is_missing(arg) {
                    out.push(0.0);
                }
            }
            Value::Number(n) => out.push(n),
            Value::Error(e) => return Err(e),
            Value::Text(s) if !is_reference(arg) => out.push(ctx.to_number(&Value::Text(s))?),
            _ => return Err(ErrorKind::Value),
        }
    }
    for x in out.iter_mut() {
        *x = x.trunc();
        if *x < 0.0 || *x >= MAX_EXACT {
            return Err(ErrorKind::Num);
        }
    }
    Ok(out)
}

/// Validate the size of a generated array.
fn array_dims(rows: f64, cols: f64) -> R<(usize, usize)> {
    let (rows, cols) = (rows.trunc(), cols.trunc());
    if rows < 0.0 || cols < 0.0 {
        return Err(ErrorKind::Value);
    }
    if rows == 0.0 || cols == 0.0 {
        return Err(ErrorKind::Calc);
    }
    if rows > MAX_ROWS as f64 || cols > MAX_COLS as f64 || rows * cols > MAX_GENERATED_CELLS {
        return Err(ErrorKind::Value);
    }
    Ok((rows as usize, cols as usize))
}

fn numeric_array(rows: usize, cols: usize, data: Vec<f64>) -> R<Value> {
    let data = data.into_iter().map(number).collect::<R<Vec<Value>>>()?;
    Ok(Value::array(Array { rows, cols, data }))
}

// ---- sums and products ------------------------------------------------------

fn f_sum(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    number(ctx.numbers(args)?.iter().sum())
}

fn f_sumif(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    if_function(ctx, args, |nums| number(nums.iter().sum()))
}

fn f_sumifs(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    ifs_function(ctx, args, |nums| number(nums.iter().sum()))
}

fn f_sumproduct(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let grids = args
        .iter()
        .map(|a| Grid::eval(ctx, a))
        .collect::<R<Vec<Grid>>>()?;
    let first = &grids[0];
    if grids.iter().any(|g| !g.same_shape(first)) {
        return Err(ErrorKind::Value);
    }
    if let Some(e) = grids.iter().find_map(Grid::first_error) {
        return Err(e);
    }
    // Beyond the materialised part of any array the product is 0.
    let rows = grids.iter().map(|g| g.values.rows).min().unwrap_or(0);
    let cols = grids.iter().map(|g| g.values.cols).min().unwrap_or(0);
    let mut sum = 0.0;
    for r in 0..rows {
        for c in 0..cols {
            let mut product = 1.0;
            for g in &grids {
                match g.get(r, c) {
                    Value::Number(n) => product *= n,
                    // Text, booleans and blanks count as 0.
                    _ => {
                        product = 0.0;
                        break;
                    }
                }
            }
            sum += product;
        }
    }
    number(sum)
}

fn f_sumsq(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    number(ctx.numbers(args)?.iter().map(|x| x * x).sum())
}

fn sum_pairs(ctx: &EvalCtx, args: &[Expr], f: fn(f64, f64) -> f64) -> R<Value> {
    let pairs = stat::pairs(ctx, &args[0], &args[1])?;
    number(pairs.iter().map(|(x, y)| f(*x, *y)).sum())
}

fn f_sumx2my2(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    sum_pairs(ctx, args, |x, y| x * x - y * y)
}

fn f_sumx2py2(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    sum_pairs(ctx, args, |x, y| x * x + y * y)
}

fn f_sumxmy2(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    sum_pairs(ctx, args, |x, y| (x - y) * (x - y))
}

fn f_product(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let nums = ctx.numbers(args)?;
    number(if nums.is_empty() { 0.0 } else { nums.iter().product() })
}

fn f_seriessum(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let x = ctx.num(&args[0])?;
    let n = ctx.num(&args[1])?;
    let m = ctx.num(&args[2])?;
    let coefficients = Grid::eval(ctx, &args[3])?;
    let mut sum = 0.0;
    for (i, v) in coefficients.values.iter().enumerate() {
        let a = match v {
            Value::Number(a) => *a,
            Value::Empty => continue,
            Value::Error(e) => return Err(*e),
            _ => return Err(ErrorKind::Value),
        };
        sum += a * x.powf(n + i as f64 * m);
    }
    number(sum)
}

// ---- simple scalar functions --------------------------------------------------

/// Define one-argument numeric functions: `name = |x| -> R<f64>`.
macro_rules! unary {
    ($($name:ident = |$x:ident| $body:expr;)*) => {
        $(
            fn $name(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
                let $x = ctx.num(&args[0])?;
                let out: R<f64> = $body;
                number(out?)
            }
        )*
    };
}

fn domain(ok: bool, value: f64) -> R<f64> {
    if ok { Ok(value) } else { Err(ErrorKind::Num) }
}

/// Largest argument accepted by `COT`, `SEC` and `CSC`.
const TRIG_LIMIT: f64 = 134_217_728.0; // 2^27

unary! {
    f_abs = |x| Ok(x.abs());
    f_sign = |x| Ok(if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { 0.0 });
    f_sqrt = |x| domain(x >= 0.0, x.sqrt());
    f_sqrtpi = |x| domain(x >= 0.0, (x * PI).sqrt());
    f_exp = |x| Ok(x.exp());
    f_ln = |x| domain(x > 0.0, x.ln());
    f_log10 = |x| domain(x > 0.0, x.log10());
    f_int = |x| Ok(x.floor());
    f_sin = |x| Ok(x.sin());
    f_cos = |x| Ok(x.cos());
    f_tan = |x| Ok(x.tan());
    f_asin = |x| domain((-1.0..=1.0).contains(&x), x.asin());
    f_acos = |x| domain((-1.0..=1.0).contains(&x), x.acos());
    f_atan = |x| Ok(x.atan());
    f_sinh = |x| Ok(x.sinh());
    f_cosh = |x| Ok(x.cosh());
    f_tanh = |x| Ok(x.tanh());
    f_asinh = |x| Ok(x.asinh());
    f_acosh = |x| domain(x >= 1.0, x.acosh());
    f_atanh = |x| domain(x > -1.0 && x < 1.0, x.atanh());
    f_cot = |x| if x == 0.0 { Err(ErrorKind::Div0) } else { domain(x.abs() < TRIG_LIMIT, x.cos() / x.sin()) };
    f_csc = |x| if x == 0.0 { Err(ErrorKind::Div0) } else { domain(x.abs() < TRIG_LIMIT, 1.0 / x.sin()) };
    f_sec = |x| domain(x.abs() < TRIG_LIMIT, 1.0 / x.cos());
    f_acot = |x| Ok(PI / 2.0 - x.atan());
    f_degrees = |x| Ok(x.to_degrees());
    f_radians = |x| Ok(x.to_radians());
    f_even = |x| Ok({
        let a = x.abs().ceil();
        let a = if a % 2.0 == 0.0 { a } else { a + 1.0 };
        if x < 0.0 { -a } else { a }
    });
    f_odd = |x| Ok({
        let a = x.abs().ceil();
        let a = if a % 2.0 == 1.0 { a } else { a + 1.0 };
        if x < 0.0 { -a } else { a }
    });
    f_fact = |x| domain(x >= 0.0, factorial(x.trunc()));
    f_factdouble = |x| {
        let n = x.trunc();
        if n < -1.0 {
            Err(ErrorKind::Num)
        } else {
            let mut r = 1.0f64;
            let mut i = n;
            while i > 1.0 && r.is_finite() {
                r *= i;
                i -= 2.0;
            }
            Ok(r)
        }
    };
}

fn f_pi(_ctx: &EvalCtx, _args: &[Expr]) -> R<Value> {
    Ok(Value::Number(PI))
}

fn f_atan2(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let x = ctx.num(&args[0])?;
    let y = ctx.num(&args[1])?;
    if x == 0.0 && y == 0.0 {
        return Err(ErrorKind::Div0);
    }
    number(y.atan2(x))
}

fn f_quotient(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let n = ctx.num(&args[0])?;
    let d = ctx.num(&args[1])?;
    if d == 0.0 {
        return Err(ErrorKind::Div0);
    }
    number((n / d).trunc())
}

/// `n - d*INT(n/d)`: the result takes the sign of the divisor.
fn f_mod(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let n = ctx.num(&args[0])?;
    let d = ctx.num(&args[1])?;
    if d == 0.0 {
        return Err(ErrorKind::Div0);
    }
    let q = n / d;
    if q.abs() >= 4_503_599_627_370_496.0 {
        // The quotient has no fractional bits left: use the exact remainder.
        let r = n % d;
        return number(if r != 0.0 && (r < 0.0) != (d < 0.0) { r + d } else { r });
    }
    number(n - d * q.floor())
}

fn f_power(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let base = ctx.num(&args[0])?;
    let exp = ctx.num(&args[1])?;
    if base == 0.0 && exp == 0.0 {
        return Err(ErrorKind::Num);
    }
    if base == 0.0 && exp < 0.0 {
        return Err(ErrorKind::Div0);
    }
    number(base.powf(exp))
}

fn f_log(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let x = ctx.num(&args[0])?;
    let base = ctx.num_or(args, 1, 10.0)?;
    if x <= 0.0 || base <= 0.0 {
        return Err(ErrorKind::Num);
    }
    if base == 1.0 {
        return Err(ErrorKind::Div0);
    }
    let r = if base == 10.0 {
        x.log10()
    } else if base == 2.0 {
        x.log2()
    } else {
        let r = x.ln() / base.ln();
        // Exact powers give exact integers (LOG(125,5) = 3).
        let k = r.round();
        if (r - k).abs() < 1e-9 && base.powf(k) == x { k } else { r }
    };
    number(r)
}

// ---- rounding ---------------------------------------------------------------

fn digits_arg(d: f64) -> i32 {
    d.trunc().clamp(-400.0, 400.0) as i32
}

fn round_with(ctx: &EvalCtx, args: &[Expr], mode: Rounding) -> R<Value> {
    let x = ctx.num(&args[0])?;
    let digits = digits_arg(ctx.num_or(args, 1, 0.0)?);
    number(round_to(x, digits, mode))
}

fn f_round(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    round_with(ctx, args, Rounding::HalfAway)
}

fn f_roundup(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    round_with(ctx, args, Rounding::Up)
}

fn f_rounddown(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    round_with(ctx, args, Rounding::Down)
}

fn f_mround(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let x = ctx.num(&args[0])?;
    let m = ctx.num(&args[1])?;
    if x == 0.0 || m == 0.0 {
        return Ok(Value::Number(0.0));
    }
    if (x < 0.0) != (m < 0.0) {
        return Err(ErrorKind::Num);
    }
    number(to_multiple(x, m, round_half_away))
}

fn f_ceiling(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let x = ctx.num(&args[0])?;
    let s = ctx.num(&args[1])?;
    if x == 0.0 || s == 0.0 {
        return Ok(Value::Number(0.0));
    }
    if x > 0.0 && s < 0.0 {
        return Err(ErrorKind::Num);
    }
    number(to_multiple(x, s, f64::ceil))
}

fn f_floor(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let x = ctx.num(&args[0])?;
    let s = ctx.num(&args[1])?;
    if x == 0.0 {
        return Ok(Value::Number(0.0));
    }
    if s == 0.0 {
        return Err(ErrorKind::Div0);
    }
    if x > 0.0 && s < 0.0 {
        return Err(ErrorKind::Num);
    }
    number(to_multiple(x, s, f64::floor))
}

/// `CEILING.MATH` / `FLOOR.MATH`: the sign of the significance is ignored;
/// a non-zero mode reverses the direction for negative numbers.
fn math_multiple(ctx: &EvalCtx, args: &[Expr], ceiling: bool) -> R<Value> {
    let x = ctx.num(&args[0])?;
    let s = ctx.num_or(args, 1, 1.0)?.abs();
    let mode = ctx.num_or(args, 2, 0.0)?;
    if x == 0.0 || s == 0.0 {
        return Ok(Value::Number(0.0));
    }
    let up = ceiling != (x < 0.0 && mode != 0.0);
    number(to_multiple(x, s, if up { f64::ceil } else { f64::floor }))
}

fn f_ceiling_math(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    math_multiple(ctx, args, true)
}

fn f_floor_math(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    math_multiple(ctx, args, false)
}

fn f_ceiling_precise(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    math_multiple(ctx, &args[..args.len().min(2)], true)
}

fn f_floor_precise(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    math_multiple(ctx, &args[..args.len().min(2)], false)
}

// ---- random numbers -----------------------------------------------------------

fn seed() -> u64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0x9E37_79B9_7F4A_7C15);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    nanos ^ COUNTER.fetch_add(0x9E37_79B9_7F4A_7C15, AtomicOrdering::Relaxed)
}

thread_local! {
    static RNG_STATE: Cell<u64> = Cell::new(seed());
}

/// Uniform random number in `[0, 1)` (SplitMix64).
fn random() -> f64 {
    RNG_STATE.with(|state| {
        let mut z = state.get().wrapping_add(0x9E37_79B9_7F4A_7C15);
        state.set(z);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    })
}

/// Random integer in `lo ..= hi` (both integers, `lo <= hi`).
fn random_between(lo: f64, hi: f64) -> f64 {
    (lo + (random() * (hi - lo + 1.0)).floor()).min(hi)
}

fn f_rand(_ctx: &EvalCtx, _args: &[Expr]) -> R<Value> {
    Ok(Value::Number(random()))
}

fn f_randbetween(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let lo = ctx.num(&args[0])?.ceil();
    let hi = ctx.num(&args[1])?.floor();
    if lo > hi {
        return Err(ErrorKind::Num);
    }
    number(random_between(lo, hi))
}

fn f_randarray(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let rows = ctx.num_or(args, 0, 1.0)?;
    let cols = ctx.num_or(args, 1, 1.0)?;
    let min = ctx.num_or(args, 2, 0.0)?;
    let max = ctx.num_or(args, 3, 1.0)?;
    let whole = ctx.bool_or(args, 4, false)?;
    let (rows, cols) = array_dims(rows, cols)?;
    if min > max || (whole && (min.fract() != 0.0 || max.fract() != 0.0)) {
        return Err(ErrorKind::Value);
    }
    let data = (0..rows * cols)
        .map(|_| {
            if whole {
                random_between(min, max)
            } else {
                min + random() * (max - min)
            }
        })
        .collect();
    numeric_array(rows, cols, data)
}

// ---- combinatorics ----------------------------------------------------------

fn f_combin(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let n = ctx.num(&args[0])?.trunc();
    let k = ctx.num(&args[1])?.trunc();
    if n < 0.0 || k < 0.0 || k > n {
        return Err(ErrorKind::Num);
    }
    number(combin(n, k))
}

fn f_combina(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let n = ctx.num(&args[0])?.trunc();
    let k = ctx.num(&args[1])?.trunc();
    if n < 0.0 || k < 0.0 {
        return Err(ErrorKind::Num);
    }
    if k == 0.0 {
        return Ok(Value::Number(1.0));
    }
    if n == 0.0 {
        return Ok(Value::Number(0.0));
    }
    number(combin(n + k - 1.0, k))
}

fn f_permut(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let n = ctx.num(&args[0])?.trunc();
    let k = ctx.num(&args[1])?.trunc();
    if n < 0.0 || k < 0.0 || k > n {
        return Err(ErrorKind::Num);
    }
    let mut r = 1.0f64;
    let mut i = 0.0;
    while i < k && r.is_finite() {
        r *= n - i;
        i += 1.0;
    }
    number(r)
}

fn f_gcd(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    number(whole_numbers(ctx, args)?.into_iter().fold(0.0, gcd))
}

fn f_lcm(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let mut lcm = 1.0;
    for x in whole_numbers(ctx, args)? {
        if x == 0.0 || lcm == 0.0 {
            lcm = 0.0;
            continue;
        }
        lcm = lcm / gcd(lcm, x) * x;
        if lcm >= MAX_EXACT {
            return Err(ErrorKind::Num);
        }
    }
    number(lcm)
}

fn f_multinomial(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let mut total = 0.0;
    let mut result = 1.0f64;
    for x in whole_numbers(ctx, args)? {
        total += x;
        result *= combin(total, x);
        if !result.is_finite() {
            return Err(ErrorKind::Num);
        }
    }
    number(if result < MAX_EXACT { result.round() } else { result })
}

// ---- SUBTOTAL / AGGREGATE ------------------------------------------------------

#[derive(Clone, Copy)]
struct Ignore {
    /// Rows hidden by hand.
    hidden_rows: bool,
    /// Rows hidden by a filter (`Sheet::filtered_rows`).
    filtered_rows: bool,
    errors: bool,
    nested: bool,
}

/// True when the cell holds a formula calling `SUBTOTAL` or `AGGREGATE`.
fn is_subtotal_cell(ctx: &EvalCtx, sheet: usize, row: u32, col: u32) -> bool {
    let Some(expr) = ctx.formula_at(sheet, row, col) else {
        return false;
    };
    let mut found = false;
    expr.walk(&mut |e| {
        if let Expr::Func(name, _) = e
            && matches!(&**name, "SUBTOTAL" | "AGGREGATE")
        {
            found = true;
        }
    });
    found
}

/// The non-blank values of the arguments, minus what is to be ignored.
fn gather(ctx: &EvalCtx, args: &[Expr], ignore: Ignore) -> Vec<Value> {
    let mut out = Vec::new();
    for arg in args {
        match ctx.eval_ref(arg) {
            Ok(r) => {
                let values = ctx.range_values(&r);
                let sheet = ctx.workbook().sheets.get(r.sheet);
                let hidden = sheet.map(|s| &s.hidden_rows).filter(|h| ignore.hidden_rows && !h.is_empty());
                let filtered = sheet.map(|s| &s.filtered_rows).filter(|f| ignore.filtered_rows && !f.is_empty());
                for i in 0..values.rows {
                    let row = r.rect.r0 + i as u32;
                    if hidden.is_some_and(|h| h.contains(&row)) || filtered.is_some_and(|f| f.contains(&row)) {
                        continue;
                    }
                    for j in 0..values.cols {
                        let v = values.get(i, j);
                        if v.is_empty() {
                            continue;
                        }
                        if ignore.nested && is_subtotal_cell(ctx, r.sheet, row, r.rect.c0 + j as u32) {
                            continue;
                        }
                        out.push(v.clone());
                    }
                }
            }
            Err(_) => out.extend(
                ctx.flatten(std::slice::from_ref(arg))
                    .into_iter()
                    .filter(|v| !v.is_empty()),
            ),
        }
    }
    if ignore.errors {
        out.retain(|v| !v.is_error());
    }
    out
}

/// Apply aggregation `func` (the numbering shared by `SUBTOTAL` 1-11 and
/// `AGGREGATE` 1-19) to gathered values.
fn aggregate(func: u32, values: &[Value], k: Option<f64>) -> R<Value> {
    match func {
        2 => return Ok(Value::Number(values.iter().filter(|v| v.is_number()).count() as f64)),
        3 => return Ok(Value::Number(values.len() as f64)),
        _ => {}
    }
    let mut nums = Vec::with_capacity(values.len());
    for v in values {
        match v {
            Value::Number(n) => nums.push(*n),
            Value::Error(e) => return Err(*e),
            _ => {}
        }
    }
    let out = match func {
        1 if nums.is_empty() => return Err(ErrorKind::Div0),
        1 => stat::mean(&nums),
        4 => stat::extreme(&nums, true),
        5 => stat::extreme(&nums, false),
        6 if nums.is_empty() => 0.0,
        6 => nums.iter().product(),
        7 => stat::variance(&nums, true)?.sqrt(),
        8 => stat::variance(&nums, false)?.sqrt(),
        9 => nums.iter().sum(),
        10 => stat::variance(&nums, true)?,
        11 => stat::variance(&nums, false)?,
        12 => stat::median(&mut nums)?,
        13 => *stat::modes(&nums).first().ok_or(ErrorKind::NA)?,
        14..=19 => {
            let k = k.ok_or(ErrorKind::Value)?;
            nums.sort_by(f64::total_cmp);
            match func {
                14 => stat::kth(&nums, k, true)?,
                15 => stat::kth(&nums, k, false)?,
                16 => stat::percentile(&nums, k, false)?,
                17 => stat::quartile(&nums, k, false)?,
                18 => stat::percentile(&nums, k, true)?,
                _ => stat::quartile(&nums, k, true)?,
            }
        }
        _ => return Err(ErrorKind::Value),
    };
    number(out)
}

fn f_subtotal(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let code = ctx.num(&args[0])?.trunc();
    let (func, hidden_rows) = if (1.0..=11.0).contains(&code) {
        (code as u32, false)
    } else if (101.0..=111.0).contains(&code) {
        (code as u32 - 100, true)
    } else {
        return Err(ErrorKind::Value);
    };
    // Rows a filter hides never count; rows hidden by hand only drop out
    // of the 101-111 variants.
    let ignore = Ignore {
        hidden_rows,
        filtered_rows: true,
        errors: false,
        nested: true,
    };
    aggregate(func, &gather(ctx, &args[1..], ignore), None)
}

fn f_aggregate(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let func = ctx.num(&args[0])?.trunc();
    let option = ctx.num(&args[1])?.trunc();
    if !(1.0..=19.0).contains(&func) || !(0.0..=7.0).contains(&option) {
        return Err(ErrorKind::Value);
    }
    let (func, option) = (func as u32, option as u32);
    // "Ignore hidden rows" covers both rows hidden by hand and by a filter.
    let ignore = Ignore {
        hidden_rows: matches!(option, 1 | 3 | 5 | 7),
        filtered_rows: matches!(option, 1 | 3 | 5 | 7),
        errors: matches!(option, 2 | 3 | 6 | 7),
        nested: option <= 3,
    };
    if func >= 14 {
        // Array form: AGGREGATE(function, options, array, k).
        if args.len() != 4 {
            return Err(ErrorKind::Value);
        }
        let values = gather(ctx, &args[2..3], ignore);
        return lift(ctx, &[args.get(3)], |k| {
            let k = ctx.to_number(k[0].as_ref().ok_or(ErrorKind::Value)?)?;
            aggregate(func, &values, Some(k))
        });
    }
    aggregate(func, &gather(ctx, &args[2..], ignore), None)
}

// ---- number systems -----------------------------------------------------------

fn f_roman(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    const CHARS: [char; 7] = ['M', 'D', 'C', 'L', 'X', 'V', 'I'];
    const VALUES: [u32; 7] = [1000, 500, 100, 50, 10, 5, 1];
    let n = ctx.num(&args[0])?.trunc();
    // The form may also be given as TRUE (classic) / FALSE (most concise).
    let form = match args.get(1) {
        None | Some(Expr::Missing) => 0.0,
        Some(e) => match ctx.eval_scalar(e) {
            Value::Bool(true) => 0.0,
            Value::Bool(false) => 4.0,
            v => ctx.to_number(&v)?.trunc(),
        },
    };
    if !(0.0..=3999.0).contains(&n) || !(0.0..=4.0).contains(&form) {
        return Err(ErrorKind::Value);
    }
    let mut val = n as u32;
    let form = form as u32;
    let mut out = String::new();
    for i in 0..=3 {
        let mut index = 2 * i;
        let digit = val / VALUES[index];
        if digit % 5 == 4 {
            // Subtractive notation; higher forms subtract smaller symbols.
            let index2 = if digit == 4 { index - 1 } else { index - 2 };
            let mut steps = 0;
            while steps < form && index < 6 {
                steps += 1;
                if VALUES[index2] - VALUES[index + 1] <= val {
                    index += 1;
                } else {
                    steps = form;
                }
            }
            out.push(CHARS[index]);
            out.push(CHARS[index2]);
            val = val + VALUES[index] - VALUES[index2];
        } else {
            if digit > 4 {
                out.push(CHARS[index - 1]);
            }
            for _ in 0..digit % 5 {
                out.push(CHARS[index]);
            }
            val %= VALUES[index];
        }
    }
    Ok(Value::from(out))
}

fn f_arabic(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let text = ctx.text(&args[0])?;
    let text = text.trim();
    if text.chars().count() > 255 {
        return Err(ErrorKind::Value);
    }
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let mut values = Vec::with_capacity(digits.len());
    for ch in digits.chars() {
        values.push(match ch.to_ascii_uppercase() {
            'I' => 1.0,
            'V' => 5.0,
            'X' => 10.0,
            'L' => 50.0,
            'C' => 100.0,
            'D' => 500.0,
            'M' => 1000.0,
            _ => return Err(ErrorKind::Value),
        });
    }
    if negative && values.is_empty() {
        return Err(ErrorKind::Value);
    }
    let mut total = 0.0;
    for (i, v) in values.iter().enumerate() {
        if values.get(i + 1).is_some_and(|next| next > v) {
            total -= v;
        } else {
            total += v;
        }
    }
    number(if negative { -total } else { total })
}

fn radix_arg(ctx: &EvalCtx, e: &Expr) -> R<u32> {
    let radix = ctx.num(e)?.trunc();
    if (2.0..=36.0).contains(&radix) {
        Ok(radix as u32)
    } else {
        Err(ErrorKind::Num)
    }
}

fn f_base(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let n = ctx.num(&args[0])?.trunc();
    let radix = radix_arg(ctx, &args[1])? as u64;
    let min_len = ctx.num_or(args, 2, 0.0)?.trunc();
    if !(0.0..MAX_EXACT).contains(&n) || !(0.0..=255.0).contains(&min_len) {
        return Err(ErrorKind::Num);
    }
    let mut n = n as u64;
    let mut digits = Vec::new();
    loop {
        digits.push(char::from_digit((n % radix) as u32, radix as u32).unwrap_or('0').to_ascii_uppercase());
        n /= radix;
        if n == 0 {
            break;
        }
    }
    while digits.len() < min_len as usize {
        digits.push('0');
    }
    Ok(Value::from(digits.into_iter().rev().collect::<String>()))
}

fn f_decimal(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let text = ctx.text(&args[0])?;
    let radix = radix_arg(ctx, &args[1])?;
    let text = text.trim();
    if text.chars().count() > 255 {
        return Err(ErrorKind::Value);
    }
    let mut n = 0.0f64;
    for ch in text.chars() {
        let digit = ch.to_digit(radix).ok_or(ErrorKind::Num)?;
        n = n * radix as f64 + digit as f64;
    }
    number(n)
}

// ---- arrays and matrices ------------------------------------------------------

fn f_sequence(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let rows = ctx.num_or(args, 0, 1.0)?;
    let cols = ctx.num_or(args, 1, 1.0)?;
    let start = ctx.num_or(args, 2, 1.0)?;
    let step = ctx.num_or(args, 3, 1.0)?;
    let (rows, cols) = array_dims(rows, cols)?;
    let data = (0..rows * cols).map(|i| start + step * i as f64).collect();
    numeric_array(rows, cols, data)
}

fn f_munit(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let n = ctx.num(&args[0])?.trunc();
    if n < 1.0 {
        return Err(ErrorKind::Value);
    }
    let (n, _) = array_dims(n, n)?;
    let mut out = Array::new(n, n);
    for r in 0..n {
        for c in 0..n {
            out.set(r, c, Value::Number(if r == c { 1.0 } else { 0.0 }));
        }
    }
    Ok(Value::array(out))
}

fn f_mmult(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let (ar, ac, a) = matrix(ctx, &args[0])?;
    let (br, bc, b) = matrix(ctx, &args[1])?;
    if ac != br {
        return Err(ErrorKind::Value);
    }
    let mut data = Vec::with_capacity(ar * bc);
    for r in 0..ar {
        for c in 0..bc {
            data.push((0..ac).map(|k| a[r * ac + k] * b[k * bc + c]).sum());
        }
    }
    numeric_array(ar, bc, data)
}

fn square_matrix(ctx: &EvalCtx, e: &Expr) -> R<(usize, Vec<f64>)> {
    let (rows, cols, data) = matrix(ctx, e)?;
    if rows != cols {
        return Err(ErrorKind::Value);
    }
    Ok((rows, data))
}

/// Determinant by LU decomposition with partial pivoting.
fn determinant(n: usize, m: &mut [f64]) -> f64 {
    let mut det = 1.0;
    for i in 0..n {
        let mut p = i;
        for r in i + 1..n {
            if m[r * n + i].abs() > m[p * n + i].abs() {
                p = r;
            }
        }
        if m[p * n + i] == 0.0 {
            return 0.0;
        }
        if p != i {
            for c in 0..n {
                m.swap(i * n + c, p * n + c);
            }
            det = -det;
        }
        let pivot = m[i * n + i];
        det *= pivot;
        for r in i + 1..n {
            let f = m[r * n + i] / pivot;
            if f != 0.0 {
                for c in i..n {
                    m[r * n + c] -= f * m[i * n + c];
                }
            }
        }
    }
    det
}

fn f_mdeterm(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let (n, mut m) = square_matrix(ctx, &args[0])?;
    let integral = m.iter().all(|x| x.fract() == 0.0);
    let det = determinant(n, &mut m);
    // The determinant of an integer matrix is an integer.
    number(if integral && det.abs() < MAX_EXACT { det.round() } else { det })
}

/// Inverse by Gauss-Jordan elimination with partial pivoting; `None` when
/// the matrix is singular.
fn inverse(n: usize, a: &[f64]) -> Option<Vec<f64>> {
    let mut m = a.to_vec();
    let mut inv = vec![0.0; n * n];
    for i in 0..n {
        inv[i * n + i] = 1.0;
    }
    // Pivots are judged against the size of their original column.
    let scale: Vec<f64> = (0..n)
        .map(|c| (0..n).map(|r| a[r * n + c].abs()).fold(0.0, f64::max))
        .collect();
    for i in 0..n {
        let mut p = i;
        for r in i + 1..n {
            if m[r * n + i].abs() > m[p * n + i].abs() {
                p = r;
            }
        }
        if m[p * n + i].abs() <= scale[i] * 1e-13 {
            return None;
        }
        if p != i {
            for c in 0..n {
                m.swap(i * n + c, p * n + c);
                inv.swap(i * n + c, p * n + c);
            }
        }
        let pivot = m[i * n + i];
        for c in 0..n {
            m[i * n + c] /= pivot;
            inv[i * n + c] /= pivot;
        }
        for r in 0..n {
            let f = m[r * n + i];
            if r != i && f != 0.0 {
                for c in 0..n {
                    m[r * n + c] -= f * m[i * n + c];
                    inv[r * n + c] -= f * inv[i * n + c];
                }
            }
        }
    }
    Some(inv)
}

fn f_minverse(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let (n, m) = square_matrix(ctx, &args[0])?;
    let inv = inverse(n, &m).ok_or(ErrorKind::Num)?;
    numeric_array(n, n, inv)
}

pub const FUNCTIONS: &[FunctionDef] = &[
    FunctionDef::new("SUM", 1, VAR, Kind::Raw, Category::Math, f_sum),
    FunctionDef::new("SUMIF", 2, 3, Kind::Raw, Category::Math, f_sumif),
    FunctionDef::new("SUMIFS", 3, VAR, Kind::Raw, Category::Math, f_sumifs),
    FunctionDef::new("SUMPRODUCT", 1, VAR, Kind::Raw, Category::Math, f_sumproduct),
    FunctionDef::new("SUMSQ", 1, VAR, Kind::Raw, Category::Math, f_sumsq),
    FunctionDef::new("SUMX2MY2", 2, 2, Kind::Raw, Category::Math, f_sumx2my2),
    FunctionDef::new("SUMX2PY2", 2, 2, Kind::Raw, Category::Math, f_sumx2py2),
    FunctionDef::new("SUMXMY2", 2, 2, Kind::Raw, Category::Math, f_sumxmy2),
    FunctionDef::new("PRODUCT", 1, VAR, Kind::Raw, Category::Math, f_product),
    FunctionDef::new("SERIESSUM", 4, 4, Kind::Raw, Category::Math, f_seriessum),
    FunctionDef::new("QUOTIENT", 2, 2, Kind::Scalar, Category::Math, f_quotient),
    FunctionDef::new("MOD", 2, 2, Kind::Scalar, Category::Math, f_mod),
    FunctionDef::new("ABS", 1, 1, Kind::Scalar, Category::Math, f_abs),
    FunctionDef::new("SIGN", 1, 1, Kind::Scalar, Category::Math, f_sign),
    FunctionDef::new("SQRT", 1, 1, Kind::Scalar, Category::Math, f_sqrt),
    FunctionDef::new("SQRTPI", 1, 1, Kind::Scalar, Category::Math, f_sqrtpi),
    FunctionDef::new("POWER", 2, 2, Kind::Scalar, Category::Math, f_power),
    FunctionDef::new("EXP", 1, 1, Kind::Scalar, Category::Math, f_exp),
    FunctionDef::new("LN", 1, 1, Kind::Scalar, Category::Math, f_ln),
    FunctionDef::new("LOG", 1, 2, Kind::Scalar, Category::Math, f_log),
    FunctionDef::new("LOG10", 1, 1, Kind::Scalar, Category::Math, f_log10),
    FunctionDef::new("ROUND", 2, 2, Kind::Scalar, Category::Math, f_round),
    FunctionDef::new("ROUNDUP", 2, 2, Kind::Scalar, Category::Math, f_roundup),
    FunctionDef::new("ROUNDDOWN", 2, 2, Kind::Scalar, Category::Math, f_rounddown),
    FunctionDef::new("MROUND", 2, 2, Kind::Scalar, Category::Math, f_mround),
    FunctionDef::new("INT", 1, 1, Kind::Scalar, Category::Math, f_int),
    FunctionDef::new("TRUNC", 1, 2, Kind::Scalar, Category::Math, f_rounddown),
    FunctionDef::new("CEILING", 2, 2, Kind::Scalar, Category::Math, f_ceiling),
    FunctionDef::new("FLOOR", 2, 2, Kind::Scalar, Category::Math, f_floor),
    FunctionDef::new("CEILING.MATH", 1, 3, Kind::Scalar, Category::Math, f_ceiling_math),
    FunctionDef::new("FLOOR.MATH", 1, 3, Kind::Scalar, Category::Math, f_floor_math),
    FunctionDef::new("CEILING.PRECISE", 1, 2, Kind::Scalar, Category::Math, f_ceiling_precise),
    FunctionDef::new("FLOOR.PRECISE", 1, 2, Kind::Scalar, Category::Math, f_floor_precise),
    FunctionDef::new("ISO.CEILING", 1, 2, Kind::Scalar, Category::Math, f_ceiling_precise),
    FunctionDef::new("EVEN", 1, 1, Kind::Scalar, Category::Math, f_even),
    FunctionDef::new("ODD", 1, 1, Kind::Scalar, Category::Math, f_odd),
    FunctionDef::new("RAND", 0, 0, Kind::Scalar, Category::Math, f_rand),
    FunctionDef::new("RANDBETWEEN", 2, 2, Kind::Scalar, Category::Math, f_randbetween),
    FunctionDef::new("RANDARRAY", 0, 5, Kind::Raw, Category::Math, f_randarray),
    FunctionDef::new("PI", 0, 0, Kind::Scalar, Category::Math, f_pi),
    FunctionDef::new("SIN", 1, 1, Kind::Scalar, Category::Math, f_sin),
    FunctionDef::new("COS", 1, 1, Kind::Scalar, Category::Math, f_cos),
    FunctionDef::new("TAN", 1, 1, Kind::Scalar, Category::Math, f_tan),
    FunctionDef::new("ASIN", 1, 1, Kind::Scalar, Category::Math, f_asin),
    FunctionDef::new("ACOS", 1, 1, Kind::Scalar, Category::Math, f_acos),
    FunctionDef::new("ATAN", 1, 1, Kind::Scalar, Category::Math, f_atan),
    FunctionDef::new("ATAN2", 2, 2, Kind::Scalar, Category::Math, f_atan2),
    FunctionDef::new("SINH", 1, 1, Kind::Scalar, Category::Math, f_sinh),
    FunctionDef::new("COSH", 1, 1, Kind::Scalar, Category::Math, f_cosh),
    FunctionDef::new("TANH", 1, 1, Kind::Scalar, Category::Math, f_tanh),
    FunctionDef::new("ASINH", 1, 1, Kind::Scalar, Category::Math, f_asinh),
    FunctionDef::new("ACOSH", 1, 1, Kind::Scalar, Category::Math, f_acosh),
    FunctionDef::new("ATANH", 1, 1, Kind::Scalar, Category::Math, f_atanh),
    FunctionDef::new("COT", 1, 1, Kind::Scalar, Category::Math, f_cot),
    FunctionDef::new("SEC", 1, 1, Kind::Scalar, Category::Math, f_sec),
    FunctionDef::new("CSC", 1, 1, Kind::Scalar, Category::Math, f_csc),
    FunctionDef::new("ACOT", 1, 1, Kind::Scalar, Category::Math, f_acot),
    FunctionDef::new("DEGREES", 1, 1, Kind::Scalar, Category::Math, f_degrees),
    FunctionDef::new("RADIANS", 1, 1, Kind::Scalar, Category::Math, f_radians),
    FunctionDef::new("FACT", 1, 1, Kind::Scalar, Category::Math, f_fact),
    FunctionDef::new("FACTDOUBLE", 1, 1, Kind::Scalar, Category::Math, f_factdouble),
    FunctionDef::new("COMBIN", 2, 2, Kind::Scalar, Category::Math, f_combin),
    FunctionDef::new("COMBINA", 2, 2, Kind::Scalar, Category::Math, f_combina),
    FunctionDef::new("PERMUT", 2, 2, Kind::Scalar, Category::Math, f_permut),
    FunctionDef::new("GCD", 1, VAR, Kind::Raw, Category::Math, f_gcd),
    FunctionDef::new("LCM", 1, VAR, Kind::Raw, Category::Math, f_lcm),
    FunctionDef::new("MULTINOMIAL", 1, VAR, Kind::Raw, Category::Math, f_multinomial),
    FunctionDef::new("SUBTOTAL", 2, VAR, Kind::Raw, Category::Math, f_subtotal),
    FunctionDef::new("AGGREGATE", 3, VAR, Kind::Raw, Category::Math, f_aggregate),
    FunctionDef::new("ROMAN", 1, 2, Kind::Scalar, Category::Math, f_roman),
    FunctionDef::new("ARABIC", 1, 1, Kind::Scalar, Category::Math, f_arabic),
    FunctionDef::new("BASE", 2, 3, Kind::Scalar, Category::Math, f_base),
    FunctionDef::new("DECIMAL", 2, 2, Kind::Scalar, Category::Math, f_decimal),
    FunctionDef::new("SEQUENCE", 1, 4, Kind::Raw, Category::Math, f_sequence),
    FunctionDef::new("MMULT", 2, 2, Kind::Raw, Category::Math, f_mmult),
    FunctionDef::new("MINVERSE", 1, 1, Kind::Raw, Category::Math, f_minverse),
    FunctionDef::new("MDETERM", 1, 1, Kind::Raw, Category::Math, f_mdeterm),
    FunctionDef::new("MUNIT", 1, 1, Kind::Raw, Category::Math, f_munit),
];

#[cfg(test)]
mod tests {
    use super::{Rounding, mul_exact, round_to, snap15};
    use crate::tests::{eval, num, val, wb};
    use crate::value::{ErrorKind, Value};
    use crate::workbook::Workbook;

    fn err(formula: &str) -> ErrorKind {
        match eval(formula) {
            Value::Error(e) => e,
            other => panic!("{formula} gave {other:?}, expected an error"),
        }
    }

    fn close(formula: &str, expected: f64) {
        let got = num(formula);
        assert!(
            (got - expected).abs() <= 1e-9 * expected.abs().max(1.0),
            "{formula}: got {got}, expected {expected}"
        );
    }

    fn text(formula: &str) -> String {
        match eval(formula) {
            Value::Text(s) => s.to_string(),
            other => panic!("{formula} gave {other:?}, expected text"),
        }
    }

    fn cell(w: &Workbook, name: &str) -> f64 {
        match val(w, name) {
            Value::Number(n) => n,
            other => panic!("{name} holds {other:?}, expected a number"),
        }
    }

    /// The array a formula spills, read back from the sheet.
    fn spill(formula: &str, rows: u32, cols: u32) -> Vec<Vec<f64>> {
        let w = wb(&[("A1", &format!("={formula}"))]);
        (0..rows)
            .map(|r| {
                (0..cols)
                    .map(|c| match w.value_at(0, r, c) {
                        Value::Number(n) => n,
                        other => panic!("{formula} [{r},{c}] is {other:?}"),
                    })
                    .collect()
            })
            .collect()
    }

    /// Category / amount / region table used by the conditional sums.
    fn sales() -> Workbook {
        wb(&[
            ("A1", "fruit"),
            ("A2", "veg"),
            ("A3", "fruit"),
            ("A4", "Fruit"),
            ("A5", "meat"),
            ("B1", "10"),
            ("B2", "20"),
            ("B3", "30"),
            ("B4", "40"),
            ("B5", "n/a"),
            ("C1", "north"),
            ("C2", "south"),
            ("C3", "south"),
            ("C4", "north"),
            ("C5", "north"),
        ])
    }

    fn calc(w: &mut Workbook, formula: &str) -> Value {
        w.set_cell_text(0, 49, 9, &format!("={formula}")).unwrap();
        w.commit("formula");
        w.value_at(0, 49, 9)
    }

    #[test]
    fn rounding_helpers() {
        assert_eq!(snap15(6.999999999999999), 7.0);
        assert_eq!(snap15(0.1 + 0.2), 0.3);
        assert_eq!(snap15(-1234.5), -1234.5);
        assert_eq!(snap15(1234567890123456.0), 1234567890123456.0);
        assert_eq!(round_to(2.675, 2, Rounding::HalfAway), 2.68);
        assert_eq!(round_to(1.005, 2, Rounding::HalfAway), 1.01);
        assert_eq!(round_to(999.5, 0, Rounding::HalfAway), 1000.0);
        assert_eq!(round_to(0.95, 1, Rounding::HalfAway), 1.0);
        assert_eq!(round_to(0.3, 1, Rounding::Up), 0.3);
        assert_eq!(round_to(0.1 + 0.2, 1, Rounding::Up), 0.3);
        assert_eq!(round_to(0.30001, 1, Rounding::Up), 0.4);
        assert_eq!(round_to(4.35 * 100.0, 0, Rounding::Down), 435.0);
        assert_eq!(round_to(1e20, 2, Rounding::HalfAway), 1e20);
        assert_eq!(round_to(9007199254740992.0, 0, Rounding::HalfAway), 9007199254740992.0);
        assert_eq!(round_to(1234567890123456.0, -1, Rounding::HalfAway), 1234567890123460.0);
        assert_eq!(round_to(1234567890123456.0, -1, Rounding::Down), 1234567890123450.0);
        assert_eq!(round_to(-1250.0, -2, Rounding::HalfAway), -1300.0);
        assert_eq!(round_to(0.1 + 0.2, 15, Rounding::HalfAway), 0.3);
        assert_eq!(round_to(123456789.123456, 4, Rounding::HalfAway), 123456789.1235);
        assert_eq!(mul_exact(7.0, 0.1), 0.7);
        assert_eq!(mul_exact(-3.0, 0.1), -0.3);
        assert_eq!(mul_exact(2.0, 1.0 / 3.0), 2.0 / 3.0);
        assert_eq!(mul_exact(12.0, 250.0), 3000.0);
    }

    #[test]
    fn sum() {
        assert_eq!(num("SUM(1,2,3)"), 6.0);
        assert_eq!(num("SUM(\"3\",TRUE,2)"), 6.0);
        assert_eq!(num("SUM({1,2;3,4})"), 10.0);
        assert_eq!(num("SUM(1,)"), 1.0);
        assert_eq!(err("SUM(\"a\",1)"), ErrorKind::Value);
        assert_eq!(err("SUM(1,1/0)"), ErrorKind::Div0);
        let w = wb(&[
            ("A1", "10"),
            ("A2", "20"),
            ("A3", "x"),
            ("A4", "TRUE"),
            ("A5", "'5"),
            ("A7", "30"),
            ("B1", "=SUM(A1:A7)"),
            ("B2", "=SUM(A3)"),
            ("B3", "=SUM(A1,A3,A4,A5)"),
            ("B4", "=SUM(A:A)"),
            ("B5", "=SUM(A1:A2,5,\"1\")"),
        ]);
        assert_eq!(cell(&w, "B1"), 60.0);
        assert_eq!(cell(&w, "B2"), 0.0);
        assert_eq!(cell(&w, "B3"), 10.0);
        assert_eq!(cell(&w, "B4"), 60.0);
        assert_eq!(cell(&w, "B5"), 36.0);
        let w = wb(&[("A1", "1"), ("A2", "#N/A"), ("B1", "=SUM(A1:A2)")]);
        assert_eq!(val(&w, "B1"), Value::Error(ErrorKind::NA));
    }

    #[test]
    fn sumif() {
        let mut w = sales();
        let mut n = |f: &str| match calc(&mut w, f) {
            Value::Number(n) => n,
            other => panic!("{f} gave {other:?}"),
        };
        assert_eq!(n("SUMIF(A1:A5,\"fruit\",B1:B5)"), 80.0);
        assert_eq!(n("SUMIF(A1:A5,\"f*\",B1:B5)"), 80.0);
        assert_eq!(n("SUMIF(A1:A5,\"?eg\",B1:B5)"), 20.0);
        assert_eq!(n("SUMIF(A1:A5,\"<>fruit\",B1:B5)"), 20.0);
        assert_eq!(n("SUMIF(B1:B5,\">15\")"), 90.0);
        assert_eq!(n("SUMIF(B1:B5,\">=20\")"), 90.0);
        assert_eq!(n("SUMIF(B1:B5,\"<20\")"), 10.0);
        assert_eq!(n("SUMIF(B1:B5,\"<=20\")"), 30.0);
        assert_eq!(n("SUMIF(B1:B5,20)"), 20.0);
        assert_eq!(n("SUMIF(B1:B5,\"=20\")"), 20.0);
        assert_eq!(n("SUMIF(B1:B5,\"<>20\")"), 80.0);
        assert_eq!(n("SUMIF(B1:B5,\">\"&B2)"), 70.0);
        // Only the top-left corner of the sum range matters.
        assert_eq!(n("SUMIF(A1:A5,\"fruit\",B1)"), 80.0);
        assert_eq!(n("SUMIF(A1:A5,\"fruit\",B2:B3)"), 60.0);
        // Whole columns.
        assert_eq!(n("SUMIF(A:A,\"veg\",B:B)"), 20.0);
        assert_eq!(n("SUMIF(C:C,\"north\",B:B)"), 50.0);
        // Nothing matches.
        assert_eq!(n("SUMIF(A1:A5,\"fish\",B1:B5)"), 0.0);
        // Criterion taken from a cell.
        assert_eq!(n("SUMIF(A1:A5,A2,B1:B5)"), 20.0);
    }

    #[test]
    fn sumif_errors() {
        let w = wb(&[
            ("A1", "a"),
            ("A2", "b"),
            ("B1", "#DIV/0!"),
            ("B2", "5"),
            ("D1", "=SUMIF(A1:A2,\"a\",B1:B2)"),
            ("D2", "=SUMIF(A1:A2,\"b\",B1:B2)"),
            ("D3", "=SUMIF(B1:B2,\">0\")"),
            ("D4", "=COUNTIF(B1:B2,\"#DIV/0!\")"),
        ]);
        assert_eq!(val(&w, "D1"), Value::Error(ErrorKind::Div0));
        assert_eq!(val(&w, "D2"), Value::Number(5.0));
        assert_eq!(val(&w, "D3"), Value::Number(5.0));
        assert_eq!(val(&w, "D4"), Value::Number(1.0));
    }

    #[test]
    fn sumifs() {
        let mut w = sales();
        assert_eq!(calc(&mut w, "SUMIFS(B1:B5,A1:A5,\"fruit\",C1:C5,\"north\")"), Value::Number(50.0));
        assert_eq!(calc(&mut w, "SUMIFS(B1:B5,A1:A5,\"fruit\",B1:B5,\">10\")"), Value::Number(70.0));
        assert_eq!(calc(&mut w, "SUMIFS(B:B,A:A,\"fruit\",C:C,\"s*\")"), Value::Number(30.0));
        assert_eq!(calc(&mut w, "SUMIFS(B1:B5,C1:C5,\"north\")"), Value::Number(50.0));
        assert_eq!(calc(&mut w, "SUMIFS(B1:B5,A1:A5,\"fish\")"), Value::Number(0.0));
        // Ranges of different shapes.
        assert_eq!(calc(&mut w, "SUMIFS(B1:B5,A1:A4,\"fruit\")"), Value::Error(ErrorKind::Value));
        assert_eq!(calc(&mut w, "SUMIFS(B1:B5,A1:A5,\"fruit\",C1:C5)"), Value::Error(ErrorKind::Value));
    }

    #[test]
    fn sumproduct() {
        assert_eq!(num("SUMPRODUCT({1,2,3},{4,5,6})"), 32.0);
        assert_eq!(num("SUMPRODUCT({3,4;8,6;1,9},{2,7;6,7;5,3})"), 156.0);
        assert_eq!(num("SUMPRODUCT({1,2,3})"), 6.0);
        assert_eq!(num("SUMPRODUCT({1,2,\"x\"},{4,5,6})"), 14.0);
        assert_eq!(num("SUMPRODUCT({1,2,TRUE},{4,5,6})"), 14.0);
        assert_eq!(num("SUMPRODUCT(2,3)"), 6.0);
        assert_eq!(err("SUMPRODUCT({1,2,3},{4,5})"), ErrorKind::Value);
        assert_eq!(err("SUMPRODUCT({1,2,3},{4;5;6})"), ErrorKind::Value);
        assert_eq!(err("SUMPRODUCT({1,2,#N/A},{4,5,6})"), ErrorKind::NA);
        let mut w = sales();
        assert_eq!(calc(&mut w, "SUMPRODUCT((A1:A4=\"fruit\")*(B1:B4))"), Value::Number(80.0));
        assert_eq!(calc(&mut w, "SUMPRODUCT(--(A1:A4=\"fruit\"),B1:B4)"), Value::Number(80.0));
        assert_eq!(calc(&mut w, "SUMPRODUCT((A1:A4=\"fruit\")*(C1:C4=\"north\")*B1:B4)"), Value::Number(50.0));
        // Booleans are not numbers here.
        assert_eq!(calc(&mut w, "SUMPRODUCT(A1:A4=\"fruit\")"), Value::Number(0.0));
        // Text in a range counts as 0.
        assert_eq!(calc(&mut w, "SUMPRODUCT(B1:B5,B1:B5)"), Value::Number(3000.0));
        assert_eq!(calc(&mut w, "SUMPRODUCT(B:B,B:B)"), Value::Number(3000.0));
        assert_eq!(calc(&mut w, "SUMPRODUCT(B1:B5,B1:B4)"), Value::Error(ErrorKind::Value));
    }

    #[test]
    fn sums_of_squares() {
        assert_eq!(num("SUMSQ(3,4)"), 25.0);
        assert_eq!(num("SUMSQ({1,2,3},\"2\")"), 18.0);
        assert_eq!(num("SUMX2MY2({2,3,9,1,8,7,5},{6,5,11,7,5,4,4})"), -55.0);
        assert_eq!(num("SUMX2PY2({2,3,9,1,8,7,5},{6,5,11,7,5,4,4})"), 521.0);
        assert_eq!(num("SUMXMY2({2,3,9,1,8,7,5},{6,5,11,7,5,4,4})"), 79.0);
        assert_eq!(num("SUMXMY2({1,2,\"x\"},{3,5,9})"), 13.0);
        assert_eq!(err("SUMX2MY2({1,2,3},{1,2})"), ErrorKind::NA);
        assert_eq!(err("SUMXMY2({1,#DIV/0!},{1,2})"), ErrorKind::Div0);
    }

    #[test]
    fn product_quotient_mod() {
        assert_eq!(num("PRODUCT(5,15,30)"), 2250.0);
        assert_eq!(num("PRODUCT({2,3},\"4\")"), 24.0);
        let w = wb(&[("A1", "2"), ("A2", "x"), ("A3", "5"), ("B1", "=PRODUCT(A1:A4)"), ("B2", "=PRODUCT(A2)")]);
        assert_eq!(cell(&w, "B1"), 10.0);
        assert_eq!(cell(&w, "B2"), 0.0);
        assert_eq!(num("QUOTIENT(5,2)"), 2.0);
        assert_eq!(num("QUOTIENT(4.5,3.1)"), 1.0);
        assert_eq!(num("QUOTIENT(-10,3)"), -3.0);
        assert_eq!(err("QUOTIENT(1,0)"), ErrorKind::Div0);
        assert_eq!(err("QUOTIENT(\"a\",1)"), ErrorKind::Value);
        assert_eq!(num("MOD(3,2)"), 1.0);
        assert_eq!(num("MOD(-3,2)"), 1.0);
        assert_eq!(num("MOD(3,-2)"), -1.0);
        assert_eq!(num("MOD(-3,-2)"), -1.0);
        assert_eq!(num("MOD(10,3)"), 1.0);
        assert_eq!(num("MOD(5.5,2)"), 1.5);
        assert_eq!(num("MOD(6,3)"), 0.0);
        assert_eq!(num("MOD(-6,3)"), 0.0);
        assert_eq!(err("MOD(5,0)"), ErrorKind::Div0);
    }

    #[test]
    fn basic_scalars() {
        assert_eq!(num("ABS(-2.5)"), 2.5);
        assert_eq!(num("ABS(\"-3\")"), 3.0);
        assert_eq!(num("ABS(TRUE)"), 1.0);
        assert_eq!(err("ABS(\"x\")"), ErrorKind::Value);
        assert_eq!(err("ABS(#N/A)"), ErrorKind::NA);
        assert_eq!(num("SUM(ABS({-1,2,-3}))"), 6.0);
        assert_eq!(num("SIGN(10)"), 1.0);
        assert_eq!(num("SIGN(4-4)"), 0.0);
        assert_eq!(num("SIGN(-0.00001)"), -1.0);
        assert_eq!(num("SQRT(16)"), 4.0);
        assert_eq!(err("SQRT(-16)"), ErrorKind::Num);
        close("SQRTPI(1)", 1.7724538509055159);
        close("SQRTPI(2)", 2.5066282746310002);
        assert_eq!(err("SQRTPI(-1)"), ErrorKind::Num);
        assert_eq!(num("POWER(5,2)"), 25.0);
        close("POWER(98.6,3.2)", 2401077.2220695773);
        close("POWER(4,5/4)", 5.656854249492381);
        assert_eq!(num("POWER(2,-2)"), 0.25);
        assert_eq!(num("POWER(-2,3)"), -8.0);
        assert_eq!(err("POWER(0,0)"), ErrorKind::Num);
        assert_eq!(err("POWER(0,-1)"), ErrorKind::Div0);
        assert_eq!(err("POWER(-8,1/3)"), ErrorKind::Num);
        assert_eq!(err("POWER(10,400)"), ErrorKind::Num);
        close("EXP(1)", std::f64::consts::E);
        close("EXP(2)", 7.38905609893065);
        assert_eq!(err("EXP(1000)"), ErrorKind::Num);
        close("LN(86)", 4.454347296253507);
        assert_eq!(num("LN(1)"), 0.0);
        assert_eq!(err("LN(0)"), ErrorKind::Num);
        assert_eq!(err("LN(-1)"), ErrorKind::Num);
        assert_eq!(num("LOG(10)"), 1.0);
        assert_eq!(num("LOG(1000)"), 3.0);
        assert_eq!(num("LOG(8,2)"), 3.0);
        assert_eq!(num("LOG(125,5)"), 3.0);
        assert_eq!(num("LOG(243,3)"), 5.0);
        close("LOG(86,2.7182818)", 4.454347342888286);
        assert_eq!(err("LOG(0)"), ErrorKind::Num);
        assert_eq!(err("LOG(10,-2)"), ErrorKind::Num);
        assert_eq!(err("LOG(10,1)"), ErrorKind::Div0);
        close("LOG10(86)", 1.9344984512435677);
        assert_eq!(num("LOG10(100000)"), 5.0);
        assert_eq!(err("LOG10(0)"), ErrorKind::Num);
        assert_eq!(num("PI()"), std::f64::consts::PI);
    }

    #[test]
    fn round() {
        assert_eq!(num("ROUND(2.15,1)"), 2.2);
        assert_eq!(num("ROUND(2.149,1)"), 2.1);
        assert_eq!(num("ROUND(-1.475,2)"), -1.48);
        assert_eq!(num("ROUND(21.5,-1)"), 20.0);
        assert_eq!(num("ROUND(626.3,-3)"), 1000.0);
        assert_eq!(num("ROUND(1.98,-1)"), 0.0);
        assert_eq!(num("ROUND(-50.55,-2)"), -100.0);
        // Halves round away from zero, whatever their binary form.
        assert_eq!(num("ROUND(2.675,2)"), 2.68);
        assert_eq!(num("ROUND(1.005,2)"), 1.01);
        assert_eq!(num("ROUND(0.285,2)"), 0.29);
        assert_eq!(num("ROUND(2.5,0)"), 3.0);
        assert_eq!(num("ROUND(-2.5,0)"), -3.0);
        assert_eq!(num("ROUND(0.5,0)"), 1.0);
        assert_eq!(num("ROUND(-0.5,0)"), -1.0);
        assert_eq!(num("ROUND(0.4,0)"), 0.0);
        assert_eq!(num("ROUND(1234.5678,-2)"), 1200.0);
        assert_eq!(num("ROUND(5,-1)"), 10.0);
        assert_eq!(num("ROUND(4.9,-1)"), 0.0);
        assert_eq!(num("ROUND(-5,-1)"), -10.0);
        assert_eq!(num("ROUND(1.23456,3.9)"), 1.235);
        assert_eq!(num("ROUND(0.1+0.2,15)"), 0.3);
        assert_eq!(num("ROUND(1.5,)"), 2.0);
        assert_eq!(num("ROUND(\"2.5\",0)"), 3.0);
        assert_eq!(err("ROUND(\"x\",0)"), ErrorKind::Value);
        assert_eq!(num("SUM(ROUND({1.4,1.5,2.5},0))"), 6.0);
    }

    #[test]
    fn roundup_rounddown() {
        assert_eq!(num("ROUNDUP(3.2,0)"), 4.0);
        assert_eq!(num("ROUNDUP(76.9,0)"), 77.0);
        assert_eq!(num("ROUNDUP(5.14159,3)"), 5.142);
        assert_eq!(num("ROUNDUP(-3.14159,1)"), -3.2);
        assert_eq!(num("ROUNDUP(31415.92654,-2)"), 31500.0);
        assert_eq!(num("ROUNDUP(3,0)"), 3.0);
        assert_eq!(num("ROUNDUP(0.1+0.2,1)"), 0.3);
        assert_eq!(num("ROUNDUP(0.001,0)"), 1.0);
        assert_eq!(num("ROUNDUP(-0.001,0)"), -1.0);
        assert_eq!(num("ROUNDUP(12,-3)"), 1000.0);
        assert_eq!(num("ROUNDDOWN(3.2,0)"), 3.0);
        assert_eq!(num("ROUNDDOWN(76.9,0)"), 76.0);
        assert_eq!(num("ROUNDDOWN(5.14159,3)"), 5.141);
        assert_eq!(num("ROUNDDOWN(-3.14159,1)"), -3.1);
        assert_eq!(num("ROUNDDOWN(31415.92654,-2)"), 31400.0);
        assert_eq!(num("ROUNDDOWN(0.999,0)"), 0.0);
        assert_eq!(num("ROUNDDOWN(99,-3)"), 0.0);
    }

    #[test]
    fn mround_int_trunc() {
        assert_eq!(num("MROUND(10,3)"), 9.0);
        assert_eq!(num("MROUND(-10,-3)"), -9.0);
        assert_eq!(num("MROUND(1.3,0.2)"), 1.4);
        assert_eq!(num("MROUND(7.5,5)"), 10.0);
        assert_eq!(num("MROUND(12.5,5)"), 15.0);
        assert_eq!(num("MROUND(7,0)"), 0.0);
        assert_eq!(err("MROUND(5,-2)"), ErrorKind::Num);
        assert_eq!(num("INT(8.9)"), 8.0);
        assert_eq!(num("INT(-8.9)"), -9.0);
        assert_eq!(num("INT(5)"), 5.0);
        assert_eq!(num("TRUNC(8.9)"), 8.0);
        assert_eq!(num("TRUNC(-8.9)"), -8.0);
        assert_eq!(num("TRUNC(0.45)"), 0.0);
        assert_eq!(num("TRUNC(5.14159,2)"), 5.14);
        assert_eq!(num("TRUNC(-5.14159,2)"), -5.14);
        assert_eq!(num("TRUNC(1234,-2)"), 1200.0);
    }

    #[test]
    fn ceiling_floor() {
        assert_eq!(num("CEILING(2.5,1)"), 3.0);
        assert_eq!(num("CEILING(-2.5,-2)"), -4.0);
        assert_eq!(num("CEILING(-2.5,2)"), -2.0);
        assert_eq!(num("CEILING(1.5,0.1)"), 1.5);
        assert_eq!(num("CEILING(0.234,0.01)"), 0.24);
        assert_eq!(num("CEILING(4.42,0.05)"), 4.45);
        assert_eq!(num("CEILING(7,7)"), 7.0);
        assert_eq!(num("CEILING(0,5)"), 0.0);
        assert_eq!(num("CEILING(3,0)"), 0.0);
        assert_eq!(err("CEILING(2.5,-2)"), ErrorKind::Num);
        assert_eq!(num("FLOOR(3.7,2)"), 2.0);
        assert_eq!(num("FLOOR(-2.5,-2)"), -2.0);
        assert_eq!(num("FLOOR(-2.5,2)"), -4.0);
        assert_eq!(num("FLOOR(1.58,0.1)"), 1.5);
        assert_eq!(num("FLOOR(0.234,0.01)"), 0.23);
        assert_eq!(num("FLOOR(0.7,0.1)"), 0.7);
        assert_eq!(num("FLOOR(1234567890123456,1)"), 1234567890123456.0);
        assert_eq!(num("CEILING(1E+20,3)"), 1e20);
        assert_eq!(num("FLOOR(0,0)"), 0.0);
        assert_eq!(err("FLOOR(2.5,-2)"), ErrorKind::Num);
        assert_eq!(err("FLOOR(5,0)"), ErrorKind::Div0);
    }

    #[test]
    fn ceiling_floor_variants() {
        assert_eq!(num("CEILING.MATH(24.3,5)"), 25.0);
        assert_eq!(num("CEILING.MATH(6.7)"), 7.0);
        assert_eq!(num("CEILING.MATH(-8.1,2)"), -8.0);
        assert_eq!(num("CEILING.MATH(-5.5,2,-1)"), -6.0);
        assert_eq!(num("CEILING.MATH(5.5,-2)"), 6.0);
        assert_eq!(num("CEILING.MATH(5.5,0)"), 0.0);
        assert_eq!(num("FLOOR.MATH(24.3,5)"), 20.0);
        assert_eq!(num("FLOOR.MATH(6.7)"), 6.0);
        assert_eq!(num("FLOOR.MATH(-8.1,2)"), -10.0);
        assert_eq!(num("FLOOR.MATH(-5.5,2,-1)"), -4.0);
        assert_eq!(num("CEILING.PRECISE(4.3)"), 5.0);
        assert_eq!(num("CEILING.PRECISE(-4.3)"), -4.0);
        assert_eq!(num("CEILING.PRECISE(4.3,2)"), 6.0);
        assert_eq!(num("CEILING.PRECISE(4.3,-2)"), 6.0);
        assert_eq!(num("CEILING.PRECISE(-4.3,2)"), -4.0);
        assert_eq!(num("CEILING.PRECISE(-4.3,-2)"), -4.0);
        assert_eq!(num("ISO.CEILING(4.3)"), 5.0);
        assert_eq!(num("ISO.CEILING(-4.3)"), -4.0);
        assert_eq!(num("ISO.CEILING(4.3,2)"), 6.0);
        assert_eq!(num("ISO.CEILING(4.3,0)"), 0.0);
        assert_eq!(num("FLOOR.PRECISE(-3.2,-1)"), -4.0);
        assert_eq!(num("FLOOR.PRECISE(3.2,1)"), 3.0);
        assert_eq!(num("FLOOR.PRECISE(-3.2,1)"), -4.0);
        assert_eq!(num("FLOOR.PRECISE(3.2,-1)"), 3.0);
        assert_eq!(num("FLOOR.PRECISE(3.2)"), 3.0);
        assert_eq!(num("FLOOR.PRECISE(3.2,0)"), 0.0);
        assert_eq!(num("EVEN(1.5)"), 2.0);
        assert_eq!(num("EVEN(3)"), 4.0);
        assert_eq!(num("EVEN(2)"), 2.0);
        assert_eq!(num("EVEN(-1)"), -2.0);
        assert_eq!(num("EVEN(0)"), 0.0);
        assert_eq!(num("ODD(1.5)"), 3.0);
        assert_eq!(num("ODD(3)"), 3.0);
        assert_eq!(num("ODD(2)"), 3.0);
        assert_eq!(num("ODD(-1)"), -1.0);
        assert_eq!(num("ODD(-2)"), -3.0);
        assert_eq!(num("ODD(0)"), 1.0);
    }

    #[test]
    fn random() {
        for _ in 0..50 {
            let r = num("RAND()");
            assert!((0.0..1.0).contains(&r));
            let d = num("RANDBETWEEN(1,6)");
            assert!((1.0..=6.0).contains(&d) && d.fract() == 0.0);
            let d = num("RANDBETWEEN(-2.5,2.5)");
            assert!((-2.0..=2.0).contains(&d) && d.fract() == 0.0);
        }
        assert_eq!(num("RANDBETWEEN(7,7)"), 7.0);
        assert_eq!(err("RANDBETWEEN(5,1)"), ErrorKind::Num);
        assert_ne!(num("RAND()"), num("RAND()"));
        let a = spill("RANDARRAY(3,2)", 3, 2);
        assert!(a.iter().flatten().all(|x| (0.0..1.0).contains(x)));
        let a = spill("RANDARRAY(4,4,10,20,TRUE)", 4, 4);
        assert!(a.iter().flatten().all(|x| (10.0..=20.0).contains(x) && x.fract() == 0.0));
        let a = spill("RANDARRAY(2,2,5,6)", 2, 2);
        assert!(a.iter().flatten().all(|x| (5.0..6.0).contains(x)));
        let r = num("RANDARRAY()");
        assert!((0.0..1.0).contains(&r));
        assert_eq!(err("RANDARRAY(0)"), ErrorKind::Calc);
        assert_eq!(err("RANDARRAY(-1)"), ErrorKind::Value);
        assert_eq!(err("RANDARRAY(2,2,5,1)"), ErrorKind::Value);
        assert_eq!(err("RANDARRAY(2,2,1.5,3,TRUE)"), ErrorKind::Value);
    }

    #[test]
    fn trigonometry() {
        close("SIN(PI()/2)", 1.0);
        close("SIN(30*PI()/180)", 0.5);
        assert_eq!(num("SIN(0)"), 0.0);
        assert_eq!(num("COS(0)"), 1.0);
        close("COS(1.047)", 0.5001710745970701);
        close("TAN(0.785)", 0.9992039901050427);
        close("ASIN(-0.5)", -0.5235987755982988);
        assert_eq!(err("ASIN(2)"), ErrorKind::Num);
        close("ACOS(-0.5)", 2.0943951023931957);
        assert_eq!(err("ACOS(-1.1)"), ErrorKind::Num);
        close("ATAN(1)", std::f64::consts::FRAC_PI_4);
        // ATAN2 takes x first.
        close("ATAN2(1,1)", std::f64::consts::FRAC_PI_4);
        close("ATAN2(-1,-1)", -2.356194490192345);
        close("ATAN2(0,1)", std::f64::consts::FRAC_PI_2);
        close("ATAN2(-1,0)", std::f64::consts::PI);
        assert_eq!(err("ATAN2(0,0)"), ErrorKind::Div0);
        close("SINH(1)", 1.1752011936438014);
        close("COSH(4)", 27.308232836016487);
        close("TANH(-2)", -0.9640275800758169);
        close("ASINH(-2.5)", -1.6472311463710958);
        assert_eq!(num("ACOSH(1)"), 0.0);
        close("ACOSH(10)", 2.993222846126381);
        assert_eq!(err("ACOSH(0.5)"), ErrorKind::Num);
        close("ATANH(-0.1)", -0.10033534773107558);
        assert_eq!(err("ATANH(1)"), ErrorKind::Num);
        assert_eq!(err("ATANH(-1)"), ErrorKind::Num);
        close("COT(30)", -0.1561199521616592);
        close("COT(45)", 0.6173696237835551);
        assert_eq!(err("COT(0)"), ErrorKind::Div0);
        assert_eq!(err("COT(2^27)"), ErrorKind::Num);
        close("SEC(45)", 1.9035944074044246);
        close("SEC(30)", 6.482921234962678);
        assert_eq!(num("SEC(0)"), 1.0);
        close("CSC(15)", 1.5377805615408537);
        assert_eq!(err("CSC(0)"), ErrorKind::Div0);
        close("ACOT(2)", 0.4636476090008059);
        close("ACOT(-1)", 2.356194490192345);
        close("ACOT(0)", std::f64::consts::FRAC_PI_2);
        close("DEGREES(PI())", 180.0);
        close("RADIANS(270)", 4.71238898038469);
        close("RADIANS(180)", std::f64::consts::PI);
    }

    #[test]
    fn combinatorics() {
        assert_eq!(num("FACT(5)"), 120.0);
        assert_eq!(num("FACT(1.9)"), 1.0);
        assert_eq!(num("FACT(0)"), 1.0);
        assert_eq!(num("FACT(20)"), 2432902008176640000.0);
        close("FACT(170)", 7.257415615307999e306);
        assert_eq!(err("FACT(-1)"), ErrorKind::Num);
        assert_eq!(err("FACT(171)"), ErrorKind::Num);
        assert_eq!(num("FACTDOUBLE(6)"), 48.0);
        assert_eq!(num("FACTDOUBLE(7)"), 105.0);
        assert_eq!(num("FACTDOUBLE(0)"), 1.0);
        assert_eq!(num("FACTDOUBLE(-1)"), 1.0);
        assert_eq!(err("FACTDOUBLE(-2)"), ErrorKind::Num);
        assert_eq!(num("COMBIN(8,2)"), 28.0);
        assert_eq!(num("COMBIN(8.9,2.9)"), 28.0);
        assert_eq!(num("COMBIN(5,0)"), 1.0);
        assert_eq!(num("COMBIN(5,5)"), 1.0);
        assert_eq!(num("COMBIN(52,5)"), 2598960.0);
        assert_eq!(num("COMBIN(60,30)"), 118264581564861424.0);
        assert_eq!(err("COMBIN(2,3)"), ErrorKind::Num);
        assert_eq!(err("COMBIN(-1,1)"), ErrorKind::Num);
        assert_eq!(num("COMBINA(4,3)"), 20.0);
        assert_eq!(num("COMBINA(10,3)"), 220.0);
        assert_eq!(num("COMBINA(0,0)"), 1.0);
        assert_eq!(err("COMBINA(-1,2)"), ErrorKind::Num);
        assert_eq!(num("PERMUT(100,3)"), 970200.0);
        assert_eq!(num("PERMUT(3,2)"), 6.0);
        assert_eq!(num("PERMUT(5,0)"), 1.0);
        assert_eq!(err("PERMUT(2,3)"), ErrorKind::Num);
        assert_eq!(err("PERMUT(-2,1)"), ErrorKind::Num);
    }

    #[test]
    fn gcd_lcm_multinomial() {
        assert_eq!(num("GCD(5,2)"), 1.0);
        assert_eq!(num("GCD(24,36)"), 12.0);
        assert_eq!(num("GCD(7,1)"), 1.0);
        assert_eq!(num("GCD(5,0)"), 5.0);
        assert_eq!(num("GCD(0,0)"), 0.0);
        assert_eq!(num("GCD(7.9,14)"), 7.0);
        assert_eq!(num("GCD(\"12\",18)"), 6.0);
        assert_eq!(num("GCD({12,18},30)"), 6.0);
        assert_eq!(err("GCD(-5,2)"), ErrorKind::Num);
        assert_eq!(err("GCD(TRUE,2)"), ErrorKind::Value);
        assert_eq!(err("GCD(\"x\",2)"), ErrorKind::Value);
        assert_eq!(num("LCM(5,2)"), 10.0);
        assert_eq!(num("LCM(24,36)"), 72.0);
        assert_eq!(num("LCM(0,5)"), 0.0);
        assert_eq!(num("LCM(3,4,6)"), 12.0);
        assert_eq!(err("LCM(-1,2)"), ErrorKind::Num);
        assert_eq!(num("MULTINOMIAL(2,3,4)"), 1260.0);
        assert_eq!(num("MULTINOMIAL(3)"), 1.0);
        assert_eq!(num("MULTINOMIAL({1,2},3)"), 60.0);
        assert_eq!(err("MULTINOMIAL(2,-1)"), ErrorKind::Num);
        let w = wb(&[("A1", "12"), ("A2", "18"), ("A4", "x"), ("B1", "=GCD(A1:A3)"), ("B2", "=LCM(A1:A3)"), ("B3", "=GCD(A1:A4)")]);
        assert_eq!(cell(&w, "B1"), 6.0);
        assert_eq!(cell(&w, "B2"), 36.0);
        assert_eq!(val(&w, "B3"), Value::Error(ErrorKind::Value));
    }

    #[test]
    fn subtotal() {
        let mut w = wb(&[
            ("A1", "1"),
            ("A2", "2"),
            ("A3", "3"),
            ("A4", "4"),
            ("A5", "x"),
            ("A6", "=SUBTOTAL(9,A1:A5)"),
            ("B1", "=SUBTOTAL(9,A1:A6)"),
            ("B2", "=SUBTOTAL(1,A1:A5)"),
            ("B3", "=SUBTOTAL(2,A1:A5)"),
            ("B4", "=SUBTOTAL(3,A1:A5)"),
            ("B5", "=SUBTOTAL(4,A1:A5)"),
            ("B6", "=SUBTOTAL(5,A1:A5)"),
            ("B7", "=SUBTOTAL(6,A1:A5)"),
            ("B8", "=SUBTOTAL(7,A1:A5)"),
            ("B9", "=SUBTOTAL(8,A1:A5)"),
            ("B10", "=SUBTOTAL(10,A1:A5)"),
            ("B11", "=SUBTOTAL(11,A1:A5)"),
            ("C1", "=SUBTOTAL(109,A1:A5)"),
            ("C2", "=SUBTOTAL(103,A1:A5)"),
            ("C3", "=SUBTOTAL(101,A1:A5)"),
            ("C4", "=SUBTOTAL(9,A1:A2,A3:A6)"),
            ("C5", "=SUBTOTAL(12,A1:A5)"),
            ("C6", "=SUBTOTAL(0,A1:A5)"),
            ("C7", "=SUM(A1:A6)"),
        ]);
        assert_eq!(cell(&w, "A6"), 10.0);
        // The nested SUBTOTAL in A6 is not counted twice.
        assert_eq!(cell(&w, "B1"), 10.0);
        assert_eq!(cell(&w, "C7"), 20.0);
        assert_eq!(cell(&w, "B2"), 2.5);
        assert_eq!(cell(&w, "B3"), 4.0);
        assert_eq!(cell(&w, "B4"), 5.0);
        assert_eq!(cell(&w, "B5"), 4.0);
        assert_eq!(cell(&w, "B6"), 1.0);
        assert_eq!(cell(&w, "B7"), 24.0);
        assert!((cell(&w, "B8") - 1.2909944487358056).abs() < 1e-12);
        assert!((cell(&w, "B9") - 1.118033988749895).abs() < 1e-12);
        assert!((cell(&w, "B10") - 5.0 / 3.0).abs() < 1e-12);
        assert_eq!(cell(&w, "B11"), 1.25);
        assert_eq!(cell(&w, "C1"), 10.0);
        assert_eq!(cell(&w, "C4"), 10.0);
        assert_eq!(val(&w, "C5"), Value::Error(ErrorKind::Value));
        assert_eq!(val(&w, "C6"), Value::Error(ErrorKind::Value));
        // Hide row 2: only the 101-111 variants skip it.
        w.sheets[0].hidden_rows.insert(1);
        w.recalc();
        assert_eq!(cell(&w, "A6"), 10.0);
        assert_eq!(cell(&w, "C1"), 8.0);
        assert_eq!(cell(&w, "C2"), 4.0);
        assert_eq!(cell(&w, "C3"), 8.0 / 3.0);
        // A filter hides row 3: every variant skips it, plain functions
        // do not.
        w.sheets[0].filtered_rows.insert(2);
        w.recalc();
        assert_eq!(cell(&w, "A6"), 7.0);
        assert_eq!(cell(&w, "B4"), 4.0);
        assert_eq!(cell(&w, "C1"), 5.0);
        assert_eq!(cell(&w, "C7"), 17.0);
    }

    #[test]
    fn aggregate() {
        let mut w = wb(&[
            ("A1", "5"),
            ("A2", "=1/0"),
            ("A3", "3"),
            ("A4", "8"),
            ("A5", "#N/A"),
            ("B1", "=AGGREGATE(9,6,A1:A5)"),
            ("B2", "=AGGREGATE(9,4,A1:A5)"),
            ("B3", "=AGGREGATE(14,6,A1:A5,2)"),
            ("B4", "=AGGREGATE(15,6,A1:A5,1)"),
            ("B5", "=AGGREGATE(1,6,A1:A5)"),
            ("B6", "=AGGREGATE(4,6,A1:A5)"),
            ("B7", "=AGGREGATE(5,6,A1:A5)"),
            ("B8", "=AGGREGATE(12,6,A1:A5)"),
            ("B9", "=AGGREGATE(16,6,A1:A5,0.5)"),
            ("B10", "=AGGREGATE(17,6,A1:A5,1)"),
            ("B11", "=AGGREGATE(2,6,A1:A5)"),
            ("B12", "=AGGREGATE(3,6,A1:A5)"),
            ("B13", "=AGGREGATE(3,4,A1:A5)"),
            ("B14", "=AGGREGATE(14,6,A1:A5*2,1)"),
            ("B15", "=AGGREGATE(20,6,A1:A5)"),
            ("B16", "=AGGREGATE(9,8,A1:A5)"),
            ("B17", "=AGGREGATE(14,6,A1:A5)"),
            ("B18", "=AGGREGATE(6,6,A1:A5)"),
            ("B19", "=AGGREGATE(9,5,A1,A3:A4)"),
            ("B20", "=AGGREGATE(9,7,A1:A5)"),
            ("B21", "=AGGREGATE(14,4,A1:A5,1)"),
            ("B22", "=AGGREGATE(9,,A1,A3:A4)"),
            ("B23", "=AGGREGATE(7,6,A1:A5)"),
            ("B24", "=AGGREGATE(13,6,A1:A5)"),
            ("B27", "=AGGREGATE(13,6,A1:A5,A3)"),
            ("B25", "=AGGREGATE(18,6,A1:A5,0.5)"),
            ("B26", "=AGGREGATE(19,6,A1:A5,2)"),
            ("D1", "=AGGREGATE(14,6,A1:A5,{1,2})"),
        ]);
        assert_eq!(cell(&w, "B1"), 16.0);
        assert_eq!(val(&w, "B2"), Value::Error(ErrorKind::Div0));
        assert_eq!(cell(&w, "B3"), 5.0);
        assert_eq!(cell(&w, "B4"), 3.0);
        assert!((cell(&w, "B5") - 16.0 / 3.0).abs() < 1e-12);
        assert_eq!(cell(&w, "B6"), 8.0);
        assert_eq!(cell(&w, "B7"), 3.0);
        assert_eq!(cell(&w, "B8"), 5.0);
        assert_eq!(cell(&w, "B9"), 5.0);
        assert_eq!(cell(&w, "B10"), 4.0);
        assert_eq!(cell(&w, "B11"), 3.0);
        assert_eq!(cell(&w, "B12"), 3.0);
        assert_eq!(cell(&w, "B13"), 5.0);
        assert_eq!(cell(&w, "B14"), 16.0);
        assert_eq!(val(&w, "B15"), Value::Error(ErrorKind::Value));
        assert_eq!(val(&w, "B16"), Value::Error(ErrorKind::Value));
        assert_eq!(val(&w, "B17"), Value::Error(ErrorKind::Value));
        assert_eq!(cell(&w, "B18"), 120.0);
        assert_eq!(cell(&w, "B19"), 16.0);
        assert_eq!(cell(&w, "B20"), 16.0);
        assert_eq!(val(&w, "B21"), Value::Error(ErrorKind::Div0));
        assert_eq!(cell(&w, "B22"), 16.0);
        assert!((cell(&w, "B23") - 2.516611478423583).abs() < 1e-12);
        assert_eq!(val(&w, "B24"), Value::Error(ErrorKind::NA));
        assert_eq!(cell(&w, "B27"), 3.0);
        assert_eq!(cell(&w, "B25"), 5.0);
        assert_eq!(cell(&w, "B26"), 5.0);
        assert_eq!(cell(&w, "D1"), 8.0);
        assert_eq!(cell(&w, "E1"), 5.0);
        // Options 5 and 7 skip hidden rows.
        w.sheets[0].hidden_rows.insert(3);
        w.recalc();
        assert_eq!(cell(&w, "B1"), 16.0);
        assert_eq!(cell(&w, "B19"), 8.0);
        assert_eq!(cell(&w, "B20"), 8.0);
        // They also skip the rows a filter hides; the other options keep
        // them.
        w.sheets[0].filtered_rows.insert(2);
        w.recalc();
        assert_eq!(cell(&w, "B1"), 16.0);
        assert_eq!(cell(&w, "B19"), 5.0);
        assert_eq!(cell(&w, "B20"), 5.0);
    }

    #[test]
    fn roman_arabic() {
        assert_eq!(text("ROMAN(499,0)"), "CDXCIX");
        assert_eq!(text("ROMAN(499,1)"), "LDVLIV");
        assert_eq!(text("ROMAN(499,2)"), "XDIX");
        assert_eq!(text("ROMAN(499,3)"), "VDIV");
        assert_eq!(text("ROMAN(499,4)"), "ID");
        assert_eq!(text("ROMAN(499)"), "CDXCIX");
        assert_eq!(text("ROMAN(499,TRUE)"), "CDXCIX");
        assert_eq!(text("ROMAN(499,FALSE)"), "ID");
        assert_eq!(text("ROMAN(2013)"), "MMXIII");
        assert_eq!(text("ROMAN(1999)"), "MCMXCIX");
        assert_eq!(text("ROMAN(1999,4)"), "MIM");
        assert_eq!(text("ROMAN(3999)"), "MMMCMXCIX");
        assert_eq!(text("ROMAN(14)"), "XIV");
        assert_eq!(text("ROMAN(0)"), "");
        assert_eq!(err("ROMAN(4000)"), ErrorKind::Value);
        assert_eq!(err("ROMAN(-1)"), ErrorKind::Value);
        assert_eq!(err("ROMAN(10,5)"), ErrorKind::Value);
        assert_eq!(num("ARABIC(\"LVII\")"), 57.0);
        assert_eq!(num("ARABIC(\"mcmxii\")"), 1912.0);
        assert_eq!(num("ARABIC(\"\")"), 0.0);
        assert_eq!(num("ARABIC(\"-X\")"), -10.0);
        assert_eq!(num("ARABIC(\" MMXIII \")"), 2013.0);
        assert_eq!(num("ARABIC(\"ID\")"), 499.0);
        assert_eq!(num("ARABIC(ROMAN(1987))"), 1987.0);
        assert_eq!(err("ARABIC(\"ABC\")"), ErrorKind::Value);
        assert_eq!(err("ARABIC(12)"), ErrorKind::Value);
    }

    #[test]
    fn base_decimal() {
        assert_eq!(text("BASE(7,2)"), "111");
        assert_eq!(text("BASE(100,16)"), "64");
        assert_eq!(text("BASE(15,2,10)"), "0000001111");
        assert_eq!(text("BASE(255,16)"), "FF");
        assert_eq!(text("BASE(0,2)"), "0");
        assert_eq!(text("BASE(35,36)"), "Z");
        assert_eq!(err("BASE(-1,2)"), ErrorKind::Num);
        assert_eq!(err("BASE(10,1)"), ErrorKind::Num);
        assert_eq!(err("BASE(10,37)"), ErrorKind::Num);
        assert_eq!(err("BASE(10,2,256)"), ErrorKind::Num);
        assert_eq!(num("DECIMAL(\"FF\",16)"), 255.0);
        assert_eq!(num("DECIMAL(111,2)"), 7.0);
        assert_eq!(num("DECIMAL(\"zap\",36)"), 45745.0);
        assert_eq!(num("DECIMAL(\"ff\",16)"), 255.0);
        assert_eq!(err("DECIMAL(\"12\",2)"), ErrorKind::Num);
        assert_eq!(err("DECIMAL(\"1\",40)"), ErrorKind::Num);
    }

    #[test]
    fn sequence() {
        let a = spill("SEQUENCE(4,5)", 4, 5);
        assert_eq!(a[0], vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(a[3], vec![16.0, 17.0, 18.0, 19.0, 20.0]);
        assert_eq!(spill("SEQUENCE(2,2,10,5)", 2, 2), vec![vec![10.0, 15.0], vec![20.0, 25.0]]);
        assert_eq!(spill("SEQUENCE(3)", 3, 1), vec![vec![1.0], vec![2.0], vec![3.0]]);
        assert_eq!(spill("SEQUENCE(1,3,0,-0.5)", 1, 3), vec![vec![0.0, -0.5, -1.0]]);
        assert_eq!(num("SUM(SEQUENCE(10))"), 55.0);
        assert_eq!(num("SEQUENCE(1)"), 1.0);
        assert_eq!(err("SEQUENCE(0)"), ErrorKind::Calc);
        assert_eq!(err("SEQUENCE(-1)"), ErrorKind::Value);
        assert_eq!(err("SEQUENCE(\"x\")"), ErrorKind::Value);
    }

    #[test]
    fn matrices() {
        assert_eq!(spill("MMULT({1,3;7,2},{2,0;0,2})", 2, 2), vec![vec![2.0, 6.0], vec![14.0, 4.0]]);
        assert_eq!(spill("MMULT({1,2,3},{4;5;6})", 1, 1), vec![vec![32.0]]);
        assert_eq!(spill("MMULT({1;2},{3,4})", 2, 2), vec![vec![3.0, 4.0], vec![6.0, 8.0]]);
        assert_eq!(err("MMULT({1,2},{1,2})"), ErrorKind::Value);
        assert_eq!(err("MMULT({1,\"a\"},{1;2})"), ErrorKind::Value);
        assert_eq!(spill("MINVERSE({4,-1;2,0})", 2, 2), vec![vec![0.0, 0.5], vec![-1.0, 2.0]]);
        let inv = spill("MINVERSE({1,2,1;3,4,-1;0,2,0})", 3, 3);
        let expected = [[0.25, 0.25, -0.75], [0.0, 0.0, 0.5], [0.75, -0.25, -0.25]];
        for r in 0..3 {
            for c in 0..3 {
                assert!((inv[r][c] - expected[r][c]).abs() < 1e-12, "{inv:?}");
            }
        }
        assert_eq!(num("MINVERSE(4)"), 0.25);
        assert_eq!(err("MINVERSE({1,2;2,4})"), ErrorKind::Num);
        assert_eq!(err("MINVERSE({1,2,3;4,5,6;7,8,9})"), ErrorKind::Num);
        assert_eq!(err("MINVERSE({1,2,3})"), ErrorKind::Value);
        close("SUM(MMULT({2,1;5,3},MINVERSE({2,1;5,3})))", 2.0);
        assert_eq!(num("MDETERM({3,6,1;1,1,0;3,10,2})"), 1.0);
        assert_eq!(num("MDETERM({3,6;1,1})"), -3.0);
        assert_eq!(num("MDETERM({1,3,8,5;1,3,6,1;1,1,1,0;7,3,10,2})"), 88.0);
        assert_eq!(num("MDETERM({1,2,3;4,5,6;7,8,9})"), 0.0);
        close("MDETERM({0.5,1.5;2,0.25})", -2.875);
        assert_eq!(num("MDETERM(5)"), 5.0);
        assert_eq!(err("MDETERM({1,2,3;4,5,6})"), ErrorKind::Value);
        assert_eq!(
            spill("MUNIT(3)", 3, 3),
            vec![vec![1.0, 0.0, 0.0], vec![0.0, 1.0, 0.0], vec![0.0, 0.0, 1.0]]
        );
        assert_eq!(num("MUNIT(1)"), 1.0);
        assert_eq!(err("MUNIT(0)"), ErrorKind::Value);
        let w = wb(&[
            ("A1", "1"),
            ("B1", "2"),
            ("A2", "3"),
            ("B2", "4"),
            ("D1", "=MMULT(A1:B2,A1:B2)"),
            ("G1", "=MDETERM(A1:B2)"),
        ]);
        assert_eq!(cell(&w, "D1"), 7.0);
        assert_eq!(cell(&w, "E1"), 10.0);
        assert_eq!(cell(&w, "D2"), 15.0);
        assert_eq!(cell(&w, "E2"), 22.0);
        assert_eq!(cell(&w, "G1"), -2.0);
    }

    #[test]
    fn seriessum() {
        let w = wb(&[
            ("A1", "1"),
            ("A2", "=-1/FACT(2)"),
            ("A3", "=1/FACT(4)"),
            ("A4", "=-1/FACT(6)"),
            ("B1", "=SERIESSUM(PI()/4,0,2,A1:A4)"),
        ]);
        assert!((cell(&w, "B1") - 0.7071032148228457).abs() < 1e-12);
        assert_eq!(num("SERIESSUM(2,1,1,{1,1,1})"), 14.0);
        assert_eq!(num("SERIESSUM(3,0,1,5)"), 5.0);
        assert_eq!(err("SERIESSUM(2,1,1,{1,\"a\"})"), ErrorKind::Value);
    }
}
