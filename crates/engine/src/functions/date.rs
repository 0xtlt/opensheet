//! Date & time functions.
//!
//! Dates are serial day numbers in the 1900 date system (serial 60 is the
//! fictitious 29 February 1900); times are fractions of a day.

use super::{Category, FunctionDef, Kind};
use crate::ast::Expr;
use crate::datetime::{days_in_month, is_leap_year, serial_to_ymd, weekday};
use crate::eval::EvalCtx;
use crate::input::parse_date_time;
use crate::value::{ErrorKind, R, Value};

/// Serial of 9999-12-31, the last representable date.
const MAX_SERIAL: i64 = 2_958_465;

fn int_value(n: i64) -> R<Value> {
    Ok(Value::Number(n as f64))
}

// ---- calendar helpers ----------------------------------------------------

/// Days from 1899-12-30 to a proleptic Gregorian date (month in 1..=12).
fn civil_days(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468 + 25_569
}

/// Serial of a calendar date. Month and day may be out of range and roll
/// over; the day count runs through the fictitious 1900-02-29, so
/// (1900, 2, 29) is 60 and (1900, 3, 0) is 60 as well. Not range-checked.
fn ymd_serial(year: i64, month: i64, day: i64) -> i64 {
    let months = year * 12 + (month - 1);
    let first = civil_days(months.div_euclid(12), months.rem_euclid(12) + 1, 1);
    // Month starts up to February 1900 sit one below the real day count.
    let first = if first < 61 { first - 1 } else { first };
    first + day - 1
}

fn ymd(serial: i64) -> (i64, i64, i64) {
    let (y, m, d) = serial_to_ymd(serial as f64);
    (y, m as i64, d as i64)
}

/// Month length as the 1900 date system sees it (February 1900 has 29 days).
fn month_len(y: i64, m: i64) -> i64 {
    if y == 1900 && m == 2 {
        29
    } else {
        days_in_month(y, m as u32) as i64
    }
}

fn check_serial(serial: i64) -> R<i64> {
    if (0..=MAX_SERIAL).contains(&serial) {
        Ok(serial)
    } else {
        Err(ErrorKind::Num)
    }
}

/// Whole-day part of a serial; `#NUM!` outside 0..=9999-12-31.
fn to_day(n: f64) -> R<i64> {
    if n >= 0.0 && n < (MAX_SERIAL + 1) as f64 {
        Ok(n.floor() as i64)
    } else {
        Err(ErrorKind::Num)
    }
}

/// Evaluate a date argument (numbers, or text holding a date).
fn date_arg(ctx: &EvalCtx, e: &Expr) -> R<i64> {
    to_day(ctx.num(e)?)
}

/// Optional integer argument, truncated toward zero.
fn int_or(ctx: &EvalCtx, args: &[Expr], index: usize, default: i64) -> R<i64> {
    match args.get(index) {
        None | Some(Expr::Missing) => Ok(default),
        Some(e) => ctx.int(e),
    }
}

/// Day of week of a serial: 0 = Sunday .. 6 = Saturday.
fn dow(serial: i64) -> i64 {
    weekday(serial as f64) as i64
}

/// Day of week of a serial: 0 = Monday .. 6 = Sunday.
fn dow_monday(serial: i64) -> usize {
    ((dow(serial) + 6) % 7) as usize
}

// ---- current date --------------------------------------------------------

fn f_today(ctx: &EvalCtx, _args: &[Expr]) -> R<Value> {
    Ok(Value::Number(ctx.now().floor()))
}

fn f_now(ctx: &EvalCtx, _args: &[Expr]) -> R<Value> {
    Ok(Value::Number(ctx.now()))
}

// ---- building dates and times --------------------------------------------

fn f_date(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let year = ctx.int(&args[0])?;
    let month = ctx.int(&args[1])?;
    let day = ctx.int(&args[2])?;
    if !(0..=9999).contains(&year) {
        return Err(ErrorKind::Num);
    }
    // Far outside any representable date (also keeps the arithmetic small).
    if month.abs() > 1_000_000_000 || day.abs() > 1_000_000_000_000 {
        return Err(ErrorKind::Num);
    }
    let year = if year < 1900 { year + 1900 } else { year };
    int_value(check_serial(ymd_serial(year, month, day))?)
}

fn f_datevalue(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    match ctx.eval_scalar(&args[0]) {
        Value::Text(s) => {
            let (n, _) = parse_date_time(&s, ctx.locale()).ok_or(ErrorKind::Value)?;
            Ok(Value::Number(n.floor()))
        }
        Value::Error(e) => Err(e),
        _ => Err(ErrorKind::Value),
    }
}

fn f_time(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let mut total = 0i64;
    for (arg, unit) in args.iter().zip([3600i64, 60, 1]) {
        let n = ctx.int(arg)?;
        if !(-32_768..=32_767).contains(&n) {
            return Err(ErrorKind::Num);
        }
        total += n * unit;
    }
    if total < 0 {
        return Err(ErrorKind::Num);
    }
    Ok(Value::Number((total % 86_400) as f64 / 86_400.0))
}

fn f_timevalue(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    match ctx.eval_scalar(&args[0]) {
        Value::Text(s) => {
            let (n, _) = parse_date_time(&s, ctx.locale()).ok_or(ErrorKind::Value)?;
            Ok(Value::Number(n - n.floor()))
        }
        Value::Error(e) => Err(e),
        _ => Err(ErrorKind::Value),
    }
}

// ---- taking dates and times apart ----------------------------------------

fn f_year(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    int_value(ymd(date_arg(ctx, &args[0])?).0)
}

fn f_month(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    int_value(ymd(date_arg(ctx, &args[0])?).1)
}

fn f_day(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    int_value(ymd(date_arg(ctx, &args[0])?).2)
}

/// Second of the day, rounded to the nearest second; a time that rounds up
/// to midnight wraps to 0.
fn second_of_day(ctx: &EvalCtx, e: &Expr) -> R<i64> {
    let n = ctx.num(e)?;
    to_day(n)?;
    let frac = n - n.floor();
    Ok((frac * 86_400.0).round() as i64 % 86_400)
}

fn f_hour(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    int_value(second_of_day(ctx, &args[0])? / 3600)
}

fn f_minute(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    int_value(second_of_day(ctx, &args[0])? % 3600 / 60)
}

fn f_second(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    int_value(second_of_day(ctx, &args[0])? % 60)
}

