//! Statistical functions.

use super::criteria::{
    Grid, if_function, ifs_args, ifs_function, lift, matrix, opt_number, scan, with_criteria,
};
use super::math::{Rounding, combin, factorial, number, round_to, snap15};
use super::{Category, FunctionDef, Kind, VAR};
use crate::ast::Expr;
use crate::eval::{EvalCtx, is_reference};
use crate::value::{Array, ErrorKind, R, Value};
use std::collections::HashMap;
use std::f64::consts::PI;

// ---- numeric kernels (also used by SUBTOTAL / AGGREGATE) ----------------------

pub(super) fn mean(xs: &[f64]) -> f64 {
    xs.iter().sum::<f64>() / xs.len() as f64
}

/// Sum of squared deviations from the mean (exactly 0 for constant data).
pub(super) fn devsq(xs: &[f64]) -> f64 {
    let Some(first) = xs.first() else {
        return 0.0;
    };
    if xs.iter().all(|x| x == first) {
        return 0.0;
    }
    let m = mean(xs);
    xs.iter().map(|x| (x - m) * (x - m)).sum()
}

/// Variance; `#DIV/0!` when there are too few values.
pub(super) fn variance(xs: &[f64], sample: bool) -> R<f64> {
    let n = xs.len();
    let d = if sample { n.saturating_sub(1) } else { n };
    if d == 0 {
        return Err(ErrorKind::Div0);
    }
    Ok(devsq(xs) / d as f64)
}

/// Largest or smallest value; 0 when there is none.
pub(super) fn extreme(xs: &[f64], max: bool) -> f64 {
    xs.iter()
        .copied()
        .reduce(|a, b| if max { a.max(b) } else { a.min(b) })
        .unwrap_or(0.0)
}

/// Median (sorts the slice); `#NUM!` when empty.
pub(super) fn median(xs: &mut [f64]) -> R<f64> {
    if xs.is_empty() {
        return Err(ErrorKind::Num);
    }
    xs.sort_by(f64::total_cmp);
    let n = xs.len();
    Ok(if n % 2 == 1 {
        xs[n / 2]
    } else {
        (xs[n / 2 - 1] + xs[n / 2]) / 2.0
    })
}

/// The most frequent values (those occurring at least twice), in order of
/// first appearance.
pub(super) fn modes(xs: &[f64]) -> Vec<f64> {
    let mut index: HashMap<u64, usize> = HashMap::new();
    let mut counts: Vec<(f64, usize)> = Vec::new();
    for x in xs {
        let key = (x + 0.0).to_bits();
        match index.get(&key) {
            Some(i) => counts[*i].1 += 1,
            None => {
                index.insert(key, counts.len());
                counts.push((*x, 1));
            }
        }
    }
    let best = counts.iter().map(|c| c.1).max().unwrap_or(0);
    if best < 2 {
        return Vec::new();
    }
    counts
        .into_iter()
        .filter(|c| c.1 == best)
        .map(|c| c.0)
        .collect()
}

/// k-th largest / smallest value of ascending data. `k` is 1-based; a
/// fractional `k` is rounded up.
pub(super) fn kth(sorted: &[f64], k: f64, largest: bool) -> R<f64> {
    let k = k.ceil();
    if k < 1.0 || k > sorted.len() as f64 {
        return Err(ErrorKind::Num);
    }
    let i = k as usize - 1;
    Ok(if largest {
        sorted[sorted.len() - 1 - i]
    } else {
        sorted[i]
    })
}

/// Percentile of ascending data with linear interpolation, inclusive
/// (`k` in 0..=1) or exclusive (`k` strictly inside, rank `k*(n+1)`).
pub(super) fn percentile(sorted: &[f64], k: f64, exclusive: bool) -> R<f64> {
    let n = sorted.len();
    if n == 0 {
        return Err(ErrorKind::Num);
    }
    let pos = if exclusive {
        if k <= 0.0 || k >= 1.0 {
            return Err(ErrorKind::Num);
        }
        let rank = snap15(k * (n as f64 + 1.0));
        if rank < 1.0 || rank > n as f64 {
            return Err(ErrorKind::Num);
        }
        rank - 1.0
    } else {
        if !(0.0..=1.0).contains(&k) {
            return Err(ErrorKind::Num);
        }
        snap15(k * (n as f64 - 1.0))
    };
    let i = pos.floor() as usize;
    // The fraction keeps only the digits `pos` really has (15 in all), so
    // that 89999.1 leaves 0.1 and not 0.10000000000582.
    let int_digits = if pos >= 1.0 {
        pos.log10().floor() as i32 + 1
    } else {
        0
    };
    let frac = round_to(pos - pos.floor(), 15 - int_digits, Rounding::HalfAway);
    Ok(if frac == 0.0 || i + 1 >= n {
        sorted[i.min(n - 1)]
    } else {
        sorted[i] + frac * (sorted[i + 1] - sorted[i])
    })
}

/// Quartile of ascending data: `q` in 0..=4 (inclusive) or 1..=3.
pub(super) fn quartile(sorted: &[f64], q: f64, exclusive: bool) -> R<f64> {
    let q = q.trunc();
    let valid = if exclusive {
        (1.0..=3.0).contains(&q)
    } else {
        (0.0..=4.0).contains(&q)
    };
    if !valid {
        return Err(ErrorKind::Num);
    }
    percentile(sorted, q / 4.0, exclusive)
}

/// Corresponding numbers of two arrays with the same number of cells
/// (`#N/A` otherwise). A pair is dropped when either side is not a number.
pub(super) fn pairs(ctx: &EvalCtx, a: &Expr, b: &Expr) -> R<Vec<(f64, f64)>> {
    let ga = Grid::eval(ctx, a)?;
    let gb = Grid::eval(ctx, b)?;
    if let Some(e) = ga.first_error().or(gb.first_error()) {
        return Err(e);
    }
    let (a_cols, b_cols) = ga.pairing(&gb).ok_or(ErrorKind::NA)?;
    let mut out = Vec::new();
    for r in 0..ga.values.rows {
        for c in 0..ga.values.cols {
            let Value::Number(x) = ga.get(r, c) else {
                continue;
            };
            // Same position counted row by row in the other array.
            let i = r * a_cols + c;
            if let Value::Number(y) = gb.get(i / b_cols, i % b_cols) {
                out.push((*x, *y));
            }
        }
    }
    Ok(out)
}

// ---- argument collection ------------------------------------------------------

/// Numbers with the semantics of the `...A` functions: inside ranges and
/// arrays text counts as 0 and booleans as 1 / 0; blanks are skipped.
fn numbers_a(ctx: &EvalCtx, args: &[Expr]) -> R<Vec<f64>> {
    let mut out = Vec::new();
    for arg in args {
        match ctx.eval(arg) {
            Value::Array(a) => {
                for v in a.iter() {
                    match v {
                        Value::Number(n) => out.push(*n),
                        Value::Bool(b) => out.push(if *b { 1.0 } else { 0.0 }),
                        Value::Text(_) => out.push(0.0),
                        Value::Error(e) => return Err(*e),
                        _ => {}
                    }
                }
            }
            Value::Empty => {
                if ctx.is_missing(arg) {
                    out.push(0.0);
                }
            }
            Value::Text(_) if is_reference(arg) => out.push(0.0),
            v => out.push(ctx.to_number(&v)?),
        }
    }
    Ok(out)
}

/// Numbers of one array argument, sorted ascending.
fn sorted_numbers(ctx: &EvalCtx, arg: &Expr) -> R<Vec<f64>> {
    let mut nums = ctx.numbers(std::slice::from_ref(arg))?;
    nums.sort_by(f64::total_cmp);
    Ok(nums)
}

fn column(values: Vec<f64>) -> Value {
    Value::array(Array::column(
        values.into_iter().map(Value::Number).collect(),
    ))
}

// ---- averages, counts, extremes ---------------------------------------------------

fn average_of(nums: &[f64]) -> R<Value> {
    if nums.is_empty() {
        return Err(ErrorKind::Div0);
    }
    number(mean(nums))
}

fn f_average(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    average_of(&ctx.numbers(args)?)
}

fn f_averagea(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    average_of(&numbers_a(ctx, args)?)
}

fn f_averageif(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    if_function(ctx, args, average_of)
}

fn f_averageifs(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    ifs_function(ctx, args, average_of)
}

fn f_count(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let mut n = 0usize;
    for arg in args {
        match ctx.eval(arg) {
            // Inside ranges and arrays only real numbers count.
            Value::Array(a) => n += a.iter().filter(|v| v.is_number()).count(),
            Value::Number(_) => n += 1,
            Value::Empty => {
                if ctx.is_missing(arg) {
                    n += 1;
                }
            }
            Value::Error(_) => {}
            // Booleans and numeric text count when typed as arguments.
            v => {
                if !is_reference(arg) && ctx.to_number(&v).is_ok() {
                    n += 1;
                }
            }
        }
    }
    Ok(Value::Number(n as f64))
}

fn f_counta(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let mut n = 0usize;
    for arg in args {
        match ctx.eval(arg) {
            Value::Array(a) => n += a.iter().filter(|v| !v.is_empty()).count(),
            Value::Empty => {
                if ctx.is_missing(arg) {
                    n += 1;
                }
            }
            _ => n += 1,
        }
    }
    Ok(Value::Number(n as f64))
}

fn f_countblank(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let r = ctx.eval_ref(&args[0])?;
    // The values are clamped to the used cells; everything else is blank.
    let filled = ctx
        .range_values(&r)
        .iter()
        .filter(|v| match v {
            Value::Empty => false,
            Value::Text(s) => !s.is_empty(),
            _ => true,
        })
        .count();
    Ok(Value::Number(
        r.rows() as f64 * r.cols() as f64 - filled as f64,
    ))
}

fn count_matches(ranges: &[Grid], criteria: &[super::criteria::Criteria]) -> R<Value> {
    let mut n = 0usize;
    let beyond = scan(ranges, criteria, None, |_, _| n += 1);
    Ok(Value::Number((n + beyond) as f64))
}

fn f_countif(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let ranges = [Grid::eval(ctx, &args[0])?];
    with_criteria(ctx, &[args.get(1)], |criteria| {
        count_matches(&ranges, criteria)
    })
}

fn f_countifs(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let (ranges, criteria) = ifs_args(ctx, args, None)?;
    with_criteria(ctx, &criteria, |criteria| count_matches(&ranges, criteria))
}

fn f_max(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    number(extreme(&ctx.numbers(args)?, true))
}

fn f_min(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    number(extreme(&ctx.numbers(args)?, false))
}

fn f_maxa(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    number(extreme(&numbers_a(ctx, args)?, true))
}

fn f_mina(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    number(extreme(&numbers_a(ctx, args)?, false))
}

fn f_maxifs(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    ifs_function(ctx, args, |nums| number(extreme(nums, true)))
}

fn f_minifs(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    ifs_function(ctx, args, |nums| number(extreme(nums, false)))
}

// ---- central tendency and dispersion ------------------------------------------

fn f_median(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    number(median(&mut ctx.numbers(args)?)?)
}

fn f_mode(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let modes = modes(&ctx.numbers(args)?);
    number(*modes.first().ok_or(ErrorKind::NA)?)
}

fn f_mode_mult(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let modes = modes(&ctx.numbers(args)?);
    if modes.is_empty() {
        return Err(ErrorKind::NA);
    }
    Ok(column(modes))
}

fn spread(ctx: &EvalCtx, args: &[Expr], text_counts: bool, sample: bool, root: bool) -> R<Value> {
    let nums = if text_counts {
        numbers_a(ctx, args)?
    } else {
        ctx.numbers(args)?
    };
    let v = variance(&nums, sample)?;
    number(if root { v.sqrt() } else { v })
}

fn f_stdev_s(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    spread(ctx, args, false, true, true)
}

fn f_stdev_p(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    spread(ctx, args, false, false, true)
}

fn f_stdeva(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    spread(ctx, args, true, true, true)
}

fn f_stdevpa(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    spread(ctx, args, true, false, true)
}

fn f_var_s(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    spread(ctx, args, false, true, false)
}

fn f_var_p(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    spread(ctx, args, false, false, false)
}

fn f_vara(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    spread(ctx, args, true, true, false)
}

fn f_varpa(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    spread(ctx, args, true, false, false)
}

fn f_avedev(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let nums = ctx.numbers(args)?;
    if nums.is_empty() {
        return Err(ErrorKind::Num);
    }
    let m = mean(&nums);
    number(nums.iter().map(|x| (x - m).abs()).sum::<f64>() / nums.len() as f64)
}

fn f_devsq(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let nums = ctx.numbers(args)?;
    if nums.is_empty() {
        return Err(ErrorKind::Num);
    }
    number(devsq(&nums))
}

fn f_geomean(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let nums = ctx.numbers(args)?;
    if nums.is_empty() || nums.iter().any(|x| *x <= 0.0) {
        return Err(ErrorKind::Num);
    }
    let n = nums.len() as f64;
    let product: f64 = nums.iter().product();
    // The direct product is the most accurate; fall back to logarithms
    // when it leaves the comfortable range of an f64.
    if product.is_finite() && product > 1e-290 {
        let root = product.powf(1.0 / n);
        // Exact roots come out exact (GEOMEAN(2,4,8) = 4).
        let whole = root.round();
        let exact = (whole - root).abs() < 1e-9 && whole.powi(nums.len() as i32) == product;
        return number(if exact { whole } else { root });
    }
    number((nums.iter().map(|x| x.ln()).sum::<f64>() / n).exp())
}

fn f_harmean(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let nums = ctx.numbers(args)?;
    if nums.is_empty() {
        return Err(ErrorKind::NA);
    }
    if nums.iter().any(|x| *x <= 0.0) {
        return Err(ErrorKind::Num);
    }
    number(nums.len() as f64 / nums.iter().map(|x| 1.0 / x).sum::<f64>())
}

fn f_trimmean(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let nums = sorted_numbers(ctx, &args[0])?;
    let percent = ctx.num(&args[1])?;
    if nums.is_empty() || !(0.0..1.0).contains(&percent) {
        return Err(ErrorKind::Num);
    }
    // The number of excluded points is rounded down to a multiple of 2.
    let trim = (snap15(nums.len() as f64 * percent) / 2.0).floor() as usize;
    number(mean(&nums[trim..nums.len() - trim]))
}