/// First day of the week (0 = Sunday .. 6 = Saturday) for the numbering
/// types shared by `WEEKDAY` and `WEEKNUM`.
fn week_start(return_type: i64) -> Option<i64> {
    match return_type {
        1 | 17 => Some(0),
        2 | 11 => Some(1),
        12..=16 => Some(return_type - 10),
        _ => None,
    }
}

fn f_weekday(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let serial = date_arg(ctx, &args[0])?;
    let return_type = int_or(ctx, args, 1, 1)?;
    let wd = dow(serial);
    if return_type == 3 {
        // Monday = 0 .. Sunday = 6
        return int_value((wd + 6) % 7);
    }
    let start = week_start(return_type).ok_or(ErrorKind::Num)?;
    int_value((wd - start + 7) % 7 + 1)
}

fn iso_week(serial: i64) -> i64 {
    // The ISO week belongs to the year holding its Thursday.
    let thursday = serial - dow_monday(serial) as i64 + 3;
    if thursday < 1 {
        return 52;
    }
    let jan1 = ymd_serial(ymd(thursday).0, 1, 1);
    (thursday - jan1) / 7 + 1
}

fn f_weeknum(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let serial = date_arg(ctx, &args[0])?;
    let return_type = int_or(ctx, args, 1, 1)?;
    if return_type == 21 {
        return int_value(iso_week(serial));
    }
    let start = week_start(return_type).ok_or(ErrorKind::Num)?;
    // Week 1 is the week containing 1 January.
    let jan1 = ymd_serial(ymd(serial).0, 1, 1);
    let offset = (dow(jan1) - start + 7) % 7;
    int_value((serial - jan1 + offset).div_euclid(7) + 1)
}

fn f_isoweeknum(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    int_value(iso_week(date_arg(ctx, &args[0])?))
}

// ---- date arithmetic -----------------------------------------------------

/// Move a date by whole months, keeping the day (clamped to the month's
/// length) or snapping to the last day of the month.
fn shift_months(serial: i64, months: i64, to_month_end: bool) -> R<i64> {
    if months.abs() > 1_000_000 {
        return Err(ErrorKind::Num);
    }
    let (y, m, d) = ymd(serial);
    let total = y * 12 + (m - 1) + months;
    let (ny, nm) = (total.div_euclid(12), total.rem_euclid(12) + 1);
    let last = month_len(ny, nm);
    let day = if to_month_end { last } else { d.min(last) };
    check_serial(ymd_serial(ny, nm, day))
}

fn f_edate(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let serial = date_arg(ctx, &args[0])?;
    let months = ctx.int(&args[1])?;
    int_value(shift_months(serial, months, false)?)
}

fn f_eomonth(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let serial = date_arg(ctx, &args[0])?;
    let months = ctx.int(&args[1])?;
    int_value(shift_months(serial, months, true)?)
}

fn f_datedif(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let start = date_arg(ctx, &args[0])?;
    let end = date_arg(ctx, &args[1])?;
    let unit = ctx.text(&args[2])?.to_ascii_uppercase();
    if start > end {
        return Err(ErrorKind::Num);
    }
    let (y1, m1, d1) = ymd(start);
    let (y2, m2, d2) = ymd(end);
    let months = (y2 - y1) * 12 + (m2 - m1) - i64::from(d2 < d1);
    int_value(match unit.as_str() {
        "D" => end - start,
        "M" => months,
        "Y" => months / 12,
        "YM" => months % 12,
        "MD" => {
            if d2 >= d1 {
                d2 - d1
            } else {
                // Borrow the length of the month before the end date's
                // month (this reproduces Excel's well-known "MD" results).
                let (py, pm) = if m2 == 1 { (y2 - 1, 12) } else { (y2, m2 - 1) };
                d2 - d1 + month_len(py, pm)
            }
        }
        "YD" => {
            // Start date moved into the end date's year (or the one before).
            let mut anchor = ymd_serial(y2, m1, d1);
            if anchor > end {
                anchor = ymd_serial(y2 - 1, m1, d1);
            }
            end - anchor
        }
        _ => return Err(ErrorKind::Num),
    })
}

fn f_days(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let end = date_arg(ctx, &args[0])?;
    let start = date_arg(ctx, &args[1])?;
    int_value(end - start)
}

fn days360(start: i64, end: i64, european: bool) -> i64 {
    let (y1, m1, mut d1) = ymd(start);
    let (y2, m2, mut d2) = ymd(end);
    if european {
        d1 = d1.min(30);
        d2 = d2.min(30);
    } else {
        // US (NASD): a start date on the last day of its month counts as
        // the 30th; an end date on the 31st counts as the 30th only when
        // the start date is (now) the 30th.
        if d1 == 31 || (m1 == 2 && d1 == month_len(y1, 2)) {
            d1 = 30;
        }
        if d2 == 31 && d1 == 30 {
            d2 = 30;
        }
    }
    (y2 - y1) * 360 + (m2 - m1) * 30 + (d2 - d1)
}

fn f_days360(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let start = date_arg(ctx, &args[0])?;
    let end = date_arg(ctx, &args[1])?;
    let european = ctx.bool_or(args, 2, false)?;
    int_value(days360(start, end, european))
}

fn yearfrac(start: i64, end: i64, basis: i64) -> R<f64> {
    if !(0..=4).contains(&basis) {
        return Err(ErrorKind::Num);
    }
    let (start, end) = if start > end {
        (end, start)
    } else {
        (start, end)
    };
    if start == end {
        return Ok(0.0);
    }
    let (y1, m1, d1) = ymd(start);
    let (y2, m2, d2) = ymd(end);
    let days = (end - start) as f64;
    let days_30_360 = |a: i64, b: i64| ((y2 - y1) * 360 + (m2 - m1) * 30 + (b - a)) as f64;
    Ok(match basis {
        0 => {
            // US (NASD) 30/360.
            let feb_end1 = m1 == 2 && d1 == month_len(y1, 2);
            let feb_end2 = m2 == 2 && d2 == month_len(y2, 2);
            let (mut a, mut b) = (d1, d2);
            if a == 31 && b == 31 {
                a = 30;
                b = 30;
            } else if a == 31 {
                a = 30;
            } else if a == 30 && b == 31 {
                b = 30;
            } else if feb_end1 && feb_end2 {
                a = 30;
                b = 30;
            } else if feb_end1 {
                a = 30;
            }
            days_30_360(a, b) / 360.0
        }
        1 => {
            // Actual/actual.
            let within_a_year = y1 == y2 || (y1 + 1 == y2 && (m1 > m2 || (m1 == m2 && d1 >= d2)));
            if within_a_year {
                let year_len = if y1 == y2 {
                    if is_leap_year(y1) { 366.0 } else { 365.0 }
                } else {
                    // The span covers a 29 February.
                    let mar1_y1 = ymd_serial(y1, 3, 1);
                    let mar1_y2 = ymd_serial(y2, 3, 1);
                    let crosses = (is_leap_year(y1) && start < mar1_y1 && end >= mar1_y1)
                        || (is_leap_year(y2) && end >= mar1_y2 && start < mar1_y2)
                        || (m2 == 2 && d2 == 29);
                    if crosses { 366.0 } else { 365.0 }
                };
                days / year_len
            } else {
                // Average length of the calendar years touched.
                let years = (y2 - y1 + 1) as f64;
                let span = (ymd_serial(y2 + 1, 1, 1) - ymd_serial(y1, 1, 1)) as f64;
                days / (span / years)
            }
        }
        2 => days / 360.0,
        3 => days / 365.0,
        _ => days_30_360(d1.min(30), d2.min(30)) / 360.0,
    })
}

fn f_yearfrac(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let start = date_arg(ctx, &args[0])?;
    let end = date_arg(ctx, &args[1])?;
    let basis = int_or(ctx, args, 2, 0)?;
    Ok(Value::Number(yearfrac(start, end, basis)?))
}

// ---- working days --------------------------------------------------------

/// `mask[i]` is true when weekday `i` (0 = Monday .. 6 = Sunday) is a
/// non-working day.
type Weekend = [bool; 7];

const SAT_SUN: Weekend = [false, false, false, false, false, true, true];

/// Parse the `weekend` argument of the `.INTL` functions: a number
/// (1-7 two-day weekends, 11-17 single days) or a 7-character string of
/// 0/1 starting on Monday.
fn weekend_arg(ctx: &EvalCtx, arg: Option<&Expr>) -> R<Weekend> {
    let e = match arg {
        None | Some(Expr::Missing) => return Ok(SAT_SUN),
        Some(e) => e,
    };
    match ctx.eval_scalar(e) {
        Value::Error(err) => Err(err),
        Value::Text(s) => {
            let mut mask = [false; 7];
            if s.chars().count() != 7 {
                return Err(ErrorKind::Value);
            }
            for (slot, ch) in mask.iter_mut().zip(s.chars()) {
                *slot = match ch {
                    '0' => false,
                    '1' => true,
                    _ => return Err(ErrorKind::Value),
                };
            }
            Ok(mask)
        }
        v => {
            let n = ctx.to_number(&v)?.trunc();
            let mut mask = [false; 7];
            if (1.0..=7.0).contains(&n) {
                // 1 = Saturday+Sunday, 2 = Sunday+Monday, ...
                let n = n as usize;
                mask[(n + 4) % 7] = true;
                mask[(n + 5) % 7] = true;
            } else if (11.0..=17.0).contains(&n) {
                // 11 = Sunday only, 12 = Monday only, ...
                mask[(n as usize + 2) % 7] = true;
            } else {
                return Err(ErrorKind::Num);
            }
            Ok(mask)
        }
    }
}

/// Holiday dates that fall on working days, sorted and de-duplicated.
fn holidays_arg(ctx: &EvalCtx, arg: Option<&Expr>, weekend: &Weekend) -> R<Vec<i64>> {
    let e = match arg {
        None | Some(Expr::Missing) => return Ok(Vec::new()),
        Some(e) => e,
    };
    let mut out = Vec::new();
    for v in ctx.flatten(std::slice::from_ref(e)) {
        let day = match v {
            Value::Empty | Value::Array(_) => continue,
            Value::Number(n) => to_day(n)?,
            Value::Text(_) => to_day(ctx.to_number(&v)?)?,
            Value::Bool(_) => return Err(ErrorKind::Value),
            Value::Error(err) => return Err(err),
        };
        if !weekend[dow_monday(day)] {
            out.push(day);
        }
    }
    out.sort_unstable();
    out.dedup();
    Ok(out)
}

/// Number of holidays in the inclusive range `lo..=hi`.
fn holidays_between(holidays: &[i64], lo: i64, hi: i64) -> i64 {
    if lo > hi {
        return 0;
    }
    (holidays.partition_point(|h| *h <= hi) - holidays.partition_point(|h| *h < lo)) as i64
}

/// Working days in `start..=end` (negative when the dates are reversed).
fn count_workdays(start: i64, end: i64, weekend: &Weekend, holidays: &[i64]) -> i64 {
    if start > end {
        return -count_workdays(end, start, weekend, holidays);
    }
    let per_week = weekend.iter().filter(|w| !**w).count() as i64;
    let weeks = (end - start + 1) / 7;
    let mut count = weeks * per_week;
    for day in start + weeks * 7..=end {
        if !weekend[dow_monday(day)] {
            count += 1;
        }
    }
    count - holidays_between(holidays, start, end)
}

/// The date `days` working days after (or before) `start`.
fn add_workdays(start: i64, days: i64, weekend: &Weekend, holidays: &[i64]) -> R<i64> {
    let per_week = weekend.iter().filter(|w| !**w).count() as i64;
    if per_week == 0 {
        return Err(ErrorKind::Value);
    }
    let dir = days.signum();
    let mut remaining = days.abs();
    let mut cur = start;
    while remaining > 0 {
        // Jump whole weeks while more than a week's worth of days is left,
        // then walk day by day.
        let weeks = (remaining - 1) / per_week;
        if weeks > 0 {
            if weeks > MAX_SERIAL {
                return Err(ErrorKind::Num);
            }
            let next = check_serial(cur + dir * weeks * 7)?;
            let skipped = if dir > 0 {
                holidays_between(holidays, cur + 1, next)
            } else {
                holidays_between(holidays, next, cur - 1)
            };
            remaining -= weeks * per_week - skipped;
            cur = next;
        } else {
            cur = check_serial(cur + dir)?;
            if !weekend[dow_monday(cur)] && holidays.binary_search(&cur).is_err() {
                remaining -= 1;
            }
        }
    }
    Ok(cur)
}

fn f_networkdays(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let start = date_arg(ctx, &args[0])?;
    let end = date_arg(ctx, &args[1])?;
    let holidays = holidays_arg(ctx, args.get(2), &SAT_SUN)?;
    int_value(count_workdays(start, end, &SAT_SUN, &holidays))
}