fn f_skew(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let nums = ctx.numbers(args)?;
    let n = nums.len() as f64;
    if n < 3.0 {
        return Err(ErrorKind::Div0);
    }
    let s = variance(&nums, true)?.sqrt();
    if s == 0.0 {
        return Err(ErrorKind::Div0);
    }
    let m = mean(&nums);
    let cubes: f64 = nums.iter().map(|x| ((x - m) / s).powi(3)).sum();
    number(n / ((n - 1.0) * (n - 2.0)) * cubes)
}

fn f_kurt(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let nums = ctx.numbers(args)?;
    let n = nums.len() as f64;
    if n < 4.0 {
        return Err(ErrorKind::Div0);
    }
    let s = variance(&nums, true)?.sqrt();
    if s == 0.0 {
        return Err(ErrorKind::Div0);
    }
    let m = mean(&nums);
    let fourth: f64 = nums.iter().map(|x| ((x - m) / s).powi(4)).sum();
    number(
        n * (n + 1.0) / ((n - 1.0) * (n - 2.0) * (n - 3.0)) * fourth
            - 3.0 * (n - 1.0) * (n - 1.0) / ((n - 2.0) * (n - 3.0)),
    )
}

// ---- order statistics ---------------------------------------------------------

fn kth_of(ctx: &EvalCtx, args: &[Expr], largest: bool) -> R<Value> {
    let nums = sorted_numbers(ctx, &args[0])?;
    lift(ctx, &[args.get(1)], |v| {
        number(kth(&nums, opt_number(ctx, &v[0], 0.0)?, largest)?)
    })
}

fn f_large(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    kth_of(ctx, args, true)
}

fn f_small(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    kth_of(ctx, args, false)
}

fn rank_of(ctx: &EvalCtx, args: &[Expr], average: bool) -> R<Value> {
    let nums = ctx.numbers(&args[1..2])?;
    lift(ctx, &[args.first(), args.get(2)], |v| {
        let x = opt_number(ctx, &v[0], 0.0)?;
        let descending = opt_number(ctx, &v[1], 0.0)? == 0.0;
        let ahead = nums
            .iter()
            .filter(|n| if descending { **n > x } else { **n < x })
            .count() as f64;
        let ties = nums.iter().filter(|n| **n == x).count() as f64;
        if ties == 0.0 {
            return Err(ErrorKind::NA);
        }
        number(if average {
            ahead + (ties + 1.0) / 2.0
        } else {
            ahead + 1.0
        })
    })
}

fn f_rank(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    rank_of(ctx, args, false)
}

fn f_rank_avg(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    rank_of(ctx, args, true)
}

fn percentile_of(ctx: &EvalCtx, args: &[Expr], exclusive: bool) -> R<Value> {
    let nums = sorted_numbers(ctx, &args[0])?;
    lift(ctx, &[args.get(1)], |v| {
        number(percentile(&nums, opt_number(ctx, &v[0], 0.0)?, exclusive)?)
    })
}

fn f_percentile_inc(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    percentile_of(ctx, args, false)
}

fn f_percentile_exc(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    percentile_of(ctx, args, true)
}

fn quartile_of(ctx: &EvalCtx, args: &[Expr], exclusive: bool) -> R<Value> {
    let nums = sorted_numbers(ctx, &args[0])?;
    lift(ctx, &[args.get(1)], |v| {
        number(quartile(&nums, opt_number(ctx, &v[0], 0.0)?, exclusive)?)
    })
}

fn f_quartile_inc(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    quartile_of(ctx, args, false)
}

fn f_quartile_exc(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    quartile_of(ctx, args, true)
}

/// Rank of `x` as a fraction of the data set, interpolated between the
/// neighbouring values when `x` is not one of them, truncated to
/// `significance` digits.
fn percentrank(sorted: &[f64], x: f64, significance: f64, exclusive: bool) -> R<f64> {
    let n = sorted.len();
    let significance = significance.trunc();
    if n == 0 || significance < 1.0 {
        return Err(ErrorKind::Num);
    }
    if x < sorted[0] || x > sorted[n - 1] {
        return Err(ErrorKind::NA);
    }
    // Rank of a value that has `below` values under it.
    let rank = |below: usize| -> f64 {
        if exclusive {
            (below + 1) as f64 / (n + 1) as f64
        } else if n == 1 {
            1.0
        } else {
            below as f64 / (n - 1) as f64
        }
    };
    let below = sorted.partition_point(|v| *v < x);
    let r = if sorted[below] == x {
        rank(below)
    } else {
        let lower = sorted[below - 1];
        let upper = sorted[below];
        let below_lower = sorted.partition_point(|v| *v < lower);
        let (lo, hi) = (rank(below_lower), rank(below));
        lo + (x - lower) / (upper - lower) * (hi - lo)
    };
    Ok(round_to(r, significance.min(400.0) as i32, Rounding::Down))
}

fn percentrank_of(ctx: &EvalCtx, args: &[Expr], exclusive: bool) -> R<Value> {
    let nums = sorted_numbers(ctx, &args[0])?;
    lift(ctx, &[args.get(1), args.get(2)], |v| {
        let x = opt_number(ctx, &v[0], 0.0)?;
        let significance = opt_number(ctx, &v[1], 3.0)?;
        number(percentrank(&nums, x, significance, exclusive)?)
    })
}

fn f_percentrank_inc(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    percentrank_of(ctx, args, false)
}

fn f_percentrank_exc(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    percentrank_of(ctx, args, true)
}

fn f_frequency(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let data = ctx.numbers(&args[..1])?;
    let bins = ctx.numbers(&args[1..2])?;
    // Bins may come unsorted; counts are reported in the given order and
    // a repeated bin value collects everything in its first occurrence.
    let mut order: Vec<usize> = (0..bins.len()).collect();
    order.sort_by(|a, b| bins[*a].total_cmp(&bins[*b]).then(a.cmp(b)));
    let sorted: Vec<f64> = order.iter().map(|i| bins[*i]).collect();
    let mut counts = vec![0.0; bins.len() + 1];
    for x in data {
        let pos = sorted.partition_point(|b| *b < x);
        match order.get(pos) {
            Some(i) => counts[*i] += 1.0,
            None => counts[bins.len()] += 1.0,
        }
    }
    Ok(column(counts))
}

// ---- two-variable statistics --------------------------------------------------

struct PairStats {
    n: f64,
    mean_x: f64,
    mean_y: f64,
    sxx: f64,
    syy: f64,
    sxy: f64,
}

/// Sums of squares and products for the numeric pairs of `x` and `y`.
fn pair_stats(ctx: &EvalCtx, x: &Expr, y: &Expr) -> R<PairStats> {
    let pairs = pairs(ctx, x, y)?;
    let xs: Vec<f64> = pairs.iter().map(|p| p.0).collect();
    let ys: Vec<f64> = pairs.iter().map(|p| p.1).collect();
    if pairs.is_empty() {
        return Err(ErrorKind::Div0);
    }
    let (mean_x, mean_y) = (mean(&xs), mean(&ys));
    let (sxx, syy) = (devsq(&xs), devsq(&ys));
    let sxy = if sxx == 0.0 || syy == 0.0 {
        0.0
    } else {
        pairs.iter().map(|(x, y)| (x - mean_x) * (y - mean_y)).sum()
    };
    Ok(PairStats {
        n: pairs.len() as f64,
        mean_x,
        mean_y,
        sxx,
        syy,
        sxy,
    })
}

fn correlation(s: &PairStats) -> R<f64> {
    if s.sxx == 0.0 || s.syy == 0.0 {
        return Err(ErrorKind::Div0);
    }
    let product = s.sxx * s.syy;
    let denominator = if product.is_normal() {
        product.sqrt()
    } else {
        s.sxx.sqrt() * s.syy.sqrt()
    };
    Ok((s.sxy / denominator).clamp(-1.0, 1.0))
}

fn f_correl(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    number(correlation(&pair_stats(ctx, &args[0], &args[1])?)?)
}

fn f_rsq(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let r = correlation(&pair_stats(ctx, &args[0], &args[1])?)?;
    number(r * r)
}

fn f_covariance_p(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let s = pair_stats(ctx, &args[0], &args[1])?;
    number(s.sxy / s.n)
}

fn f_covariance_s(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let s = pair_stats(ctx, &args[0], &args[1])?;
    if s.n < 2.0 {
        return Err(ErrorKind::Div0);
    }
    number(s.sxy / (s.n - 1.0))
}

/// Least-squares line through `(known_x, known_y)`: `(slope, intercept, stats)`.
fn line(ctx: &EvalCtx, known_y: &Expr, known_x: &Expr) -> R<(f64, f64, PairStats)> {
    let s = pair_stats(ctx, known_x, known_y)?;
    if s.sxx == 0.0 {
        return Err(ErrorKind::Div0);
    }
    let slope = s.sxy / s.sxx;
    Ok((slope, s.mean_y - slope * s.mean_x, s))
}

fn f_slope(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    number(line(ctx, &args[0], &args[1])?.0)
}

fn f_intercept(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    number(line(ctx, &args[0], &args[1])?.1)
}

fn f_steyx(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let (_, _, s) = line(ctx, &args[0], &args[1])?;
    if s.n < 3.0 {
        return Err(ErrorKind::Div0);
    }
    number(((s.syy - s.sxy * s.sxy / s.sxx).max(0.0) / (s.n - 2.0)).sqrt())
}

fn f_forecast(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    lift(ctx, &[args.first()], |v| {
        let x = opt_number(ctx, &v[0], 0.0)?;
        let (slope, intercept, _) = line(ctx, &args[1], &args[2])?;
        number(intercept + slope * x)
    })
}

// ---- regression (LINEST / TREND / GROWTH) -----------------------------------------

/// Observations for a regression: `y` and one vector per variable.
struct Design {
    y: Vec<f64>,
    y_shape: (usize, usize),
    xs: Vec<Vec<f64>>,
    /// Variables are the rows of the x block (y is a single row) rather
    /// than its columns.
    by_rows: bool,
}

fn split_variables(rows: usize, cols: usize, data: &[f64], by_rows: bool) -> Vec<Vec<f64>> {
    if by_rows {
        data.chunks(cols).map(<[f64]>::to_vec).collect()
    } else {
        (0..cols)
            .map(|c| (0..rows).map(|r| data[r * cols + c]).collect())
            .collect()
    }
}

/// Read `known_y's` and `known_x's`. Blocks of mismatched size are `#REF!`.
fn design(ctx: &EvalCtx, y: &Expr, x: Option<&Expr>, log_y: bool) -> R<Design> {
    let (yr, yc, mut yv) = matrix(ctx, y)?;
    if log_y {
        for v in yv.iter_mut() {
            if *v <= 0.0 {
                return Err(ErrorKind::Num);
            }
            *v = v.ln();
        }
    }
    let (xs, by_rows) = match x {
        // Default x values are 1, 2, 3, ...
        None => (vec![(1..=yv.len()).map(|i| i as f64).collect()], false),
        Some(e) => {
            let (xr, xc, xv) = matrix(ctx, e)?;
            if xr == yr && xc == yc {
                (vec![xv], false)
            } else if yc == 1 && xr == yr {
                (split_variables(xr, xc, &xv, false), false)
            } else if yr == 1 && xc == yc {
                (split_variables(xr, xc, &xv, true), true)
            } else {
                return Err(ErrorKind::Ref);
            }
        }
    };
    Ok(Design {
        y: yv,
        y_shape: (yr, yc),
        xs,
        by_rows,
    })
}

struct Fit {
    slopes: Vec<f64>,
    intercept: f64,
    se: Vec<f64>,
    se_intercept: f64,
    r2: f64,
    se_y: f64,
    f: f64,
    df: f64,
    ss_reg: f64,
    ss_resid: f64,
}

/// Multiple linear regression by the normal equations on centred data.
/// Redundant (collinear) variables get a coefficient of 0.
fn fit(y: &[f64], xs: &[Vec<f64>], with_const: bool) -> Fit {
    let n = y.len();
    let k = xs.len();
    let y_bar = if with_const { mean(y) } else { 0.0 };
    let x_bar: Vec<f64> = xs
        .iter()
        .map(|x| if with_const { mean(x) } else { 0.0 })
        .collect();
    let mut a = vec![0.0; k * k];
    let mut b = vec![0.0; k];
    for i in 0..k {
        for j in i..k {
            let s: f64 = (0..n)
                .map(|t| (xs[i][t] - x_bar[i]) * (xs[j][t] - x_bar[j]))
                .sum();
            a[i * k + j] = s;
            a[j * k + i] = s;
        }
        b[i] = (0..n).map(|t| (xs[i][t] - x_bar[i]) * (y[t] - y_bar)).sum();
    }
    // Gauss-Jordan on the (positive semi-definite) matrix, pivoting on the
    // diagonal and skipping variables that add no new information.
    let diag: Vec<f64> = (0..k).map(|i| a[i * k + i]).collect();
    let mut inv = vec![0.0; k * k];
    for i in 0..k {
        inv[i * k + i] = 1.0;
    }
    let mut active = vec![true; k];
    for j in 0..k {
        let pivot = a[j * k + j];
        if !(diag[j] > 0.0 && pivot > diag[j] * 1e-11) {
            active[j] = false;
            continue;
        }
        for c in 0..k {
            a[j * k + c] /= pivot;
            inv[j * k + c] /= pivot;
        }
        for r in 0..k {
            let f = a[r * k + j];
            if r != j && f != 0.0 {
                for c in 0..k {
                    a[r * k + c] -= f * a[j * k + c];
                    inv[r * k + c] -= f * inv[j * k + c];
                }
            }
        }
    }
    let slopes: Vec<f64> = (0..k)
        .map(|i| {
            if active[i] {
                (0..k)
                    .filter(|j| active[*j])
                    .map(|j| inv[i * k + j] * b[j])
                    .sum()
            } else {
                0.0
            }
        })
        .collect();
    let intercept = if with_const {
        y_bar - (0..k).map(|i| slopes[i] * x_bar[i]).sum::<f64>()
    } else {
        0.0
    };
    let ss_resid: f64 = (0..n)
        .map(|t| {
            let predicted = intercept + (0..k).map(|i| slopes[i] * xs[i][t]).sum::<f64>();
            (y[t] - predicted) * (y[t] - predicted)
        })
        .sum();
    let ss_total: f64 = y.iter().map(|v| (v - y_bar) * (v - y_bar)).sum();
    let ss_reg = (ss_total - ss_resid).max(0.0);
    let used = active.iter().filter(|a| **a).count() as f64;
    let df = n as f64 - used - if with_const { 1.0 } else { 0.0 };
    let s2 = if df > 0.0 { ss_resid / df } else { f64::NAN };
    let se = (0..k)
        .map(|i| {
            if active[i] {
                (s2 * inv[i * k + i]).sqrt()
            } else {
                0.0
            }
        })
        .collect();
    let se_intercept = if with_const {
        let mut q = 1.0 / n as f64;
        for i in (0..k).filter(|i| active[*i]) {
            for j in (0..k).filter(|j| active[*j]) {
                q += x_bar[i] * inv[i * k + j] * x_bar[j];
            }
        }
        (s2 * q).sqrt()
    } else {
        f64::NAN
    };
    Fit {
        slopes,
        intercept,
        se,
        se_intercept,
        r2: if ss_total == 0.0 {
            1.0
        } else {
            ss_reg / ss_total
        },
        se_y: s2.sqrt(),
        f: (ss_reg / used) / s2,
        df,
        ss_reg,
        ss_resid,
    }
}

fn optional<'e>(ctx: &EvalCtx, args: &'e [Expr], index: usize) -> Option<&'e Expr> {
    args.get(index).filter(|e| !ctx.is_missing(e))
}

fn f_linest(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let d = design(ctx, &args[0], optional(ctx, args, 1), false)?;
    let with_const = ctx.bool_or(args, 2, true)?;
    let stats = ctx.bool_or(args, 3, false)?;
    let fit = fit(&d.y, &d.xs, with_const);
    let k = d.xs.len();
    let cell = |x: f64| {
        if x.is_finite() {
            Value::Number(x + 0.0)
        } else {
            Value::Error(ErrorKind::Num)
        }
    };
    let mut out = Array::new(if stats { 5 } else { 1 }, k + 1);
    // Coefficients run from the last variable to the first, then the constant.
    for i in 0..k {
        out.set(0, k - 1 - i, cell(fit.slopes[i]));
    }
    out.set(0, k, cell(fit.intercept));
    if stats {
        for v in out.data[k + 1..].iter_mut() {
            *v = Value::Error(ErrorKind::NA);
        }
        for i in 0..k {
            out.set(1, k - 1 - i, cell(fit.se[i]));
        }
        if with_const {
            out.set(1, k, cell(fit.se_intercept));
        }
        out.set(2, 0, cell(fit.r2));
        out.set(2, 1, cell(fit.se_y));
        out.set(3, 0, cell(fit.f));
        out.set(3, 1, cell(fit.df));
        out.set(4, 0, cell(fit.ss_reg));
        out.set(4, 1, cell(fit.ss_resid));
    }
    Ok(Value::array(out))
}

/// `TREND` (linear) and `GROWTH` (exponential, fitted on `ln y`).
fn predict(ctx: &EvalCtx, args: &[Expr], exponential: bool) -> R<Value> {
    let d = design(ctx, &args[0], optional(ctx, args, 1), exponential)?;
    let with_const = ctx.bool_or(args, 3, true)?;
    let fit = fit(&d.y, &d.xs, with_const);
    let k = d.xs.len();
    // New x values default to the known ones.
    let (rows, cols, points): (usize, usize, Vec<Vec<f64>>) = match optional(ctx, args, 2) {
        None => {
            let (rows, cols) = d.y_shape;
            let points = (0..d.y.len())
                .map(|t| d.xs.iter().map(|x| x[t]).collect())
                .collect();
            (rows, cols, points)
        }
        Some(e) => {
            let (nr, nc, data) = matrix(ctx, e)?;
            if k == 1 {
                (nr, nc, data.iter().map(|x| vec![*x]).collect())
            } else if d.by_rows {
                if nr != k {
                    return Err(ErrorKind::Ref);
                }
                let points = (0..nc)
                    .map(|c| (0..nr).map(|r| data[r * nc + c]).collect())
                    .collect();
                (1, nc, points)
            } else {
                if nc != k {
                    return Err(ErrorKind::Ref);
                }
                (nr, 1, data.chunks(nc).map(<[f64]>::to_vec).collect())
            }
        }
    };
    let mut data = Vec::with_capacity(points.len());
    for p in &points {
        let linear = fit.intercept + p.iter().zip(&fit.slopes).map(|(x, m)| x * m).sum::<f64>();
        data.push(number(if exponential { linear.exp() } else { linear })?);
    }
    Ok(Value::array(Array { rows, cols, data }))
}

fn f_trend(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    predict(ctx, args, false)
}

fn f_growth(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    predict(ctx, args, true)
}

// ---- special functions ----------------------------------------------------------

const SQRT_2PI: f64 = 2.506_628_274_631_000_5;
const LN_SQRT_2PI: f64 = 0.918_938_533_204_672_7;

/// Lanczos coefficients (g = 7, n = 9).
const LANCZOS: [f64; 9] = [
    0.999_999_999_999_809_9,
    676.520_368_121_885_1,
    -1_259.139_216_722_402_8,
    771.323_428_777_653_1,
    -176.615_029_162_140_6,
    12.507_343_278_686_905,
    -0.138_571_095_265_720_12,
    9.984_369_578_019_572e-6,
    1.505_632_735_149_311_6e-7,
];

/// The Lanczos series for an argument already reduced by 1.
fn lanczos_sum(x: f64) -> f64 {
    let mut a = LANCZOS[0];
    for (i, c) in LANCZOS.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    a
}

/// ln Γ(x) for x > 0.
fn ln_gamma(x: f64) -> f64 {
    if x.fract() == 0.0 && (1.0..=171.0).contains(&x) {
        return factorial(x - 1.0).ln();
    }
    if x < 0.5 {
        return (PI / (PI * x).sin().abs()).ln() - ln_gamma(1.0 - x);
    }
    let x = x - 1.0;
    let t = x + 7.5;
    LN_SQRT_2PI + (x + 0.5) * t.ln() - t + lanczos_sum(x).ln()
}

/// Γ(x); NaN at the poles (0 and the negative integers).
fn gamma(x: f64) -> f64 {
    if x.fract() == 0.0 {
        return if x <= 0.0 {
            f64::NAN
        } else {
            factorial(x - 1.0)
        };
    }
    if x < 0.5 {
        return PI / ((PI * x).sin() * gamma(1.0 - x));
    }
    let x = x - 1.0;
    let t = x + 7.5;
    // t^(x+0.5) is split in two so that it does not overflow early.
    let half = t.powf((x + 0.5) / 2.0);
    SQRT_2PI * half * (-t).exp() * half * lanczos_sum(x)
}

/// Regularized incomplete gamma functions `(P(a,x), Q(a,x))` given
/// `front = e^-x x^a / Γ(a)`.
fn inc_gamma_with(a: f64, x: f64, front: f64) -> (f64, f64) {
    if x <= 0.0 {
        return (0.0, 1.0);
    }
    if x < a + 1.0 {
        // Series for P.
        let mut ap = a;
        let mut term = 1.0 / a;
        let mut sum = term;
        for _ in 0..100_000 {
            ap += 1.0;
            term *= x / ap;
            sum += term;
            if term.abs() < sum.abs() * 1e-17 {
                break;
            }
        }
        let p = (front * sum).clamp(0.0, 1.0);
        (p, 1.0 - p)
    } else {
        // Continued fraction for Q (modified Lentz).
        const TINY: f64 = 1e-300;
        let mut b = x + 1.0 - a;
        let mut c = 1.0 / TINY;
        let mut d = 1.0 / b;
        let mut h = d;
        for i in 1..100_000 {
            let i = i as f64;
            let an = -i * (i - a);
            b += 2.0;
            d = an * d + b;
            if d.abs() < TINY {
                d = TINY;
            }
            c = b + an / c;
            if c.abs() < TINY {
                c = TINY;
            }
            d = 1.0 / d;
            let delta = d * c;
            h *= delta;
            if (delta - 1.0).abs() <= f64::EPSILON {
                break;
            }
        }
        let q = (front * h).clamp(0.0, 1.0);
        (1.0 - q, q)
    }
}

fn inc_gamma(a: f64, x: f64) -> (f64, f64) {
    if x <= 0.0 {
        return (0.0, 1.0);
    }
    inc_gamma_with(a, x, (-x + a * x.ln() - ln_gamma(a)).exp())
}

/// Regularized incomplete beta function I_x(a, b).
fn inc_beta(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let front = (ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (-x).ln_1p()).exp();
    // Continued fraction (modified Lentz), valid for x < (a+1)/(a+b+2).
    let cf = |a: f64, b: f64, x: f64| -> f64 {
        const TINY: f64 = 1e-300;
        let (qab, qap, qam) = (a + b, a + 1.0, a - 1.0);
        let mut c = 1.0;
        let mut d = 1.0 - qab * x / qap;
        if d.abs() < TINY {
            d = TINY;
        }
        d = 1.0 / d;
        let mut h = d;
        for m in 1..100_000 {
            let m = m as f64;
            let m2 = 2.0 * m;
            for aa in [
                m * (b - m) * x / ((qam + m2) * (a + m2)),
                -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2)),
            ] {
                d = 1.0 + aa * d;
                if d.abs() < TINY {
                    d = TINY;
                }
                c = 1.0 + aa / c;
                if c.abs() < TINY {
                    c = TINY;
                }
                d = 1.0 / d;
                h *= d * c;
            }
            if (d * c - 1.0).abs() <= f64::EPSILON {
                break;
            }
        }
        h
    };
    let r = if x < (a + 1.0) / (a + b + 2.0) {
        front * cf(a, b, x) / a
    } else {
        1.0 - front * cf(b, a, 1.0 - x) / b
    };
    r.clamp(0.0, 1.0)
}

/// Complementary error function for x >= 0.
fn erfc_pos(x: f64) -> f64 {
    let t = x * x;
    inc_gamma_with(0.5, t, (-t).exp() * x / PI.sqrt()).1
}

/// Standard normal cumulative distribution `(P(Z <= z), P(Z > z))`.
fn norm_cdf_both(z: f64) -> (f64, f64) {
    let tail = 0.5 * erfc_pos(z.abs() / std::f64::consts::SQRT_2);
    if z <= 0.0 {
        (tail, 1.0 - tail)
    } else {
        (1.0 - tail, tail)
    }
}

fn norm_cdf(z: f64) -> f64 {
    norm_cdf_both(z).0
}

fn norm_pdf(z: f64) -> f64 {
    (-0.5 * z * z).exp() / SQRT_2PI
}

fn poly(coefficients: &[f64], x: f64) -> f64 {
    coefficients.iter().rev().fold(0.0, |acc, c| acc * x + c)
}

/// Inverse of the standard normal distribution for 0 < p < 1
/// (Wichura's AS 241, polished with one Halley step).
fn norm_inv(p: f64) -> f64 {
    const A: [f64; 8] = [
        3.387_132_872_796_366_5,
        1.331_416_678_917_843_8e2,
        1.971_590_950_306_551_3e3,
        1.373_169_376_550_946e4,
        4.592_195_393_154_987e4,
        6.726_577_092_700_87e4,
        3.343_057_558_358_813e4,
        2.509_080_928_730_122_7e3,
    ];
    const B: [f64; 8] = [
        1.0,
        4.231_333_070_160_091e1,
        6.871_870_074_920_579e2,
        5.394_196_021_424_751e3,
        2.121_379_430_158_659_7e4,
        3.930_789_580_009_271e4,
        2.872_908_573_572_194_3e4,
        5.226_495_278_852_854e3,
    ];
    const C: [f64; 8] = [
        1.423_437_110_749_683_5,
        4.630_337_846_156_546,
        5.769_497_221_460_691,
        3.647_848_324_763_204_5,
        1.270_458_252_452_368_4,
        2.417_807_251_774_506e-1,
        2.272_384_498_926_918_4e-2,
        7.745_450_142_783_414e-4,
    ];
    const D: [f64; 8] = [
        1.0,
        2.053_191_626_637_759,
        1.676_384_830_183_803_8,
        6.897_673_349_851e-1,
        1.481_039_764_274_800_8e-1,
        1.519_866_656_361_645_7e-2,
        5.475_938_084_995_345e-4,
        1.050_750_071_644_416_9e-9,
    ];
    const E: [f64; 8] = [
        6.657_904_643_501_103,
        5.463_784_911_164_114,
        1.784_826_539_917_291_3,
        2.965_605_718_285_048_7e-1,
        2.653_218_952_657_612_4e-2,
        1.242_660_947_388_078_4e-3,
        2.711_555_568_743_487_6e-5,
        2.010_334_399_292_288_1e-7,
    ];
    const F: [f64; 8] = [
        1.0,
        5.998_322_065_558_88e-1,
        1.369_298_809_227_358e-1,
        1.487_536_129_085_061_5e-2,
        7.868_691_311_456_133e-4,
        1.846_318_317_510_054_8e-5,
        1.421_511_758_316_446e-7,
        2.044_263_103_389_939_7e-15,
    ];
    let q = p - 0.5;
    let x = if q.abs() <= 0.425 {
        let r = 0.180625 - q * q;
        q * poly(&A, r) / poly(&B, r)
    } else {
        let r = (-(if q < 0.0 { p } else { 1.0 - p }).ln()).sqrt();
        let x = if r <= 5.0 {
            poly(&C, r - 1.6) / poly(&D, r - 1.6)
        } else {
            poly(&E, r - 5.0) / poly(&F, r - 5.0)
        };
        if q < 0.0 { -x } else { x }
    };
    // Polish against the CDF, measuring the error on the tail side so that
    // no precision is lost near 1.
    let (lower, upper) = norm_cdf_both(x);
    let err = if x <= 0.0 {
        lower - p
    } else {
        (1.0 - p) - upper
    };
    let density = norm_pdf(x);
    if density > 0.0 && err.is_finite() {
        let u = err / density;
        x - u / (1.0 + x * u / 2.0)
    } else {
        x
    }
}

// ---- distributions -------------------------------------------------------------

fn f_standardize(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let x = ctx.num(&args[0])?;
    let mean = ctx.num(&args[1])?;
    let sd = ctx.num(&args[2])?;
    if sd <= 0.0 {
        return Err(ErrorKind::Num);
    }
    number((x - mean) / sd)
}

fn f_norm_dist(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let x = ctx.num(&args[0])?;
    let mean = ctx.num(&args[1])?;
    let sd = ctx.num(&args[2])?;
    let cumulative = ctx.boolean(&args[3])?;
    if sd <= 0.0 {
        return Err(ErrorKind::Num);
    }
    let z = (x - mean) / sd;
    number(if cumulative {
        norm_cdf(z)
    } else {
        norm_pdf(z) / sd
    })
}