fn f_networkdays_intl(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let start = date_arg(ctx, &args[0])?;
    let end = date_arg(ctx, &args[1])?;
    let weekend = weekend_arg(ctx, args.get(2))?;
    let holidays = holidays_arg(ctx, args.get(3), &weekend)?;
    int_value(count_workdays(start, end, &weekend, &holidays))
}

fn f_workday(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let start = date_arg(ctx, &args[0])?;
    let days = ctx.int(&args[1])?;
    let holidays = holidays_arg(ctx, args.get(2), &SAT_SUN)?;
    int_value(add_workdays(start, days, &SAT_SUN, &holidays)?)
}

fn f_workday_intl(ctx: &EvalCtx, args: &[Expr]) -> R<Value> {
    let start = date_arg(ctx, &args[0])?;
    let days = ctx.int(&args[1])?;
    let weekend = weekend_arg(ctx, args.get(2))?;
    let holidays = holidays_arg(ctx, args.get(3), &weekend)?;
    int_value(add_workdays(start, days, &weekend, &holidays)?)
}

#[rustfmt::skip]
pub const FUNCTIONS: &[FunctionDef] = &[
    FunctionDef::new("TODAY", 0, 0, Kind::Raw, Category::DateTime, f_today),
    FunctionDef::new("NOW", 0, 0, Kind::Raw, Category::DateTime, f_now),
    FunctionDef::new("DATE", 3, 3, Kind::Scalar, Category::DateTime, f_date),
    FunctionDef::new("DATEVALUE", 1, 1, Kind::Scalar, Category::DateTime, f_datevalue),
    FunctionDef::new("TIME", 3, 3, Kind::Scalar, Category::DateTime, f_time),
    FunctionDef::new("TIMEVALUE", 1, 1, Kind::Scalar, Category::DateTime, f_timevalue),
    FunctionDef::new("YEAR", 1, 1, Kind::Scalar, Category::DateTime, f_year),
    FunctionDef::new("MONTH", 1, 1, Kind::Scalar, Category::DateTime, f_month),
    FunctionDef::new("DAY", 1, 1, Kind::Scalar, Category::DateTime, f_day),
    FunctionDef::new("HOUR", 1, 1, Kind::Scalar, Category::DateTime, f_hour),
    FunctionDef::new("MINUTE", 1, 1, Kind::Scalar, Category::DateTime, f_minute),
    FunctionDef::new("SECOND", 1, 1, Kind::Scalar, Category::DateTime, f_second),
    FunctionDef::new("WEEKDAY", 1, 2, Kind::Scalar, Category::DateTime, f_weekday),
    FunctionDef::new("WEEKNUM", 1, 2, Kind::Scalar, Category::DateTime, f_weeknum),
    FunctionDef::new("ISOWEEKNUM", 1, 1, Kind::Scalar, Category::DateTime, f_isoweeknum),
    FunctionDef::new("EDATE", 2, 2, Kind::Scalar, Category::DateTime, f_edate),
    FunctionDef::new("EOMONTH", 2, 2, Kind::Scalar, Category::DateTime, f_eomonth),
    FunctionDef::new("DATEDIF", 3, 3, Kind::Scalar, Category::DateTime, f_datedif),
    FunctionDef::new("DAYS", 2, 2, Kind::Scalar, Category::DateTime, f_days),
    FunctionDef::new("DAYS360", 2, 3, Kind::Scalar, Category::DateTime, f_days360),
    FunctionDef::new("NETWORKDAYS", 2, 3, Kind::Raw, Category::DateTime, f_networkdays),
    FunctionDef::new("NETWORKDAYS.INTL", 2, 4, Kind::Raw, Category::DateTime, f_networkdays_intl),
    FunctionDef::new("WORKDAY", 2, 3, Kind::Raw, Category::DateTime, f_workday),
    FunctionDef::new("WORKDAY.INTL", 2, 4, Kind::Raw, Category::DateTime, f_workday_intl),
    FunctionDef::new("YEARFRAC", 2, 3, Kind::Scalar, Category::DateTime, f_yearfrac),
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

    fn close(formula: &str, expected: f64) {
        let got = num(formula);
        assert!(
            (got - expected).abs() < 1e-7,
            "{formula} gave {got}, expected {expected}"
        );
    }

    #[test]
    fn today_and_now() {
        let today = num("TODAY()");
        let now = num("NOW()");
        assert_eq!(today, today.floor());
        assert!(today > 45_000.0);
        assert!(now >= today && now < today + 1.0);
    }

    #[test]
    fn date() {
        assert_eq!(num("DATE(2024,1,1)"), 45292.0);
        assert_eq!(num("DATE(2024,2,29)"), 45351.0);
        // Day overflow rolls into March.
        assert_eq!(num("DATE(2024,2,30)"), 45352.0);
        assert_eq!(num("DATE(2023,2,30)"), num("DATE(2023,3,2)"));
        // Examples from the Excel documentation.
        assert_eq!(num("DATE(108,1,2)"), 39449.0);
        assert_eq!(num("DATE(2008,14,2)"), 39846.0);
        assert_eq!(num("DATE(2008,-3,2)"), 39327.0);
        assert_eq!(num("DATE(2008,1,35)"), 39482.0);
        assert_eq!(num("DATE(2008,1,-15)"), 39432.0);
        assert_eq!(num("DATE(2024,13,1)"), num("DATE(2025,1,1)"));
        assert_eq!(num("DATE(2024,3,0)"), 45351.0);
        assert_eq!(num("DATE(2024.9,1.9,1.9)"), 45292.0);
        // Years below 1900 are offsets from 1900.
        assert_eq!(num("DATE(0,1,1)"), 1.0);
        assert_eq!(num("DATE(1899,12,31)"), num("DATE(3799,12,31)"));
        // The 1900 date system.
        assert_eq!(num("DATE(1900,1,1)"), 1.0);
        assert_eq!(num("DATE(1900,1,0)"), 0.0);
        assert_eq!(num("DATE(1900,2,28)"), 59.0);
        assert_eq!(num("DATE(1900,2,29)"), 60.0);
        assert_eq!(num("DATE(1900,3,0)"), 60.0);
        assert_eq!(num("DATE(1900,3,1)"), 61.0);
        assert_eq!(num("DATE(9999,12,31)"), 2958465.0);
        assert_eq!(err("DATE(9999,12,32)"), ErrorKind::Num);
        assert_eq!(err("DATE(10000,1,1)"), ErrorKind::Num);
        assert_eq!(err("DATE(-1,1,1)"), ErrorKind::Num);
        assert_eq!(err("DATE(1900,1,-1)"), ErrorKind::Num);
        assert_eq!(err("DATE(\"x\",1,1)"), ErrorKind::Value);
    }

    #[test]
    fn datevalue_and_timevalue() {
        assert_eq!(num("DATEVALUE(\"8/22/2011\")"), 40777.0);
        assert_eq!(num("DATEVALUE(\"22-MAY-2011\")"), 40685.0);
        assert_eq!(num("DATEVALUE(\"2011/02/23\")"), 40597.0);
        assert_eq!(num("DATEVALUE(\"2024-03-01\")"), num("DATE(2024,3,1)"));
        assert_eq!(
            num("DATEVALUE(\"2024-03-01 18:00\")"),
            num("DATE(2024,3,1)")
        );
        assert_eq!(num("DATEVALUE(\"12:30\")"), 0.0);
        assert_eq!(err("DATEVALUE(\"hello\")"), ErrorKind::Value);
        assert_eq!(err("DATEVALUE(45000)"), ErrorKind::Value);
        close("TIMEVALUE(\"2:24 AM\")", 0.1);
        close("TIMEVALUE(\"22-Aug-2011 6:35 AM\")", 0.274305555555556);
        close("TIMEVALUE(\"18:00\")", 0.75);
        assert_eq!(err("TIMEVALUE(\"nope\")"), ErrorKind::Value);
        assert_eq!(err("TIMEVALUE(0.5)"), ErrorKind::Value);
    }

    #[test]
    fn time() {
        assert_eq!(num("TIME(12,0,0)"), 0.5);
        close("TIME(16,48,10)", 0.700115740740741);
        close("TIME(0,750,0)", 0.520833333333333);
        // Hours wrap at 24.
        close("TIME(27,0,0)", 0.125);
        close("TIME(1,-30,0)", 30.0 / 1440.0);
        assert_eq!(err("TIME(0,-1,0)"), ErrorKind::Num);
        assert_eq!(err("TIME(32768,0,0)"), ErrorKind::Num);
    }

    #[test]
    fn parts() {
        assert_eq!(num("YEAR(DATE(2024,3,15))"), 2024.0);
        assert_eq!(num("MONTH(DATE(2024,3,15))"), 3.0);
        assert_eq!(num("DAY(DATE(2024,3,15))"), 15.0);
        assert_eq!(num("YEAR(45351.9)"), 2024.0);
        assert_eq!(num("DAY(45351.9)"), 29.0);
        // Date text is accepted wherever a date is expected.
        assert_eq!(num("YEAR(\"2024-03-01\")"), 2024.0);
        assert_eq!(num("MONTH(\"3/15/2024\")"), 3.0);
        assert_eq!(num("DAY(\"March 15, 2024\")"), 15.0);
        assert_eq!(num("YEAR(0)"), 1900.0);
        assert_eq!(num("MONTH(0)"), 1.0);
        assert_eq!(num("DAY(0)"), 0.0);
        assert_eq!(num("DAY(60)"), 29.0);
        assert_eq!(num("MONTH(60)"), 2.0);
        assert_eq!(err("YEAR(-1)"), ErrorKind::Num);
        assert_eq!(err("YEAR(2958466)"), ErrorKind::Num);
        assert_eq!(err("MONTH(\"abc\")"), ErrorKind::Value);
        assert_eq!(num("HOUR(0.75)"), 18.0);
        assert_eq!(num("HOUR(DATE(2024,1,1)+TIME(7,45,30))"), 7.0);
        assert_eq!(num("MINUTE(TIME(7,45,30))"), 45.0);
        assert_eq!(num("SECOND(TIME(7,45,30))"), 30.0);
        assert_eq!(num("MINUTE(\"12:45 PM\")"), 45.0);
        assert_eq!(num("HOUR(\"3:30 PM\")"), 15.0);
        assert_eq!(num("SECOND(\"4:48:18 PM\")"), 18.0);
        assert_eq!(num("HOUR(0.9999999)"), 0.0);
        assert_eq!(err("HOUR(-0.5)"), ErrorKind::Num);
        // Lifted over a range.
        let w = wb(&[
            ("A1", "2021-06-01"),
            ("A2", "2022-07-02"),
            ("C1", "=YEAR(A1:A2)"),
        ]);
        assert_eq!(val(&w, "C1"), Value::Number(2021.0));
        assert_eq!(val(&w, "C2"), Value::Number(2022.0));
    }

    #[test]
    fn locale_formatted_dates() {
        use crate::locale::Locale;
        use crate::workbook::Workbook;
        // French: day/month/year order, `;` separators, localized names.
        let mut w = Workbook::new(Locale::fr());
        w.set_cell_text(0, 0, 0, "=ANNEE(\"01/03/2024\")").unwrap();
        w.set_cell_text(0, 1, 0, "=MOIS(\"01/03/2024\")").unwrap();
        w.set_cell_text(0, 2, 0, "=JOURSEM(\"1 mars 2024\";2)")
            .unwrap();
        w.set_cell_text(0, 3, 0, "=DATEVALUE(\"15/08/2011\")")
            .unwrap();
        w.set_cell_text(0, 4, 0, "=EOMONTH(\"10/02/2024\";0)-DATE(2024;2;1)")
            .unwrap();
        w.set_cell_text(0, 5, 0, "=HOUR(\"01/03/2024 18:30\")")
            .unwrap();
        w.commit("fr");
        assert_eq!(w.value_at(0, 0, 0), Value::Number(2024.0));
        assert_eq!(w.value_at(0, 1, 0), Value::Number(3.0));
        assert_eq!(w.value_at(0, 2, 0), Value::Number(5.0));
        assert_eq!(w.value_at(0, 3, 0), Value::Number(40770.0));
        assert_eq!(w.value_at(0, 4, 0), Value::Number(28.0));
        assert_eq!(w.value_at(0, 5, 0), Value::Number(18.0));
    }

    #[test]
    fn weekday() {
        // 14 February 2008 was a Thursday.
        assert_eq!(num("WEEKDAY(DATE(2008,2,14))"), 5.0);
        assert_eq!(num("WEEKDAY(DATE(2008,2,14),1)"), 5.0);
        assert_eq!(num("WEEKDAY(DATE(2008,2,14),2)"), 4.0);
        assert_eq!(num("WEEKDAY(DATE(2008,2,14),3)"), 3.0);
        assert_eq!(num("WEEKDAY(DATE(2008,2,14),11)"), 4.0);
        assert_eq!(num("WEEKDAY(DATE(2008,2,14),12)"), 3.0);
        assert_eq!(num("WEEKDAY(DATE(2008,2,14),13)"), 2.0);
        assert_eq!(num("WEEKDAY(DATE(2008,2,14),14)"), 1.0);
        assert_eq!(num("WEEKDAY(DATE(2008,2,14),15)"), 7.0);
        assert_eq!(num("WEEKDAY(DATE(2008,2,14),16)"), 6.0);
        assert_eq!(num("WEEKDAY(DATE(2008,2,14),17)"), 5.0);
        // 29 February 2024 was a Thursday, 6 October 2024 a Sunday.
        assert_eq!(num("WEEKDAY(45351)"), 5.0);
        assert_eq!(num("WEEKDAY(DATE(2024,10,6))"), 1.0);
        assert_eq!(num("WEEKDAY(DATE(2024,10,6),2)"), 7.0);
        assert_eq!(num("WEEKDAY(DATE(2024,10,6),3)"), 6.0);
        assert_eq!(num("WEEKDAY(\"2024-10-05\")"), 7.0);
        assert_eq!(num("WEEKDAY(0)"), 7.0);
        assert_eq!(num("WEEKDAY(1)"), 1.0);
        assert_eq!(err("WEEKDAY(45351,4)"), ErrorKind::Num);
        assert_eq!(err("WEEKDAY(45351,18)"), ErrorKind::Num);
        assert_eq!(err("WEEKDAY(-1)"), ErrorKind::Num);
    }

    #[test]
    fn weeknum() {
        assert_eq!(num("WEEKNUM(DATE(2012,3,9))"), 10.0);
        assert_eq!(num("WEEKNUM(DATE(2012,3,9),2)"), 11.0);
        assert_eq!(num("WEEKNUM(DATE(2024,1,1))"), 1.0);
        // 2023 started on a Sunday.
        assert_eq!(num("WEEKNUM(DATE(2023,1,1))"), 1.0);
        assert_eq!(num("WEEKNUM(DATE(2023,1,1),2)"), 1.0);
        assert_eq!(num("WEEKNUM(DATE(2023,1,2),2)"), 2.0);
        assert_eq!(num("WEEKNUM(DATE(2023,1,7),1)"), 1.0);
        assert_eq!(num("WEEKNUM(DATE(2023,1,8),1)"), 2.0);
        assert_eq!(num("WEEKNUM(DATE(2023,1,8),11)"), 2.0);
        assert_eq!(num("WEEKNUM(DATE(2023,1,3),12)"), 2.0);
        assert_eq!(num("WEEKNUM(DATE(2023,1,8),17)"), 2.0);
        assert_eq!(num("WEEKNUM(DATE(2024,12,31))"), 53.0);
        assert_eq!(num("WEEKNUM(DATE(2021,1,1),21)"), 53.0);
        assert_eq!(num("WEEKNUM(DATE(2012,3,9),21)"), 10.0);
        assert_eq!(err("WEEKNUM(DATE(2012,3,9),3)"), ErrorKind::Num);
        assert_eq!(err("WEEKNUM(DATE(2012,3,9),20)"), ErrorKind::Num);
        assert_eq!(num("ISOWEEKNUM(DATE(2012,3,9))"), 10.0);
        assert_eq!(num("ISOWEEKNUM(DATE(2024,1,1))"), 1.0);
        assert_eq!(num("ISOWEEKNUM(DATE(2021,1,1))"), 53.0);
        assert_eq!(num("ISOWEEKNUM(DATE(2024,12,30))"), 1.0);
        assert_eq!(num("ISOWEEKNUM(DATE(2023,1,1))"), 52.0);
        assert_eq!(err("ISOWEEKNUM(-5)"), ErrorKind::Num);
    }

    #[test]
    fn edate_and_eomonth() {
        assert_eq!(num("EDATE(DATE(2011,1,15),1)"), 40589.0);
        assert_eq!(num("EDATE(DATE(2011,1,15),-1)"), 40527.0);
        assert_eq!(num("EDATE(DATE(2011,1,15),2)"), 40617.0);
        // The day is clamped to the target month's length.
        assert_eq!(num("EDATE(DATE(2024,1,31),1)"), num("DATE(2024,2,29)"));
        assert_eq!(num("EDATE(DATE(2023,1,31),1)"), num("DATE(2023,2,28)"));
        assert_eq!(num("EDATE(DATE(2024,2,29),12)"), num("DATE(2025,2,28)"));
        assert_eq!(num("EDATE(\"2024-03-31\",-1.9)"), num("DATE(2024,2,29)"));
        assert_eq!(err("EDATE(DATE(1900,1,15),-1)"), ErrorKind::Num);
        assert_eq!(err("EDATE(-1,1)"), ErrorKind::Num);
        assert_eq!(err("EDATE(\"x\",1)"), ErrorKind::Value);
        assert_eq!(num("EOMONTH(DATE(2011,1,1),1)"), 40602.0);
        assert_eq!(num("EOMONTH(DATE(2011,1,1),-3)"), 40482.0);
        assert_eq!(num("EOMONTH(DATE(2024,1,15),1)"), 45351.0);
        assert_eq!(num("EOMONTH(DATE(2024,2,10),0)"), 45351.0);
        assert_eq!(num("EOMONTH(DATE(2023,12,31),2)"), num("DATE(2024,2,29)"));
        assert_eq!(num("EOMONTH(DATE(2024,12,5),1)"), num("DATE(2025,1,31)"));
        assert_eq!(err("EOMONTH(DATE(9999,12,1),1)"), ErrorKind::Num);
    }

    #[test]
    fn datedif() {
        assert_eq!(num("DATEDIF(DATE(2001,1,1),DATE(2003,1,1),\"Y\")"), 2.0);
        assert_eq!(num("DATEDIF(DATE(2001,6,1),DATE(2002,8,15),\"D\")"), 440.0);
        assert_eq!(num("DATEDIF(DATE(2001,6,1),DATE(2002,8,15),\"YD\")"), 75.0);
        assert_eq!(num("DATEDIF(DATE(2001,6,1),DATE(2002,8,15),\"M\")"), 14.0);
        assert_eq!(num("DATEDIF(DATE(2001,6,1),DATE(2002,8,15),\"YM\")"), 2.0);
        assert_eq!(num("DATEDIF(DATE(2001,6,1),DATE(2002,8,15),\"MD\")"), 14.0);
        assert_eq!(num("DATEDIF(DATE(2001,6,20),DATE(2002,8,15),\"md\")"), 26.0);
        assert_eq!(num("DATEDIF(DATE(2001,6,20),DATE(2002,6,19),\"y\")"), 0.0);
        assert_eq!(num("DATEDIF(DATE(2001,6,20),DATE(2002,6,19),\"m\")"), 11.0);
        assert_eq!(
            num("DATEDIF(DATE(2001,6,20),DATE(2002,6,19),\"yd\")"),
            364.0
        );
        assert_eq!(num("DATEDIF(\"2020-01-31\",\"2020-03-01\",\"M\")"), 1.0);
        assert_eq!(num("DATEDIF(DATE(2024,5,5),DATE(2024,5,5),\"D\")"), 0.0);
        assert_eq!(
            err("DATEDIF(DATE(2024,1,2),DATE(2024,1,1),\"D\")"),
            ErrorKind::Num
        );
        assert_eq!(
            err("DATEDIF(DATE(2024,1,1),DATE(2024,1,2),\"X\")"),
            ErrorKind::Num
        );
        assert_eq!(err("DATEDIF(\"x\",DATE(2024,1,2),\"D\")"), ErrorKind::Value);
    }

    #[test]
    fn days_and_days360() {
        assert_eq!(num("DAYS(DATE(2011,3,15),DATE(2011,2,1))"), 42.0);
        assert_eq!(num("DAYS(\"15-MAR-2021\",\"1-FEB-2021\")"), 42.0);
        assert_eq!(num("DAYS(DATE(2011,2,1),DATE(2011,3,15))"), -42.0);
        assert_eq!(err("DAYS(\"x\",1)"), ErrorKind::Value);
        assert_eq!(err("DAYS(-1,1)"), ErrorKind::Num);
        assert_eq!(num("DAYS360(DATE(2011,1,30),DATE(2011,2,1))"), 1.0);
        assert_eq!(num("DAYS360(DATE(2011,1,1),DATE(2011,12,31))"), 360.0);
        assert_eq!(num("DAYS360(DATE(2011,1,1),DATE(2011,2,1))"), 30.0);
        // US: the 31st after a start before the 30th stays the 31st.
        assert_eq!(num("DAYS360(DATE(2024,1,15),DATE(2024,3,31))"), 76.0);
        assert_eq!(num("DAYS360(DATE(2024,1,15),DATE(2024,3,31),TRUE)"), 75.0);
        assert_eq!(num("DAYS360(DATE(2024,1,31),DATE(2024,3,31))"), 60.0);
        // US: the last day of February counts as the 30th when it starts.
        assert_eq!(num("DAYS360(DATE(2024,2,29),DATE(2024,3,31))"), 30.0);
        assert_eq!(num("DAYS360(DATE(2023,2,28),DATE(2023,3,1))"), 1.0);
        assert_eq!(num("DAYS360(DATE(2023,2,28),DATE(2023,3,1),TRUE)"), 3.0);
        assert_eq!(num("DAYS360(DATE(2024,3,31),DATE(2024,1,15))"), -75.0);
    }

    #[test]
    fn networkdays() {
        assert_eq!(num("NETWORKDAYS(DATE(2012,10,1),DATE(2013,3,1))"), 110.0);
        assert_eq!(
            num("NETWORKDAYS(DATE(2012,10,1),DATE(2013,3,1),DATE(2012,11,22))"),
            109.0
        );
        let w = wb(&[
            ("A1", "2012-11-22"),
            ("A2", "2012-12-04"),
            ("A3", "2013-01-21"),
            ("B1", "=NETWORKDAYS(DATE(2012,10,1),DATE(2013,3,1),A1:A3)"),
            ("B2", "=NETWORKDAYS(\"2013-03-01\",\"2012-10-01\",A1:A5)"),
            ("B3", "=WORKDAY(DATE(2012,10,1),109,A1:A3)"),
        ]);
        assert_eq!(val(&w, "B1"), Value::Number(107.0));
        assert_eq!(val(&w, "B2"), Value::Number(-107.0));
        // 106 working days after the start is 1 March; three more is 6 March.
        assert_eq!(val(&w, "B3"), Value::Number(num("DATE(2013,3,6)")));
        // A week, a weekend, a single day.
        assert_eq!(num("NETWORKDAYS(DATE(2024,10,7),DATE(2024,10,13))"), 5.0);
        assert_eq!(num("NETWORKDAYS(DATE(2024,10,5),DATE(2024,10,6))"), 0.0);
        assert_eq!(num("NETWORKDAYS(DATE(2024,10,7),DATE(2024,10,7))"), 1.0);
        // Holidays on a weekend or listed twice are not subtracted twice.
        assert_eq!(
            num("NETWORKDAYS(DATE(2024,10,7),DATE(2024,10,13),{45572,45572,45577})"),
            4.0
        );
        assert_eq!(err("NETWORKDAYS(\"x\",DATE(2024,10,13))"), ErrorKind::Value);
        assert_eq!(err("NETWORKDAYS(1,10,\"x\")"), ErrorKind::Value);
    }

    #[test]
    fn networkdays_intl() {
        assert_eq!(
            num("NETWORKDAYS.INTL(DATE(2006,1,1),DATE(2006,1,31))"),
            22.0
        );
        assert_eq!(
            num("NETWORKDAYS.INTL(DATE(2006,2,28),DATE(2006,1,31))"),
            -21.0
        );
        assert_eq!(
            num("NETWORKDAYS.INTL(DATE(2006,1,1),DATE(2006,2,1),7,{\"2006/1/2\",\"2006/1/16\"})"),
            22.0
        );
        assert_eq!(
            num(
                "NETWORKDAYS.INTL(DATE(2006,1,1),DATE(2006,2,1),\"0010001\",{\"2006/1/2\",\"2006/1/16\"})"
            ),
            20.0
        );
        // January 2006 has 5 Sundays, 5 Mondays, 5 Tuesdays.
        assert_eq!(
            num("NETWORKDAYS.INTL(DATE(2006,1,1),DATE(2006,1,31),11)"),
            26.0
        );
        assert_eq!(
            num("NETWORKDAYS.INTL(DATE(2006,1,1),DATE(2006,1,31),2)"),
            21.0
        );
        assert_eq!(
            num("NETWORKDAYS.INTL(DATE(2006,1,1),DATE(2006,1,31),17)"),
            27.0
        );
        assert_eq!(
            num("NETWORKDAYS.INTL(DATE(2006,1,1),DATE(2006,1,31),\"1111111\")"),
            0.0
        );
        assert_eq!(
            num("NETWORKDAYS.INTL(DATE(2006,1,1),DATE(2006,1,31),\"0000000\")"),
            31.0
        );
        assert_eq!(
            err("NETWORKDAYS.INTL(DATE(2006,1,1),DATE(2006,1,31),8)"),
            ErrorKind::Num
        );
        assert_eq!(
            err("NETWORKDAYS.INTL(DATE(2006,1,1),DATE(2006,1,31),\"011\")"),
            ErrorKind::Value
        );
        assert_eq!(
            err("NETWORKDAYS.INTL(DATE(2006,1,1),DATE(2006,1,31),\"0000002\")"),
            ErrorKind::Value
        );
    }

    #[test]
    fn workday() {
        assert_eq!(num("WORKDAY(DATE(2008,10,1),151)"), num("DATE(2009,4,30)"));
        assert_eq!(
            num("WORKDAY(DATE(2008,10,1),151,{39778,39786,39834})"),
            num("DATE(2009,5,5)")
        );
        // Friday + 1 = Monday; Monday - 1 = Friday; 0 days = the start.
        assert_eq!(num("WORKDAY(DATE(2024,10,4),1)"), num("DATE(2024,10,7)"));
        assert_eq!(num("WORKDAY(DATE(2024,10,7),-1)"), num("DATE(2024,10,4)"));
        assert_eq!(num("WORKDAY(DATE(2024,10,5),0)"), num("DATE(2024,10,5)"));
        assert_eq!(num("WORKDAY(DATE(2024,10,5),1)"), num("DATE(2024,10,7)"));
        assert_eq!(
            num("WORKDAY(DATE(2024,10,4),1,DATE(2024,10,7))"),
            num("DATE(2024,10,8)")
        );
        assert_eq!(num("WORKDAY(DATE(2024,10,4),5.9)"), num("DATE(2024,10,11)"));
        assert_eq!(num("WORKDAY(DATE(2024,10,11),-10)"), num("DATE(2024,9,27)"));
        // Consistency with NETWORKDAYS over a long span with holidays.
        assert_eq!(
            num(
                "NETWORKDAYS(DATE(2020,1,1),WORKDAY(DATE(2020,1,1),1000,{43850,43851,44200}),{43850,43851,44200})"
            ),
            1001.0
        );
        assert_eq!(err("WORKDAY(DATE(9999,12,1),100)"), ErrorKind::Num);
        assert_eq!(err("WORKDAY(10,-100)"), ErrorKind::Num);
        assert_eq!(err("WORKDAY(\"x\",1)"), ErrorKind::Value);
    }

    #[test]
    fn workday_intl() {
        assert_eq!(num("WORKDAY.INTL(DATE(2012,1,1),90,11)"), 41013.0);
        assert_eq!(num("WORKDAY.INTL(DATE(2012,1,1),30,17)"), 40944.0);
        assert_eq!(err("WORKDAY.INTL(DATE(2012,1,1),30,0)"), ErrorKind::Num);
        assert_eq!(
            err("WORKDAY.INTL(DATE(2012,1,1),30,\"1111111\")"),
            ErrorKind::Value
        );
        // Friday/Saturday weekend: Thursday + 1 = Sunday.
        assert_eq!(
            num("WORKDAY.INTL(DATE(2024,10,3),1,7)"),
            num("DATE(2024,10,6)")
        );
        assert_eq!(
            num("WORKDAY.INTL(DATE(2024,10,3),1)"),
            num("DATE(2024,10,4)")
        );
        assert_eq!(
            num("WORKDAY.INTL(DATE(2024,10,3),1,\"0000111\",DATE(2024,10,7))"),
            num("DATE(2024,10,8)")
        );
        assert_eq!(
            num("WORKDAY.INTL(DATE(2024,10,7),-1,\"0000011\")"),
            num("DATE(2024,10,4)")
        );
    }

    #[test]
    fn yearfrac() {
        // Examples from the Excel documentation: 1 Jan 2012 to 30 Jul 2012.
        close(
            "YEARFRAC(DATE(2012,1,1),DATE(2012,7,30))",
            0.580555555555556,
        );
        close(
            "YEARFRAC(DATE(2012,1,1),DATE(2012,7,30),0)",
            0.580555555555556,
        );
        close(
            "YEARFRAC(DATE(2012,1,1),DATE(2012,7,30),1)",
            0.576502732240437,
        );
        close(
            "YEARFRAC(DATE(2012,1,1),DATE(2012,7,30),2)",
            0.586111111111111,
        );
        close(
            "YEARFRAC(DATE(2012,1,1),DATE(2012,7,30),3)",
            0.578082191780822,
        );
        close(
            "YEARFRAC(DATE(2012,1,1),DATE(2012,7,30),4)",
            0.580555555555556,
        );
        // Reversed dates give the same result.
        close(
            "YEARFRAC(DATE(2012,7,30),DATE(2012,1,1),1)",
            0.576502732240437,
        );
        close("YEARFRAC(DATE(2023,1,1),DATE(2024,1,1),1)", 1.0);
        close("YEARFRAC(DATE(2023,6,1),DATE(2024,6,1),1)", 366.0 / 366.0);
        close("YEARFRAC(DATE(2022,6,1),DATE(2023,6,1),1)", 1.0);
        // More than a year: average length of 2020..2023 is 365.25 days.
        close("YEARFRAC(DATE(2020,1,1),DATE(2023,7,1),1)", 1277.0 / 365.25);
        close("YEARFRAC(DATE(2024,1,31),DATE(2024,3,31),0)", 60.0 / 360.0);
        close("YEARFRAC(DATE(2024,1,15),DATE(2024,3,31),0)", 76.0 / 360.0);
        close("YEARFRAC(DATE(2024,1,15),DATE(2024,3,31),4)", 75.0 / 360.0);
        close("YEARFRAC(DATE(2024,2,29),DATE(2025,2,28),0)", 1.0);
        close("YEARFRAC(\"2024-01-01\",\"2024-07-01\",3)", 182.0 / 365.0);
        assert_eq!(num("YEARFRAC(DATE(2024,1,1),DATE(2024,1,1),1)"), 0.0);
        assert_eq!(
            err("YEARFRAC(DATE(2012,1,1),DATE(2012,7,30),5)"),
            ErrorKind::Num
        );
        assert_eq!(
            err("YEARFRAC(DATE(2012,1,1),DATE(2012,7,30),-1)"),
            ErrorKind::Num
        );
        assert_eq!(err("YEARFRAC(\"x\",DATE(2012,7,30))"), ErrorKind::Value);
    }
}