fn f_norm_inv(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let p = ctx.num(&args[0])?;
    let mean = ctx.num(&args[1])?;
    let sd = ctx.num(&args[2])?;
    if p <= 0.0 || p >= 1.0 || sd <= 0.0 {
        return Err(ErrorKind::Num);
    }
    number(mean + sd * norm_inv(p))
}

fn f_norm_s_dist(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let z = ctx.num(&args[0])?;
    let cumulative = ctx.boolean(&args[1])?;
    number(if cumulative { norm_cdf(z) } else { norm_pdf(z) })
}

fn f_normsdist(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    number(norm_cdf(ctx.num(&args[0])?))
}

fn f_norm_s_inv(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let p = ctx.num(&args[0])?;
    if p <= 0.0 || p >= 1.0 {
        return Err(ErrorKind::Num);
    }
    number(norm_inv(p))
}

fn f_confidence(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let alpha = ctx.num(&args[0])?;
    let sd = ctx.num(&args[1])?;
    let size = ctx.num(&args[2])?.trunc();
    if alpha <= 0.0 || alpha >= 1.0 || sd <= 0.0 || size < 1.0 {
        return Err(ErrorKind::Num);
    }
    number(-norm_inv(alpha / 2.0) * sd / size.sqrt())
}

fn binom_pmf(k: f64, n: f64, p: f64) -> f64 {
    if p <= 0.0 {
        return if k == 0.0 { 1.0 } else { 0.0 };
    }
    if p >= 1.0 {
        return if k == n { 1.0 } else { 0.0 };
    }
    if n <= 170.0 {
        combin(n, k) * p.powf(k) * (1.0 - p).powf(n - k)
    } else {
        (ln_gamma(n + 1.0) - ln_gamma(k + 1.0) - ln_gamma(n - k + 1.0)
            + k * p.ln()
            + (n - k) * (-p).ln_1p())
        .exp()
    }
}

fn f_binom_dist(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let k = ctx.num(&args[0])?.trunc();
    let n = ctx.num(&args[1])?.trunc();
    let p = ctx.num(&args[2])?;
    let cumulative = ctx.boolean(&args[3])?;
    if k < 0.0 || n < 0.0 || k > n || !(0.0..=1.0).contains(&p) {
        return Err(ErrorKind::Num);
    }
    if !cumulative {
        return number(binom_pmf(k, n, p));
    }
    if k == n {
        return Ok(Value::Number(1.0));
    }
    if n > 2000.0 && p > 0.0 && p < 1.0 {
        // P(X <= k) = I_{1-p}(n-k, k+1)
        return number(inc_beta(n - k, k + 1.0, 1.0 - p));
    }
    let mut sum = 0.0;
    let mut i = 0.0;
    while i <= k {
        sum += binom_pmf(i, n, p);
        i += 1.0;
    }
    number(sum.min(1.0))
}

fn f_poisson_dist(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let x = ctx.num(&args[0])?.trunc();
    let mean = ctx.num(&args[1])?;
    let cumulative = ctx.boolean(&args[2])?;
    if x < 0.0 || mean < 0.0 {
        return Err(ErrorKind::Num);
    }
    if mean == 0.0 {
        return Ok(Value::Number(if cumulative || x == 0.0 {
            1.0
        } else {
            0.0
        }));
    }
    if !cumulative {
        // The direct formula is exact for small arguments; logarithms
        // keep the large ones from overflowing.
        return number(if x <= 20.0 && mean <= 20.0 {
            (-mean).exp() * mean.powf(x) / factorial(x)
        } else {
            (-mean + x * mean.ln() - ln_gamma(x + 1.0)).exp()
        });
    }
    if x > 2000.0 || mean >= 700.0 {
        // P(X <= x) = Q(x + 1, mean)
        return number(inc_gamma(x + 1.0, mean).1);
    }
    let mut term = (-mean).exp();
    let mut sum = term;
    let mut i = 1.0;
    while i <= x {
        term *= mean / i;
        sum += term;
        i += 1.0;
    }
    number(sum.min(1.0))
}

fn f_expon_dist(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let x = ctx.num(&args[0])?;
    let lambda = ctx.num(&args[1])?;
    let cumulative = ctx.boolean(&args[2])?;
    if x < 0.0 || lambda <= 0.0 {
        return Err(ErrorKind::Num);
    }
    number(if cumulative {
        -(-lambda * x).exp_m1()
    } else {
        lambda * (-lambda * x).exp()
    })
}

fn f_fisher(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let x = ctx.num(&args[0])?;
    if x <= -1.0 || x >= 1.0 {
        return Err(ErrorKind::Num);
    }
    number(x.atanh())
}

fn f_fisherinv(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    number(ctx.num(&args[0])?.tanh())
}

fn f_gammaln(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let x = ctx.num(&args[0])?;
    if x <= 0.0 {
        return Err(ErrorKind::Num);
    }
    number(ln_gamma(x))
}

fn f_gamma(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    number(gamma(ctx.num(&args[0])?))
}

const S: Category = Category::Statistical;

pub const FUNCTIONS: &[FunctionDef] = &[
    FunctionDef::new("AVERAGE", 1, VAR, Kind::Raw, S, f_average),
    FunctionDef::new("AVERAGEA", 1, VAR, Kind::Raw, S, f_averagea),
    FunctionDef::new("AVERAGEIF", 2, 3, Kind::Raw, S, f_averageif),
    FunctionDef::new("AVERAGEIFS", 3, VAR, Kind::Raw, S, f_averageifs),
    FunctionDef::new("COUNT", 1, VAR, Kind::Raw, S, f_count),
    FunctionDef::new("COUNTA", 1, VAR, Kind::Raw, S, f_counta),
    FunctionDef::new("COUNTBLANK", 1, 1, Kind::Raw, S, f_countblank),
    FunctionDef::new("COUNTIF", 2, 2, Kind::Raw, S, f_countif),
    FunctionDef::new("COUNTIFS", 2, VAR, Kind::Raw, S, f_countifs),
    FunctionDef::new("MAX", 1, VAR, Kind::Raw, S, f_max),
    FunctionDef::new("MIN", 1, VAR, Kind::Raw, S, f_min),
    FunctionDef::new("MAXA", 1, VAR, Kind::Raw, S, f_maxa),
    FunctionDef::new("MINA", 1, VAR, Kind::Raw, S, f_mina),
    FunctionDef::new("MAXIFS", 3, VAR, Kind::Raw, S, f_maxifs),
    FunctionDef::new("MINIFS", 3, VAR, Kind::Raw, S, f_minifs),
    FunctionDef::new("MEDIAN", 1, VAR, Kind::Raw, S, f_median),
    FunctionDef::new("MODE", 1, VAR, Kind::Raw, S, f_mode),
    FunctionDef::new("MODE.SNGL", 1, VAR, Kind::Raw, S, f_mode),
    FunctionDef::new("MODE.MULT", 1, VAR, Kind::Raw, S, f_mode_mult),
    FunctionDef::new("STDEV", 1, VAR, Kind::Raw, S, f_stdev_s),
    FunctionDef::new("STDEV.S", 1, VAR, Kind::Raw, S, f_stdev_s),
    FunctionDef::new("STDEV.P", 1, VAR, Kind::Raw, S, f_stdev_p),
    FunctionDef::new("STDEVP", 1, VAR, Kind::Raw, S, f_stdev_p),
    FunctionDef::new("STDEVA", 1, VAR, Kind::Raw, S, f_stdeva),
    FunctionDef::new("STDEVPA", 1, VAR, Kind::Raw, S, f_stdevpa),
    FunctionDef::new("VAR", 1, VAR, Kind::Raw, S, f_var_s),
    FunctionDef::new("VAR.S", 1, VAR, Kind::Raw, S, f_var_s),
    FunctionDef::new("VAR.P", 1, VAR, Kind::Raw, S, f_var_p),
    FunctionDef::new("VARP", 1, VAR, Kind::Raw, S, f_var_p),
    FunctionDef::new("VARA", 1, VAR, Kind::Raw, S, f_vara),
    FunctionDef::new("VARPA", 1, VAR, Kind::Raw, S, f_varpa),
    FunctionDef::new("LARGE", 2, 2, Kind::Raw, S, f_large),
    FunctionDef::new("SMALL", 2, 2, Kind::Raw, S, f_small),
    FunctionDef::new("RANK", 2, 3, Kind::Raw, S, f_rank),
    FunctionDef::new("RANK.EQ", 2, 3, Kind::Raw, S, f_rank),
    FunctionDef::new("RANK.AVG", 2, 3, Kind::Raw, S, f_rank_avg),
    FunctionDef::new("PERCENTILE", 2, 2, Kind::Raw, S, f_percentile_inc),
    FunctionDef::new("PERCENTILE.INC", 2, 2, Kind::Raw, S, f_percentile_inc),
    FunctionDef::new("PERCENTILE.EXC", 2, 2, Kind::Raw, S, f_percentile_exc),
    FunctionDef::new("QUARTILE", 2, 2, Kind::Raw, S, f_quartile_inc),
    FunctionDef::new("QUARTILE.INC", 2, 2, Kind::Raw, S, f_quartile_inc),
    FunctionDef::new("QUARTILE.EXC", 2, 2, Kind::Raw, S, f_quartile_exc),
    FunctionDef::new("PERCENTRANK", 2, 3, Kind::Raw, S, f_percentrank_inc),
    FunctionDef::new("PERCENTRANK.INC", 2, 3, Kind::Raw, S, f_percentrank_inc),
    FunctionDef::new("PERCENTRANK.EXC", 2, 3, Kind::Raw, S, f_percentrank_exc),
    FunctionDef::new("CORREL", 2, 2, Kind::Raw, S, f_correl),
    FunctionDef::new("PEARSON", 2, 2, Kind::Raw, S, f_correl),
    FunctionDef::new("COVAR", 2, 2, Kind::Raw, S, f_covariance_p),
    FunctionDef::new("COVARIANCE.P", 2, 2, Kind::Raw, S, f_covariance_p),
    FunctionDef::new("COVARIANCE.S", 2, 2, Kind::Raw, S, f_covariance_s),
    FunctionDef::new("SLOPE", 2, 2, Kind::Raw, S, f_slope),
    FunctionDef::new("INTERCEPT", 2, 2, Kind::Raw, S, f_intercept),
    FunctionDef::new("RSQ", 2, 2, Kind::Raw, S, f_rsq),
    FunctionDef::new("STEYX", 2, 2, Kind::Raw, S, f_steyx),
    FunctionDef::new("FORECAST", 3, 3, Kind::Raw, S, f_forecast),
    FunctionDef::new("FORECAST.LINEAR", 3, 3, Kind::Raw, S, f_forecast),
    FunctionDef::new("TREND", 1, 4, Kind::Raw, S, f_trend),
    FunctionDef::new("GROWTH", 1, 4, Kind::Raw, S, f_growth),
    FunctionDef::new("LINEST", 1, 4, Kind::Raw, S, f_linest),
    FunctionDef::new("GEOMEAN", 1, VAR, Kind::Raw, S, f_geomean),
    FunctionDef::new("HARMEAN", 1, VAR, Kind::Raw, S, f_harmean),
    FunctionDef::new("TRIMMEAN", 2, 2, Kind::Raw, S, f_trimmean),
    FunctionDef::new("AVEDEV", 1, VAR, Kind::Raw, S, f_avedev),
    FunctionDef::new("DEVSQ", 1, VAR, Kind::Raw, S, f_devsq),
    FunctionDef::new("SKEW", 1, VAR, Kind::Raw, S, f_skew),
    FunctionDef::new("KURT", 1, VAR, Kind::Raw, S, f_kurt),
    FunctionDef::new("STANDARDIZE", 3, 3, Kind::Scalar, S, f_standardize),
    FunctionDef::new("NORM.DIST", 4, 4, Kind::Scalar, S, f_norm_dist),
    FunctionDef::new("NORMDIST", 4, 4, Kind::Scalar, S, f_norm_dist),
    FunctionDef::new("NORM.INV", 3, 3, Kind::Scalar, S, f_norm_inv),
    FunctionDef::new("NORMINV", 3, 3, Kind::Scalar, S, f_norm_inv),
    FunctionDef::new("NORM.S.DIST", 2, 2, Kind::Scalar, S, f_norm_s_dist),
    FunctionDef::new("NORMSDIST", 1, 1, Kind::Scalar, S, f_normsdist),
    FunctionDef::new("NORM.S.INV", 1, 1, Kind::Scalar, S, f_norm_s_inv),
    FunctionDef::new("NORMSINV", 1, 1, Kind::Scalar, S, f_norm_s_inv),
    FunctionDef::new("FREQUENCY", 2, 2, Kind::Raw, S, f_frequency),
    FunctionDef::new("BINOM.DIST", 4, 4, Kind::Scalar, S, f_binom_dist),
    FunctionDef::new("BINOMDIST", 4, 4, Kind::Scalar, S, f_binom_dist),
    FunctionDef::new("POISSON.DIST", 3, 3, Kind::Scalar, S, f_poisson_dist),
    FunctionDef::new("POISSON", 3, 3, Kind::Scalar, S, f_poisson_dist),
    FunctionDef::new("EXPON.DIST", 3, 3, Kind::Scalar, S, f_expon_dist),
    FunctionDef::new("EXPONDIST", 3, 3, Kind::Scalar, S, f_expon_dist),
    FunctionDef::new("CONFIDENCE", 3, 3, Kind::Scalar, S, f_confidence),
    FunctionDef::new("CONFIDENCE.NORM", 3, 3, Kind::Scalar, S, f_confidence),
    FunctionDef::new("FISHER", 1, 1, Kind::Scalar, S, f_fisher),
    FunctionDef::new("FISHERINV", 1, 1, Kind::Scalar, S, f_fisherinv),
    FunctionDef::new("GAMMALN", 1, 1, Kind::Scalar, S, f_gammaln),
    FunctionDef::new("GAMMA", 1, 1, Kind::Scalar, S, f_gamma),
];

#[cfg(test)]
mod tests {
    use super::{gamma, inc_beta, inc_gamma, ln_gamma, norm_cdf, norm_inv};
    use crate::tests::{eval, num, val, wb};
    use crate::value::{ErrorKind, Value};
    use crate::workbook::Workbook;

    fn err(formula: &str) -> ErrorKind {
        match eval(formula) {
            Value::Error(e) => e,
            other => panic!("{formula} gave {other:?}, expected an error"),
        }
    }

    fn near(got: f64, expected: f64, tolerance: f64, what: &str) {
        assert!(
            (got - expected).abs() <= tolerance * expected.abs().max(1.0),
            "{what}: got {got}, expected {expected}"
        );
    }

    fn close(formula: &str, expected: f64) {
        near(num(formula), expected, 1e-9, formula);
    }

    fn cell(w: &Workbook, name: &str) -> f64 {
        match val(w, name) {
            Value::Number(n) => n,
            other => panic!("{name} holds {other:?}, expected a number"),
        }
    }

    /// Evaluate a formula against a workbook's data.
    fn calc(w: &mut Workbook, formula: &str) -> Value {
        w.set_cell_text(0, 49, 9, &format!("={formula}")).unwrap();
        w.commit("formula");
        w.value_at(0, 49, 9)
    }

    fn calc_num(w: &mut Workbook, formula: &str) -> f64 {
        match calc(w, formula) {
            Value::Number(n) => n,
            other => panic!("{formula} gave {other:?}, expected a number"),
        }
    }

    /// The array a formula spills, read back from the sheet.
    fn spill(formula: &str, rows: u32, cols: u32) -> Vec<Vec<Value>> {
        let w = wb(&[("A1", &format!("={formula}"))]);
        (0..rows)
            .map(|r| (0..cols).map(|c| w.value_at(0, r, c)).collect())
            .collect()
    }

    fn spill_nums(formula: &str, rows: u32, cols: u32) -> Vec<Vec<f64>> {
        spill(formula, rows, cols)
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|v| match v {
                        Value::Number(n) => n,
                        other => panic!("{formula} produced {other:?}"),
                    })
                    .collect()
            })
            .collect()
    }

    /// A column mixing every kind of value:
    /// 1, text "2", TRUE, "x", blank, #N/A, 3.5.
    fn mixed() -> Workbook {
        wb(&[
            ("A1", "1"),
            ("A2", "'2"),
            ("A3", "TRUE"),
            ("A4", "x"),
            ("A6", "#N/A"),
            ("A7", "3.5"),
            ("B1", "=\"\""),
        ])
    }

    /// Like `mixed` but without the error: 1, text "2", TRUE, "x", blank, 4.
    fn mixed_clean() -> Workbook {
        wb(&[
            ("A1", "1"),
            ("A2", "'2"),
            ("A3", "TRUE"),
            ("A4", "x"),
            ("A6", "4"),
        ])
    }

    #[test]
    fn count() {
        assert_eq!(num("COUNT(1,\"2\",TRUE,\"x\")"), 3.0);
        assert_eq!(num("COUNT({1,\"2\",TRUE,3})"), 2.0);
        assert_eq!(num("COUNT(\"1/15/2024\")"), 1.0);
        assert_eq!(num("COUNT(1,,3)"), 3.0);
        assert_eq!(num("COUNT(#N/A,1/0,5)"), 1.0);
        let mut w = mixed();
        assert_eq!(calc_num(&mut w, "COUNT(A1:A7)"), 2.0);
        assert_eq!(calc_num(&mut w, "COUNT(A:A)"), 2.0);
        assert_eq!(calc_num(&mut w, "COUNT(A2)"), 0.0);
        assert_eq!(calc_num(&mut w, "COUNT(A3)"), 0.0);
        assert_eq!(calc_num(&mut w, "COUNT(A5)"), 0.0);
        assert_eq!(calc_num(&mut w, "COUNT(A1:A7,5,\"7\",TRUE)"), 5.0);
    }

    #[test]
    fn counta_countblank() {
        assert_eq!(num("COUNTA(1,\"\",\"x\",FALSE,#N/A)"), 5.0);
        assert_eq!(num("COUNTA(1,,2)"), 3.0);
        let mut w = mixed();
        assert_eq!(calc_num(&mut w, "COUNTA(A1:A7)"), 6.0);
        assert_eq!(calc_num(&mut w, "COUNTA(A5)"), 0.0);
        assert_eq!(calc_num(&mut w, "COUNTA(A1:A7,B1)"), 7.0);
        assert_eq!(calc_num(&mut w, "COUNTBLANK(A1:A7)"), 1.0);
        assert_eq!(calc_num(&mut w, "COUNTBLANK(A1:A20)"), 14.0);
        // Empty strings count as blank; the range is not clamped.
        assert_eq!(calc_num(&mut w, "COUNTBLANK(B1:B2)"), 2.0);
        assert_eq!(calc_num(&mut w, "COUNTBLANK(A1:B7)"), 8.0);
        assert_eq!(calc_num(&mut w, "COUNTBLANK(A1:A1000)"), 994.0);
        assert_eq!(calc_num(&mut w, "COUNTBLANK(XFD1:XFD10)"), 10.0);
        assert_eq!(calc_num(&mut w, "COUNTBLANK(A:A)"), 1_048_570.0);
        assert_eq!(err("COUNTBLANK(5)"), ErrorKind::Value);
    }

    #[test]
    fn average() {
        assert_eq!(num("AVERAGE(10,7,9,27,2)"), 11.0);
        assert_eq!(num("AVERAGE(TRUE,3)"), 2.0);
        assert_eq!(num("AVERAGE(\"4\",{1,\"9\",TRUE})"), 2.5);
        assert_eq!(err("AVERAGE(\"x\")"), ErrorKind::Value);
        assert_eq!(err("AVERAGE(1,#REF!)"), ErrorKind::Ref);
        let mut w = mixed_clean();
        assert_eq!(calc_num(&mut w, "AVERAGE(A1:A6)"), 2.5);
        assert_eq!(calc_num(&mut w, "AVERAGE(A:A)"), 2.5);
        assert_eq!(calc_num(&mut w, "AVERAGE(A1:A6,10)"), 5.0);
        assert_eq!(
            calc(&mut w, "AVERAGE(A2:A5)"),
            Value::Error(ErrorKind::Div0)
        );
        assert_eq!(calc(&mut w, "AVERAGE(A4)"), Value::Error(ErrorKind::Div0));
        assert_eq!(
            calc(&mut w, "AVERAGE(C1:C9)"),
            Value::Error(ErrorKind::Div0)
        );
        let mut w = mixed();
        assert_eq!(calc(&mut w, "AVERAGE(A1:A7)"), Value::Error(ErrorKind::NA));
    }

    #[test]
    fn averagea_maxa_mina() {
        let mut w = wb(&[
            ("A1", "10"),
            ("A2", "7"),
            ("A3", "9"),
            ("A4", "2"),
            ("A5", "Not available"),
        ]);
        assert_eq!(calc_num(&mut w, "AVERAGEA(A1:A5)"), 5.6);
        assert_eq!(calc_num(&mut w, "AVERAGEA(A1:A4,A6)"), 7.0);
        assert_eq!(calc_num(&mut w, "AVERAGE(A1:A5)"), 7.0);
        assert_eq!(calc_num(&mut w, "MINA(A1:A5)"), 0.0);
        assert_eq!(calc_num(&mut w, "MIN(A1:A5)"), 2.0);
        assert_eq!(calc_num(&mut w, "MAXA(A1:A5)"), 10.0);
        assert_eq!(calc_num(&mut w, "AVERAGEA(A5)"), 0.0);
        let mut w = wb(&[
            ("A1", "0.2"),
            ("A2", "0.5"),
            ("A3", "TRUE"),
            ("B1", "-3"),
            ("B2", "FALSE"),
        ]);
        assert_eq!(calc_num(&mut w, "MAXA(A1:A3)"), 1.0);
        assert_eq!(calc_num(&mut w, "MAX(A1:A3)"), 0.5);
        assert_eq!(calc_num(&mut w, "MINA(A1:A3)"), 0.2);
        assert_eq!(calc_num(&mut w, "MAXA(B1:B2)"), 0.0);
        assert_eq!(calc_num(&mut w, "MAX(B1:B2)"), -3.0);
        near(
            calc_num(&mut w, "AVERAGEA(A1:A3)"),
            1.7 / 3.0,
            1e-12,
            "AVERAGEA",
        );
        assert_eq!(num("AVERAGEA(TRUE,3)"), 2.0);
        assert_eq!(num("AVERAGEA(\"4\",2)"), 3.0);
        assert_eq!(err("AVERAGEA(\"x\",2)"), ErrorKind::Value);
        assert_eq!(err("AVERAGEA(C1:C3)"), ErrorKind::Div0);
        assert_eq!(num("MAXA(C1:C3)"), 0.0);
    }

    #[test]
    fn max_min() {
        assert_eq!(num("MAX(10,7,9,27,2)"), 27.0);
        assert_eq!(num("MIN(10,7,9,27,2)"), 2.0);
        assert_eq!(num("MAX(\"5\",3)"), 5.0);
        assert_eq!(num("MAX(TRUE,0.5)"), 1.0);
        assert_eq!(num("MIN({4,\"1\",TRUE},6)"), 4.0);
        assert_eq!(num("MAX(-5,-2)"), -2.0);
        assert_eq!(err("MAX(\"x\",1)"), ErrorKind::Value);
        assert_eq!(err("MIN(1,#DIV/0!)"), ErrorKind::Div0);
        let mut w = mixed_clean();
        assert_eq!(calc_num(&mut w, "MAX(A1:A6)"), 4.0);
        assert_eq!(calc_num(&mut w, "MIN(A1:A6)"), 1.0);
        assert_eq!(calc_num(&mut w, "MIN(A:A)"), 1.0);
        // No numbers at all gives 0.
        assert_eq!(calc_num(&mut w, "MAX(A2:A5)"), 0.0);
        assert_eq!(calc_num(&mut w, "MIN(C1:C5)"), 0.0);
    }

    /// Property values and commissions, with a region column.
    fn property() -> Workbook {
        wb(&[
            ("A2", "100000"),
            ("A3", "200000"),
            ("A4", "300000"),
            ("A5", "400000"),
            ("B2", "7000"),
            ("B3", "14000"),
            ("B4", "21000"),
            ("B5", "28000"),
            ("C2", "East"),
            ("C3", "West"),
            ("C4", "east"),
            ("C5", "North"),
        ])
    }

    #[test]
    fn averageif() {
        let mut w = property();
        assert_eq!(calc_num(&mut w, "AVERAGEIF(B2:B5,\"<23000\")"), 14000.0);
        assert_eq!(calc_num(&mut w, "AVERAGEIF(A2:A5,\"<250000\")"), 150000.0);
        assert_eq!(
            calc(&mut w, "AVERAGEIF(A2:A5,\"<95000\")"),
            Value::Error(ErrorKind::Div0)
        );
        assert_eq!(
            calc_num(&mut w, "AVERAGEIF(A2:A5,\">250000\",B2:B5)"),
            24500.0
        );
        assert_eq!(calc_num(&mut w, "AVERAGEIF(C2:C5,\"east\",B2:B5)"), 14000.0);
        assert_eq!(calc_num(&mut w, "AVERAGEIF(C2:C5,\"<>east\",B2)"), 21000.0);
        assert_eq!(calc_num(&mut w, "AVERAGEIF(C:C,\"*t\",A:A)"), 200000.0);
        // Matching cells without numbers are skipped.
        assert_eq!(
            calc(&mut w, "AVERAGEIF(B2:B5,\">0\",C2:C5)"),
            Value::Error(ErrorKind::Div0)
        );
        assert_eq!(
            calc_num(&mut w, "AVERAGEIFS(B2:B5,C2:C5,\"east\",A2:A5,\">150000\")"),
            21000.0
        );
        assert_eq!(
            calc_num(
                &mut w,
                "AVERAGEIFS(B2:B5,A2:A5,\">=200000\",A2:A5,\"<=300000\")"
            ),
            17500.0
        );
        assert_eq!(
            calc(&mut w, "AVERAGEIFS(B2:B5,C2:C5,\"south\")"),
            Value::Error(ErrorKind::Div0)
        );
        assert_eq!(
            calc(&mut w, "AVERAGEIFS(B2:B5,C2:C4,\"east\")"),
            Value::Error(ErrorKind::Value)
        );
    }

    #[test]
    fn countif() {
        let mut w = wb(&[
            ("A2", "apples"),
            ("A3", "oranges"),
            ("A4", "peaches"),
            ("A5", "apples"),
            ("B2", "32"),
            ("B3", "54"),
            ("B4", "75"),
            ("B5", "86"),
        ]);
        assert_eq!(calc_num(&mut w, "COUNTIF(A2:A5,\"apples\")"), 2.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(A2:A5,\"APPLES\")"), 2.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(A2:A5,A4)"), 1.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(A2:A5,A2)+COUNTIF(A2:A5,A3)"), 3.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(B2:B5,\">55\")"), 2.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(B2:B5,\"<>\"&B4)"), 3.0);
        assert_eq!(
            calc_num(&mut w, "COUNTIF(B2:B5,\">=32\")-COUNTIF(B2:B5,\">85\")"),
            3.0
        );
        assert_eq!(calc_num(&mut w, "COUNTIF(A2:A5,\"*\")"), 4.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(A2:A5,\"?????es\")"), 2.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(A2:A5,\"*es\")"), 4.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(A2:A5,\"<>apples\")"), 2.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(A2:A5,\">b\")"), 2.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(B2:B5,54)"), 1.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(B2:B5,\"54\")"), 1.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(B2:B5,\"<=54\")"), 2.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(A2:B5,\"<>\")"), 8.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(A1:B6,\"\")"), 4.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(A1:B6,\"=\")"), 4.0);
        // An empty criterion cell means 0.
        assert_eq!(calc_num(&mut w, "COUNTIF(B2:B5,C1)"), 0.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(A:A,\"apples\")"), 2.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(B:B,\">=54\")"), 3.0);
    }

    #[test]
    fn countif_value_kinds() {
        let mut w = mixed();
        // 1, text "2", TRUE, "x", blank, #N/A, 3.5
        assert_eq!(calc_num(&mut w, "COUNTIF(A1:A7,2)"), 1.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(A1:A7,\">0\")"), 2.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(A1:A7,TRUE)"), 1.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(A1:A7,\"true\")"), 1.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(A1:A7,1)"), 1.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(A1:A7,\"#N/A\")"), 1.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(A1:A7,A6)"), 1.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(A1:A7,\"*\")"), 2.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(A1:A7,\"\")"), 1.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(A1:A7,\"<>\")"), 6.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(A1:A7,\"<>x\")"), 6.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(A1:A7,\"<>1\")"), 6.0);
        // B1 holds an empty string: blank for "" but not for "=".
        assert_eq!(calc_num(&mut w, "COUNTIF(B1:B2,\"\")"), 2.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(B1:B2,\"=\")"), 1.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(B1:B2,\"<>\")"), 1.0);
        assert_eq!(calc_num(&mut w, "COUNTIF(B1:B2,\"*\")"), 1.0);
    }

    #[test]
    fn countifs() {
        let mut w = property();
        assert_eq!(calc_num(&mut w, "COUNTIFS(C2:C5,\"east\")"), 2.0);
        assert_eq!(
            calc_num(&mut w, "COUNTIFS(C2:C5,\"east\",A2:A5,\">150000\")"),
            1.0
        );
        assert_eq!(
            calc_num(&mut w, "COUNTIFS(A2:A5,\">100000\",B2:B5,\"<28000\")"),
            2.0
        );
        assert_eq!(
            calc_num(
                &mut w,
                "COUNTIFS(A2:A5,\">100000\",A2:A5,\"<400000\",C2:C5,\"w*\")"
            ),
            1.0
        );
        assert_eq!(
            calc_num(&mut w, "COUNTIFS(A:A,\">100000\",C:C,\"<>west\")"),
            2.0
        );
        assert_eq!(calc_num(&mut w, "COUNTIFS(A1:A6,\"\",B1:B6,\"\")"), 2.0);
        assert_eq!(
            calc(&mut w, "COUNTIFS(A2:A5,\">1\",B2:B4,\">1\")"),
            Value::Error(ErrorKind::Value)
        );
        assert_eq!(
            calc(&mut w, "COUNTIFS(A2:A5,\">1\",B2:B5)"),
            Value::Error(ErrorKind::Value)
        );
        assert_eq!(
            calc(&mut w, "COUNTIFS(A2:A5,\">1\",B2:C3,\">1\")"),
            Value::Error(ErrorKind::Value)
        );
    }

    #[test]
    fn maxifs_minifs() {
        let mut w = property();
        assert_eq!(calc_num(&mut w, "MAXIFS(B2:B5,C2:C5,\"east\")"), 21000.0);
        assert_eq!(calc_num(&mut w, "MINIFS(B2:B5,C2:C5,\"east\")"), 7000.0);
        assert_eq!(
            calc_num(&mut w, "MAXIFS(B2:B5,A2:A5,\"<400000\",C2:C5,\"<>east\")"),
            14000.0
        );
        assert_eq!(calc_num(&mut w, "MINIFS(A2:A5,B2:B5,\">7000\")"), 200000.0);
        assert_eq!(calc_num(&mut w, "MAXIFS(B:B,C:C,\"north\")"), 28000.0);
        // No match gives 0.
        assert_eq!(calc_num(&mut w, "MAXIFS(B2:B5,C2:C5,\"south\")"), 0.0);
        assert_eq!(calc_num(&mut w, "MINIFS(B2:B5,C2:C5,\"south\")"), 0.0);
        assert_eq!(
            calc(&mut w, "MAXIFS(B2:B5,C2:C4,\"east\")"),
            Value::Error(ErrorKind::Value)
        );
    }

    #[test]
    fn median_mode() {
        assert_eq!(num("MEDIAN(1,2,3,4,5)"), 3.0);
        assert_eq!(num("MEDIAN(1,2,3,4,5,6)"), 3.5);
        assert_eq!(num("MEDIAN({5,1,9},\"3\")"), 4.0);
        assert_eq!(num("MEDIAN(7)"), 7.0);
        assert_eq!(err("MEDIAN(C1:C5)"), ErrorKind::Num);
        assert_eq!(num("MODE({5.6,4,4,3,2,4})"), 4.0);
        assert_eq!(num("MODE.SNGL({5.6,4,4,3,2,4})"), 4.0);
        // Ties go to the value met first.
        assert_eq!(num("MODE(1,2,2,1)"), 1.0);
        assert_eq!(num("MODE(3,2,2,3,1)"), 3.0);
        assert_eq!(err("MODE(1,2,3)"), ErrorKind::NA);
        assert_eq!(err("MODE.SNGL(C1:C5)"), ErrorKind::NA);
        assert_eq!(
            spill_nums("MODE.MULT({1,2,3,4,3,2,1,2,3,5,6,1})", 3, 1),
            vec![vec![1.0], vec![2.0], vec![3.0]]
        );
        assert_eq!(num("MODE.MULT({1,2,2,3})"), 2.0);
        assert_eq!(err("MODE.MULT({1,2,3})"), ErrorKind::NA);
        let mut w = mixed_clean();
        assert_eq!(calc_num(&mut w, "MEDIAN(A1:A6)"), 2.5);
    }

    #[test]
    fn stdev_var() {
        let d = "{1345,1301,1368,1322,1310,1370,1318,1350,1303,1299}";
        close(&format!("STDEV.S({d})"), 27.463915719843495);
        close(&format!("STDEV({d})"), 27.463915719843495);
        close(&format!("STDEV.P({d})"), 26.054558142482477);
        close(&format!("STDEVP({d})"), 26.054558142482477);
        close(&format!("VAR.S({d})"), 754.2666666666667);
        close(&format!("VAR({d})"), 754.2666666666667);
        close(&format!("VAR.P({d})"), 678.84);
        close(&format!("VARP({d})"), 678.84);
        assert_eq!(num("STDEV.P(2,4,4,4,5,5,7,9)"), 2.0);
        assert_eq!(num("VAR.S(1,2,3,4)"), 5.0 / 3.0);
        assert_eq!(num("STDEV(0.1,0.1,0.1)"), 0.0);
        assert_eq!(err("STDEV(5)"), ErrorKind::Div0);
        assert_eq!(err("VAR.S(5)"), ErrorKind::Div0);
        assert_eq!(num("STDEV.P(5)"), 0.0);
        assert_eq!(num("VARP(5)"), 0.0);
        assert_eq!(err("STDEVP(C1:C3)"), ErrorKind::Div0);
        assert_eq!(err("VAR.P(C1:C3)"), ErrorKind::Div0);
        // 1, 2, "x", TRUE: text and booleans count only for the A variants.
        let mut w = wb(&[("A1", "1"), ("A2", "2"), ("A3", "x"), ("A4", "TRUE")]);
        near(
            calc_num(&mut w, "STDEV(A1:A4)"),
            std::f64::consts::FRAC_1_SQRT_2,
            1e-12,
            "STDEV",
        );
        assert_eq!(calc_num(&mut w, "STDEVP(A1:A4)"), 0.5);
        assert_eq!(calc_num(&mut w, "VAR(A1:A4)"), 0.5);
        assert_eq!(calc_num(&mut w, "VARP(A1:A4)"), 0.25);
        near(
            calc_num(&mut w, "STDEVA(A1:A4)"),
            0.816496580927726,
            1e-12,
            "STDEVA",
        );
        near(
            calc_num(&mut w, "STDEVPA(A1:A4)"),
            std::f64::consts::FRAC_1_SQRT_2,
            1e-12,
            "STDEVPA",
        );
        near(calc_num(&mut w, "VARA(A1:A4)"), 2.0 / 3.0, 1e-12, "VARA");
        assert_eq!(calc_num(&mut w, "VARPA(A1:A4)"), 0.5);
    }

    #[test]
    fn large_small() {
        assert_eq!(num("LARGE({3,4,5,2,3,4,5,6,4,7},3)"), 5.0);
        assert_eq!(num("LARGE({3,4,5,2,3,4,5,6,4,7},7)"), 4.0);
        assert_eq!(num("LARGE({3,4,5,2,3,4,5,6,4,7},1)"), 7.0);
        assert_eq!(num("SMALL({3,4,5,2,3,4,6,4,7},4)"), 4.0);
        assert_eq!(num("SMALL({1,4,8,3,7,12,54,8,23},2)"), 3.0);
        assert_eq!(num("SMALL({5,\"1\",TRUE,3},1)"), 3.0);
        assert_eq!(err("LARGE({1,2,3},0)"), ErrorKind::Num);
        assert_eq!(err("LARGE({1,2,3},4)"), ErrorKind::Num);
        assert_eq!(err("SMALL({1,2,3},-1)"), ErrorKind::Num);
        assert_eq!(err("SMALL(C1:C3,1)"), ErrorKind::Num);
        assert_eq!(err("LARGE({1,#N/A,3},1)"), ErrorKind::NA);
        // An array of ranks gives an array of results.
        assert_eq!(num("SUM(LARGE({3,4,5,2,3,4,5,6,4,7},{1,2,3}))"), 18.0);
        assert_eq!(
            spill_nums("SMALL({9,7,8},{1;2;3})", 3, 1),
            vec![vec![7.0], vec![8.0], vec![9.0]]
        );
        let mut w = mixed_clean();
        assert_eq!(calc_num(&mut w, "LARGE(A1:A6,1)"), 4.0);
        assert_eq!(calc_num(&mut w, "SMALL(A:A,2)"), 4.0);
        assert_eq!(calc(&mut w, "LARGE(A1:A6,3)"), Value::Error(ErrorKind::Num));
    }

    #[test]
    fn rank() {
        let mut w = wb(&[
            ("A1", "7"),
            ("A2", "3.5"),
            ("A3", "3.5"),
            ("A4", "1"),
            ("A5", "2"),
            ("A6", "x"),
        ]);
        assert_eq!(calc_num(&mut w, "RANK(A2,A1:A5,1)"), 3.0);
        assert_eq!(calc_num(&mut w, "RANK(A1,A1:A5,1)"), 5.0);
        assert_eq!(calc_num(&mut w, "RANK(A1,A1:A6)"), 1.0);
        assert_eq!(calc_num(&mut w, "RANK.EQ(3.5,A1:A5)"), 2.0);
        assert_eq!(calc_num(&mut w, "RANK.EQ(2,A1:A5)"), 4.0);
        assert_eq!(calc_num(&mut w, "RANK.EQ(2,A1:A5,0)"), 4.0);
        assert_eq!(calc_num(&mut w, "RANK.EQ(2,A1:A5,5)"), 2.0);
        assert_eq!(calc_num(&mut w, "RANK.AVG(3.5,A1:A5)"), 2.5);
        assert_eq!(calc_num(&mut w, "RANK.AVG(3.5,A1:A5,1)"), 3.5);
        assert_eq!(calc_num(&mut w, "RANK.AVG(7,A1:A5)"), 1.0);
        assert_eq!(calc(&mut w, "RANK(4,A1:A5)"), Value::Error(ErrorKind::NA));
        assert_eq!(
            calc(&mut w, "RANK(\"x\",A1:A5)"),
            Value::Error(ErrorKind::Value)
        );
        assert_eq!(calc_num(&mut w, "SUM(RANK(A1:A5,A1:A5))"), 14.0);
        assert_eq!(num("RANK.AVG(94,{89,88,92,101,94,97,95})"), 4.0);
    }

    #[test]
    fn percentile_quartile() {
        close("PERCENTILE({1,2,3,4},0.3)", 1.9);
        assert_eq!(num("PERCENTILE({1,3,2,4},0.3)"), 1.9);
        assert_eq!(num("PERCENTILE.INC({1,2,3,4},0)"), 1.0);
        assert_eq!(num("PERCENTILE.INC({1,2,3,4},1)"), 4.0);
        assert_eq!(num("PERCENTILE.INC({1,2,3,4},0.5)"), 2.5);
        assert_eq!(num("PERCENTILE({15,20,35,40,50},0.4)"), 29.0);
        assert_eq!(num("PERCENTILE(7,0.3)"), 7.0);
        // Long data: the interpolation weight stays clean.
        assert_eq!(num("PERCENTILE(SEQUENCE(100000),0.9)"), 90000.1);
        assert_eq!(num("PERCENTILE(SEQUENCE(1001)/10,0.333)"), 33.4);
        assert_eq!(num("PERCENTILE.EXC(SEQUENCE(99),0.123)"), 12.3);
        assert_eq!(err("PERCENTILE({1,2,3,4},1.5)"), ErrorKind::Num);
        assert_eq!(err("PERCENTILE({1,2,3,4},-0.1)"), ErrorKind::Num);
        assert_eq!(err("PERCENTILE(C1:C4,0.5)"), ErrorKind::Num);
        assert_eq!(num("PERCENTILE.EXC({1,2,3,6,6,6,7,8,9},0.25)"), 2.5);
        assert_eq!(num("PERCENTILE.EXC({1,2,3,4},0.5)"), 2.5);
        assert_eq!(err("PERCENTILE.EXC({1,2,3,6,6,6,7,8,9},0)"), ErrorKind::Num);
        assert_eq!(
            err("PERCENTILE.EXC({1,2,3,6,6,6,7,8,9},0.01)"),
            ErrorKind::Num
        );
        assert_eq!(err("PERCENTILE.EXC({1,2,3,6,6,6,7,8,9},1)"), ErrorKind::Num);
        assert_eq!(
            err("PERCENTILE.EXC({1,2,3,6,6,6,7,8,9},0.95)"),
            ErrorKind::Num
        );
        assert_eq!(num("QUARTILE({1,2,4,7,8,9,10,12},1)"), 3.5);
        assert_eq!(num("QUARTILE.INC({1,2,4,7,8,9,10,12},3)"), 9.25);
        assert_eq!(num("QUARTILE({1,2,4,7,8,9,10,12},0)"), 1.0);
        assert_eq!(num("QUARTILE({1,2,4,7,8,9,10,12},2)"), 7.5);
        assert_eq!(num("QUARTILE({1,2,4,7,8,9,10,12},4)"), 12.0);
        assert_eq!(num("QUARTILE({1,2,4,7,8,9,10,12},1.9)"), 3.5);
        assert_eq!(err("QUARTILE({1,2,4,7,8,9,10,12},5)"), ErrorKind::Num);
        assert_eq!(err("QUARTILE({1,2,4,7,8,9,10,12},-1)"), ErrorKind::Num);
        assert_eq!(
            num("QUARTILE.EXC({6,7,15,36,39,40,41,42,43,47,49},1)"),
            15.0
        );
        assert_eq!(
            num("QUARTILE.EXC({6,7,15,36,39,40,41,42,43,47,49},2)"),
            40.0
        );
        assert_eq!(
            num("QUARTILE.EXC({6,7,15,36,39,40,41,42,43,47,49},3)"),
            43.0
        );
        assert_eq!(
            err("QUARTILE.EXC({6,7,15,36,39,40,41,42,43,47,49},0)"),
            ErrorKind::Num
        );
        assert_eq!(
            err("QUARTILE.EXC({6,7,15,36,39,40,41,42,43,47,49},4)"),
            ErrorKind::Num
        );
        assert_eq!(num("SUM(QUARTILE({1,2,4,7,8,9,10,12},{0,4}))"), 13.0);
    }

    #[test]
    fn percentrank() {
        let d = "{13,12,11,8,4,3,2,1,1,1}";
        assert_eq!(num(&format!("PERCENTRANK.INC({d},2)")), 0.333);
        assert_eq!(num(&format!("PERCENTRANK.INC({d},4)")), 0.555);
        assert_eq!(num(&format!("PERCENTRANK.INC({d},8)")), 0.666);
        assert_eq!(num(&format!("PERCENTRANK.INC({d},5)")), 0.583);
        assert_eq!(num(&format!("PERCENTRANK({d},5)")), 0.583);
        assert_eq!(num(&format!("PERCENTRANK({d},13)")), 1.0);
        assert_eq!(num(&format!("PERCENTRANK({d},1)")), 0.0);
        assert_eq!(num(&format!("PERCENTRANK({d},2,5)")), 0.33333);
        assert_eq!(num(&format!("PERCENTRANK({d},8,1)")), 0.6);
        assert_eq!(err(&format!("PERCENTRANK({d},14)")), ErrorKind::NA);
        assert_eq!(err(&format!("PERCENTRANK({d},0)")), ErrorKind::NA);
        assert_eq!(err(&format!("PERCENTRANK({d},2,0)")), ErrorKind::Num);
        let e = "{1,2,3,6,6,6,7,8,9}";
        assert_eq!(num(&format!("PERCENTRANK.EXC({e},7)")), 0.7);
        assert_eq!(num(&format!("PERCENTRANK.EXC({e},5.43)")), 0.381);
        assert_eq!(num(&format!("PERCENTRANK.EXC({e},5.43,1)")), 0.3);
        assert_eq!(num(&format!("PERCENTRANK.EXC({e},1)")), 0.1);
        assert_eq!(num(&format!("PERCENTRANK.EXC({e},9)")), 0.9);
        assert_eq!(err(&format!("PERCENTRANK.EXC({e},10)")), ErrorKind::NA);
    }

    #[test]
    fn frequency() {
        assert_eq!(
            spill_nums("FREQUENCY({79,85,78,85,50,81,95,88,97},{70,79,89})", 4, 1),
            vec![vec![1.0], vec![2.0], vec![4.0], vec![2.0]]
        );
        // Unsorted bins keep their order; repeated bins count once.
        assert_eq!(
            spill_nums("FREQUENCY({1,2,3,4,5,6},{4,2,2})", 4, 1),
            vec![vec![2.0], vec![2.0], vec![0.0], vec![2.0]]
        );
        assert_eq!(num("SUM(FREQUENCY({1,2,3,\"x\"},{2}))"), 3.0);
        let mut w = wb(&[
            ("A1", "1"),
            ("A2", "5"),
            ("A3", "x"),
            ("A4", "9"),
            ("B1", "4"),
        ]);
        assert_eq!(calc_num(&mut w, "SUM(FREQUENCY(A1:A5,B1:B2))"), 3.0);
    }

    #[test]
    fn correlation_covariance() {
        close("CORREL({3,2,4,5,6},{9,7,12,15,17})", 0.9970544855015815);
        close("PEARSON({9,7,5,3,1},{10,6,1,5,3})", 0.6993786061802354);
        assert_eq!(num("CORREL({1,2,3},{2,4,6})"), 1.0);
        assert_eq!(num("CORREL({1,2,3},{6,4,2})"), -1.0);
        assert_eq!(
            num("CORREL({1,2,3,4},{2;4;6;9})"),
            num("CORREL({1,2,3,4},{2,4,6,9})")
        );
        assert_eq!(err("CORREL({1,2,3},{1,2})"), ErrorKind::NA);
        assert_eq!(err("CORREL({1,1,1},{1,2,3})"), ErrorKind::Div0);
        assert_eq!(err("CORREL({1,\"a\"},{\"b\",2})"), ErrorKind::Div0);
        assert_eq!(err("CORREL({1,2,#N/A},{1,2,3})"), ErrorKind::NA);
        close("COVAR({3,2,4,5,6},{9,7,12,15,17})", 5.2);
        close("COVARIANCE.P({3,2,4,5,6},{9,7,12,15,17})", 5.2);
        close("COVARIANCE.S({2,4,8},{5,11,12})", 9.666666666666666);
        assert_eq!(err("COVARIANCE.S({2},{5})"), ErrorKind::Div0);
        assert_eq!(num("COVARIANCE.P({2},{5})"), 0.0);
        assert_eq!(err("COVAR({1,2},{1,2,3})"), ErrorKind::NA);
        close("RSQ({2,3,9,1,8,7,5},{6,5,11,7,5,4,4})", 0.05795019157088123);
        // Pairs with text or blanks on either side are dropped.
        let mut w = wb(&[
            ("A1", "3"),
            ("A2", "2"),
            ("A3", "x"),
            ("A4", "4"),
            ("A5", "5"),
            ("A6", "6"),
            ("B1", "9"),
            ("B2", "7"),
            ("B3", "100"),
            ("B4", "12"),
            ("B5", "15"),
            ("B6", "17"),
        ]);
        near(
            calc_num(&mut w, "CORREL(A1:A6,B1:B6)"),
            0.9970544855015815,
            1e-12,
            "CORREL",
        );
        near(calc_num(&mut w, "COVAR(A1:A6,B1:B6)"), 5.2, 1e-12, "COVAR");
        assert_eq!(
            calc(&mut w, "CORREL(A1:A6,B1:B5)"),
            Value::Error(ErrorKind::NA)
        );
    }

    #[test]
    fn simple_regression() {
        let (y, x) = ("{2,3,9,1,8,7,5}", "{6,5,11,7,5,4,4}");
        close(&format!("SLOPE({y},{x})"), 0.3055555555555556);
        close(&format!("INTERCEPT({y},{x})"), 3.1666666666666665);
        close(&format!("STEYX({y},{x})"), 3.305718950210041);
        close("INTERCEPT({2,3,9,1,8},{6,5,11,7,5})", 0.04838709677419306);
        close(
            "FORECAST(30,{6,7,9,15,21},{20,28,31,38,40})",
            10.607253086419755,
        );
        close(
            "FORECAST.LINEAR(30,{6,7,9,15,21},{20,28,31,38,40})",
            10.607253086419755,
        );
        assert_eq!(num("SLOPE({3,5,7},{1,2,3})"), 2.0);
        assert_eq!(num("INTERCEPT({3,5,7},{1,2,3})"), 1.0);
        assert_eq!(num("STEYX({3,5,7},{1,2,3})"), 0.0);
        assert_eq!(num("SUM(FORECAST({4,5},{3,5,7},{1,2,3}))"), 20.0);
        assert_eq!(err("SLOPE({1,2,3},{4,4,4})"), ErrorKind::Div0);
        assert_eq!(err("SLOPE({1,2,3},{4,5})"), ErrorKind::NA);
        assert_eq!(err("STEYX({1,2},{4,5})"), ErrorKind::Div0);
        assert_eq!(err("FORECAST(\"x\",{3,5,7},{1,2,3})"), ErrorKind::Value);
        assert_eq!(err("FORECAST(1,{3,5,7},{2,2,2})"), ErrorKind::Div0);
    }

    #[test]
    fn linest() {
        assert_eq!(
            spill_nums("LINEST({1,9,5,7},{0,4,2,3})", 1, 2),
            vec![vec![2.0, 1.0]]
        );
        assert_eq!(spill_nums("LINEST({3,5,7})", 1, 2), vec![vec![2.0, 1.0]]);
        assert_eq!(
            spill_nums("LINEST({1;9;5;7},{0;4;2;3})", 1, 2),
            vec![vec![2.0, 1.0]]
        );
        assert_eq!(
            spill_nums("LINEST({2,4,6},{1,2,3},FALSE)", 1, 2),
            vec![vec![2.0, 0.0]]
        );
        assert_eq!(err("LINEST({1,2,3},{1,2})"), ErrorKind::Ref);
        assert_eq!(err("LINEST({1,\"a\",3},{1,2,3})"), ErrorKind::Value);
        // Full statistics.
        let s = spill("LINEST({2,4,5,4,5},{1,2,3,4,5},TRUE,TRUE)", 5, 2);
        let n = |r: usize, c: usize| match &s[r][c] {
            Value::Number(n) => *n,
            other => panic!("LINEST stats [{r},{c}] is {other:?}"),
        };
        near(n(0, 0), 0.6, 1e-12, "slope");
        near(n(0, 1), 2.2, 1e-12, "intercept");
        near(n(1, 0), 0.282842712474619, 1e-12, "se slope");
        near(n(1, 1), 0.938083151964686, 1e-12, "se intercept");
        near(n(2, 0), 0.6, 1e-12, "r2");
        near(n(2, 1), 0.894427190999916, 1e-12, "se y");
        near(n(3, 0), 4.5, 1e-12, "F");
        near(n(3, 1), 3.0, 1e-12, "df");
        near(n(4, 0), 3.6, 1e-12, "ss reg");
        near(n(4, 1), 2.4, 1e-12, "ss resid");
        // Without a constant the standard error of b is not available.
        let s = spill("LINEST({2,4,6.5},{1,2,3},FALSE,TRUE)", 5, 2);
        assert_eq!(s[0][1], Value::Number(0.0));
        assert_eq!(s[1][1], Value::Error(ErrorKind::NA));
        assert_eq!(s[3][1], Value::Number(2.0));
    }

    #[test]
    fn multiple_regression() {
        // y = 2*x1 + 3*x2 + 5
        let w = wb(&[
            ("A1", "1"),
            ("B1", "4"),
            ("C1", "19"),
            ("A2", "2"),
            ("B2", "1"),
            ("C2", "12"),
            ("A3", "3"),
            ("B3", "7"),
            ("C3", "32"),
            ("A4", "4"),
            ("B4", "2"),
            ("C4", "19"),
            ("A5", "5"),
            ("B5", "9"),
            ("C5", "42"),
            ("A7", "10"),
            ("B7", "10"),
            ("A8", "0"),
            ("B8", "1"),
            ("E1", "=LINEST(C1:C5,A1:B5)"),
            ("E3", "=TREND(C1:C5,A1:B5,A7:B8)"),
            ("E6", "=SUM(TREND(C1:C5,A1:B5))"),
            ("E7", "=LINEST(C1:C5,A1:B4)"),
            ("E8", "=TREND(C1:C5,A1:B5,A7:A8)"),
        ]);
        near(cell(&w, "E1"), 3.0, 1e-9, "m2");
        near(cell(&w, "F1"), 2.0, 1e-9, "m1");
        near(cell(&w, "G1"), 5.0, 1e-9, "b");
        near(cell(&w, "E3"), 55.0, 1e-9, "trend 1");
        near(cell(&w, "E4"), 8.0, 1e-9, "trend 2");
        near(cell(&w, "E6"), 124.0, 1e-9, "trend sum");
        assert_eq!(val(&w, "E7"), Value::Error(ErrorKind::Ref));
        assert_eq!(val(&w, "E8"), Value::Error(ErrorKind::Ref));
        // The same data laid out in rows.
        let w = wb(&[
            ("A1", "1"),
            ("B1", "2"),
            ("C1", "3"),
            ("D1", "4"),
            ("E1", "5"),
            ("A2", "4"),
            ("B2", "1"),
            ("C2", "7"),
            ("D2", "2"),
            ("E2", "9"),
            ("A3", "19"),
            ("B3", "12"),
            ("C3", "32"),
            ("D3", "19"),
            ("E3", "42"),
            ("G1", "10"),
            ("G2", "10"),
            ("A5", "=LINEST(A3:E3,A1:E2)"),
            ("A6", "=TREND(A3:E3,A1:E2,G1:G2)"),
        ]);
        near(cell(&w, "A5"), 3.0, 1e-9, "m2");
        near(cell(&w, "B5"), 2.0, 1e-9, "m1");
        near(cell(&w, "C5"), 5.0, 1e-9, "b");
        near(cell(&w, "A6"), 55.0, 1e-9, "trend");
        // A redundant variable gets a zero coefficient.
        let w = wb(&[
            ("A1", "1"),
            ("B1", "2"),
            ("C1", "5"),
            ("A2", "2"),
            ("B2", "4"),
            ("C2", "7"),
            ("A3", "3"),
            ("B3", "6"),
            ("C3", "9"),
            ("A4", "4"),
            ("B4", "8"),
            ("C4", "11"),
            ("E1", "=LINEST(C1:C4,A1:B4)"),
        ]);
        near(cell(&w, "E1"), 0.0, 1e-9, "redundant");
        near(cell(&w, "F1"), 2.0, 1e-9, "m1");
        near(cell(&w, "G1"), 3.0, 1e-9, "b");
    }

    #[test]
    fn trend_growth() {
        let t = spill_nums("TREND({1,9,5,7},{0,4,2,3},{5,6})", 1, 2);
        near(t[0][0], 11.0, 1e-12, "trend");
        near(t[0][1], 13.0, 1e-12, "trend");
        let t = spill_nums("TREND({3,5,7})", 1, 3);
        near(t[0][0], 3.0, 1e-12, "trend");
        near(t[0][2], 7.0, 1e-12, "trend");
        close("TREND({3,5,7},,4)", 9.0);
        close("TREND({2,4,6},{1,2,3},4,FALSE)", 8.0);
        close("TREND({3,5,7},{1,2,3},10)", 21.0);
        let t = spill_nums("TREND({3;5;7},{1;2;3},{4,5;6,7})", 2, 2);
        near(t[1][1], 15.0, 1e-12, "trend");
        let (y, x) = (
            "{33100,47300,69000,102000,150000,220000}",
            "{11,12,13,14,15,16}",
        );
        let g = spill_nums(&format!("GROWTH({y},{x},{{17,18}})"), 1, 2);
        near(g[0][0], 320196.7183634738, 1e-9, "growth");
        near(g[0][1], 468536.05418404966, 1e-9, "growth");
        let g = spill_nums(&format!("GROWTH({y},{x})"), 1, 6);
        near(g[0][0], 32618.20377353983, 1e-9, "growth");
        close("GROWTH({2,4,8},{1,2,3},4)", 16.0);
        close("GROWTH({2,4,8},,5)", 32.0);
        assert_eq!(err("GROWTH({2,0,8},{1,2,3},4)"), ErrorKind::Num);
        assert_eq!(err("TREND({1,2,3},{1,2},4)"), ErrorKind::Ref);
    }

    #[test]
    fn means() {
        close("GEOMEAN(4,5,8,7,11,4,3)", 5.476986969656962);
        assert_eq!(num("GEOMEAN(4,9)"), 6.0);
        assert_eq!(num("GEOMEAN(2,4,8)"), 4.0);
        close("GEOMEAN(1E200,1E200,1E-100)", 1e100);
        assert_eq!(err("GEOMEAN(4,0)"), ErrorKind::Num);
        assert_eq!(err("GEOMEAN(4,-1)"), ErrorKind::Num);
        assert_eq!(err("GEOMEAN(C1:C3)"), ErrorKind::Num);
        close("HARMEAN(4,5,8,7,11,4,3)", 5.028375962061728);
        assert_eq!(num("HARMEAN(1,1,1)"), 1.0);
        assert_eq!(err("HARMEAN(4,0)"), ErrorKind::Num);
        assert_eq!(err("HARMEAN(C1:C3)"), ErrorKind::NA);
        close("TRIMMEAN({4,5,6,7,2,3,4,5,1,2,3},0.2)", 3.7777777777777777);
        assert_eq!(num("TRIMMEAN({1,2,3,4,100},0.4)"), 3.0);
        assert_eq!(num("TRIMMEAN({1,2,3,4,100},0)"), 22.0);
        assert_eq!(num("TRIMMEAN({1,2,3,4,100},0.39)"), 22.0);
        assert_eq!(err("TRIMMEAN({1,2,3},1)"), ErrorKind::Num);
        assert_eq!(err("TRIMMEAN({1,2,3},-0.1)"), ErrorKind::Num);
        assert_eq!(err("TRIMMEAN(C1:C3,0.1)"), ErrorKind::Num);
    }

    #[test]
    fn deviations_and_shape() {
        close("AVEDEV(4,5,6,7,5,4,3)", 1.0204081632653061);
        assert_eq!(err("AVEDEV(C1:C3)"), ErrorKind::Num);
        assert_eq!(num("DEVSQ(4,5,8,7,11,4,3)"), 48.0);
        assert_eq!(num("DEVSQ(5)"), 0.0);
        assert_eq!(err("DEVSQ(C1:C3)"), ErrorKind::Num);
        close("SKEW(3,4,5,2,3,4,5,6,4,7)", 0.3595430714067978);
        assert_eq!(err("SKEW(1,2)"), ErrorKind::Div0);
        assert_eq!(err("SKEW(2,2,2)"), ErrorKind::Div0);
        close("KURT(3,4,5,2,3,4,5,6,4,7)", -0.15179963720841272);
        assert_eq!(err("KURT(1,2,3)"), ErrorKind::Div0);
        assert_eq!(err("KURT(2,2,2,2)"), ErrorKind::Div0);
        close("STANDARDIZE(42,40,1.5)", 1.3333333333333333);
        assert_eq!(err("STANDARDIZE(42,40,0)"), ErrorKind::Num);
        assert_eq!(err("STANDARDIZE(42,40,-1)"), ErrorKind::Num);
    }

    #[test]
    fn special_function_kernels() {
        for (z, expected) in [
            (-37.0, 5.725571222525138e-300),
            (-20.0, 2.7536241186063314e-89),
            (-10.0, 7.619853024160593e-24),
            (-8.0, 6.22096057427182e-16),
            (-3.0, 0.0013498980316300957),
            (-1.0, 0.15865525393145705),
            (-0.5, 0.30853753872598694),
            (0.0, 0.5),
            (0.5, 0.691462461274013),
            (1.0, 0.8413447460685429),
            (1.2247, 0.8896558629290543),
            (1.23, 0.890651447574308),
            (2.0, 0.9772498680518208),
            (3.0, 0.9986501019683699),
            (5.0, 0.9999997133484281),
        ] {
            near(norm_cdf(z), expected, 1e-13, &format!("norm_cdf({z})"));
            assert!(
                (norm_cdf(z) - expected).abs() <= 1e-13 * expected,
                "norm_cdf({z}) relative"
            );
        }
        for (p, expected) in [
            (1e-300, -37.0470962993612),
            (1e-10, -6.361340902404057),
            (0.001, -3.090232306167813),
            (0.05, -1.6448536269514722),
            (0.3, -0.5244005127080408),
            (0.5, 0.0),
            (0.5000001, 2.506628273311649e-7),
            (0.6, 0.2533471031357998),
            (0.9, 1.2815515655446006),
            (0.975, 1.9599639845400534),
            (0.99, 2.3263478740408408),
            (0.999, 3.090232306167813),
            (0.999999999, 5.997807019601638),
        ] {
            let got = norm_inv(p);
            assert!(
                (got - expected).abs() <= 1e-12 * expected.abs().max(1e-6),
                "norm_inv({p}): got {got}, expected {expected}"
            );
        }
        near(ln_gamma(0.5), 0.5723649429247004, 1e-13, "ln_gamma");
        near(ln_gamma(2.5), 0.2846828704729196, 1e-13, "ln_gamma");
        near(ln_gamma(100.5), 361.4355404677776, 1e-13, "ln_gamma");
        near(ln_gamma(1e-5), 11.512919692895824, 1e-13, "ln_gamma");
        assert_eq!(ln_gamma(1.0), 0.0);
        assert_eq!(ln_gamma(2.0), 0.0);
        near(gamma(0.5), 1.7724538509055159, 1e-13, "gamma");
        near(gamma(10.3), 716430.6890623765, 1e-13, "gamma");
        near(gamma(170.5) / 5.56209241456e305, 1.0, 1e-11, "gamma");
        assert_eq!(gamma(5.0), 24.0);
        assert!(gamma(171.7).is_infinite());
        assert!(gamma(-2.0).is_nan());
        // P(a, x) + Q(a, x) = 1, and known values.
        near(
            inc_gamma(1.0, 2.0).0,
            1.0 - (-2.0f64).exp(),
            1e-13,
            "inc_gamma",
        );
        near(
            inc_gamma(3.0, 5.0).1,
            0.12465201948308113,
            1e-13,
            "inc_gamma",
        );
        near(
            inc_gamma(2501.0, 2400.0).1,
            0.979362325026836,
            1e-10,
            "inc_gamma",
        );
        near(inc_beta(1.0, 1.0, 0.3), 0.3, 1e-13, "inc_beta");
        near(inc_beta(2.0, 3.0, 0.5), 0.6875, 1e-13, "inc_beta");
        near(inc_beta(4.0, 7.0, 0.5), 0.828125, 1e-13, "inc_beta");
    }

    #[test]
    fn normal_distribution() {
        close("NORM.DIST(42,40,1.5,TRUE)", 0.9087887802741321);
        close("NORM.DIST(42,40,1.5,FALSE)", 0.10934004978399575);
        close("NORMDIST(42,40,1.5,TRUE)", 0.9087887802741321);
        close("NORMDIST(42,40,1.5,0)", 0.10934004978399575);
        assert_eq!(num("NORM.DIST(40,40,1.5,TRUE)"), 0.5);
        assert_eq!(err("NORM.DIST(42,40,0,TRUE)"), ErrorKind::Num);
        assert_eq!(err("NORM.DIST(42,40,-1,TRUE)"), ErrorKind::Num);
        assert_eq!(err("NORM.DIST(\"x\",40,1,TRUE)"), ErrorKind::Value);
        close("NORM.INV(0.908789,40,1.5)", 42.00000200956616);
        close("NORMINV(0.908789,40,1.5)", 42.00000200956616);
        assert_eq!(num("NORM.INV(0.5,40,1.5)"), 40.0);
        assert_eq!(err("NORM.INV(0,40,1.5)"), ErrorKind::Num);
        assert_eq!(err("NORM.INV(1,40,1.5)"), ErrorKind::Num);
        assert_eq!(err("NORM.INV(0.5,40,0)"), ErrorKind::Num);
        close("NORM.S.DIST(1.333333,TRUE)", 0.9087887256040951);
        close("NORM.S.DIST(1.333333,FALSE)", 0.16401014756936722);
        close("NORMSDIST(1.333333)", 0.9087887256040951);
        assert_eq!(num("NORMSDIST(0)"), 0.5);
        close("NORMSDIST(-3)", 0.0013498980316300957);
        close("NORM.S.DIST(0,FALSE)", 0.3989422804014327);
        close("NORM.S.INV(0.908789)", 1.3333346730441071);
        close("NORMSINV(0.908789)", 1.3333346730441071);
        close("NORM.S.INV(0.975)", 1.9599639845400534);
        assert_eq!(num("NORMSINV(0.5)"), 0.0);
        close("NORMSINV(0.001)", -3.090232306167813);
        close("NORMSDIST(NORMSINV(0.123456))", 0.123456);
        assert_eq!(err("NORM.S.INV(0)"), ErrorKind::Num);
        assert_eq!(err("NORM.S.INV(1)"), ErrorKind::Num);
        assert_eq!(err("NORM.S.INV(-0.5)"), ErrorKind::Num);
        close("CONFIDENCE(0.05,2.5,50)", 0.6929519121748386);
        close("CONFIDENCE.NORM(0.05,2.5,50)", 0.6929519121748386);
        close("CONFIDENCE(0.05,2.5,50.9)", 0.6929519121748386);
        assert_eq!(err("CONFIDENCE(0,2.5,50)"), ErrorKind::Num);
        assert_eq!(err("CONFIDENCE(1,2.5,50)"), ErrorKind::Num);
        assert_eq!(err("CONFIDENCE(0.05,0,50)"), ErrorKind::Num);
        assert_eq!(err("CONFIDENCE(0.05,2.5,0.5)"), ErrorKind::Num);
    }

    #[test]
    fn discrete_distributions() {
        assert_eq!(num("BINOM.DIST(6,10,0.5,FALSE)"), 0.205078125);
        assert_eq!(num("BINOMDIST(6,10,0.5,FALSE)"), 0.205078125);
        close("BINOM.DIST(6,10,0.5,TRUE)", 0.828125);
        assert_eq!(num("BINOM.DIST(10,10,0.5,TRUE)"), 1.0);
        assert_eq!(num("BINOM.DIST(0,10,0,FALSE)"), 1.0);
        assert_eq!(num("BINOM.DIST(3,10,1,FALSE)"), 0.0);
        assert_eq!(num("BINOM.DIST(6.9,10.9,0.5,FALSE)"), 0.205078125);
        close("BINOM.DIST(60,200,0.25,FALSE)", 0.01708367092089676);
        close("BINOM.DIST(480,1000,0.5,TRUE)", 0.10872414660207047);
        close("BINOM.DIST(3000,10000,0.3,TRUE)", 0.504932983819297);
        assert_eq!(err("BINOM.DIST(11,10,0.5,FALSE)"), ErrorKind::Num);
        assert_eq!(err("BINOM.DIST(-1,10,0.5,FALSE)"), ErrorKind::Num);
        assert_eq!(err("BINOM.DIST(5,10,1.5,FALSE)"), ErrorKind::Num);
        assert_eq!(err("BINOM.DIST(5,10,-0.5,TRUE)"), ErrorKind::Num);
        close("POISSON.DIST(2,5,TRUE)", 0.12465201948308113);
        close("POISSON.DIST(2,5,FALSE)", 0.08422433748856833);
        close("POISSON(2,5,TRUE)", 0.12465201948308113);
        close("POISSON(2.9,5,FALSE)", 0.08422433748856833);
        close("POISSON.DIST(20,12.5,TRUE)", 0.9826918824345406);
        close("POISSON.DIST(200,180,FALSE)", 0.009651740527381398);
        close("POISSON.DIST(170,150,FALSE)", 0.0085223577140306);
        close("POISSON.DIST(1000,1000,FALSE)", 0.012614611348721499);
        close("POISSON.DIST(3,0.5,FALSE)", 0.012636055410679864);
        near(
            num("POISSON.DIST(170,699,FALSE)") / 1.3442963875025884e-127,
            1.0,
            1e-9,
            "POISSON tail",
        );
        close("POISSON.DIST(700,720,TRUE)", 0.23459448446916364);
        close("POISSON.DIST(2500,2400,TRUE)", 0.979362325026836);
        assert_eq!(num("POISSON.DIST(0,0,FALSE)"), 1.0);
        assert_eq!(num("POISSON.DIST(3,0,TRUE)"), 1.0);
        assert_eq!(err("POISSON.DIST(-1,5,TRUE)"), ErrorKind::Num);
        assert_eq!(err("POISSON.DIST(2,-5,TRUE)"), ErrorKind::Num);
        close("EXPON.DIST(0.2,10,TRUE)", 0.8646647167633873);
        close("EXPON.DIST(0.2,10,FALSE)", 1.353352832366127);
        close("EXPONDIST(0.2,10,TRUE)", 0.8646647167633873);
        assert_eq!(num("EXPON.DIST(0,3,TRUE)"), 0.0);
        assert_eq!(num("EXPON.DIST(0,3,FALSE)"), 3.0);
        assert_eq!(err("EXPON.DIST(-1,10,TRUE)"), ErrorKind::Num);
        assert_eq!(err("EXPON.DIST(1,0,TRUE)"), ErrorKind::Num);
    }

    #[test]
    fn fisher_gamma() {
        close("FISHER(0.75)", 0.9729550745276566);
        assert_eq!(num("FISHER(0)"), 0.0);
        assert_eq!(err("FISHER(1)"), ErrorKind::Num);
        assert_eq!(err("FISHER(-1)"), ErrorKind::Num);
        close("FISHERINV(0.972955)", 0.7499999673941484);
        close("FISHERINV(FISHER(0.3))", 0.3);
        close("GAMMALN(4)", 1.791759469228055);
        close("GAMMALN(0.5)", 0.5723649429247004);
        assert_eq!(num("GAMMALN(1)"), 0.0);
        assert_eq!(err("GAMMALN(0)"), ErrorKind::Num);
        assert_eq!(err("GAMMALN(-1)"), ErrorKind::Num);
        close("GAMMA(2.5)", 1.329340388179137);
        close("GAMMA(-3.75)", 0.2678661288614166);
        assert_eq!(num("GAMMA(5)"), 24.0);
        assert_eq!(num("GAMMA(1)"), 1.0);
        close("GAMMA(0.5)", 1.7724538509055159);
        assert_eq!(err("GAMMA(0)"), ErrorKind::Num);
        assert_eq!(err("GAMMA(-2)"), ErrorKind::Num);
        assert_eq!(err("GAMMA(172)"), ErrorKind::Num);
        // Scalar functions lift over arrays.
        close("SUM(GAMMA({1,2,3,4}))", 10.0);
    }
}
