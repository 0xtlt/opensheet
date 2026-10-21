//! Serial date arithmetic (1900 date system, including the fictitious
//! 29 February 1900 that spreadsheets inherited from Lotus 1-2-3).

/// Days from civil date, proleptic Gregorian, relative to 1970-01-01.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Days between 1899-12-30 and 1970-01-01.
const EPOCH_OFFSET: i64 = 25569;

pub fn is_leap_year(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub fn days_in_month(y: i64, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if is_leap_year(y) {
                29
            } else {
                28
            }
        }
    }
}

/// Serial number for a calendar date. Month and day may be out of range and
/// roll over like the `DATE` function (month 13 = January next year, day 0 =
/// last day of previous month). The day offset is applied in serial space,
/// so the fictitious 1900-02-29 (serial 60) is reachable by rollover exactly
/// as in Excel. Returns `None` outside 1900-01-00..9999-12-31.
pub fn date_to_serial(year: i64, month: i64, day: i64) -> Option<f64> {
    // Normalise the month.
    let months = year * 12 + (month - 1);
    let y = months.div_euclid(12);
    let m = months.rem_euclid(12) + 1;
    // Serial of the first day of that month.
    let mut first = days_from_civil(y, m, 1) + EPOCH_OFFSET;
    // Real dates before 1900-03-01 are one lower than the naive count
    // because serial 60 is the non-existent 1900-02-29.
    if first < 61 {
        first -= 1;
    }
    let serial = first + (day - 1);
    if !(0..=2_958_465).contains(&serial) {
        return None;
    }
    Some(serial as f64)
}

/// Calendar date (year, month, day) for a serial number. Serial 60 maps to
/// 1900-02-29 and serial 0 to 1900-01-00, as in Excel.
pub fn serial_to_ymd(serial: f64) -> (i64, u32, u32) {
    let s = serial.floor() as i64;
    if s == 60 {
        return (1900, 2, 29);
    }
    if s <= 0 {
        return (1900, 1, 0);
    }
    let days = if s < 60 { s + 1 } else { s } - EPOCH_OFFSET;
    civil_from_days(days)
}

/// Time of day (hour, minute, second) of a serial number, rounded to the
/// nearest second.
pub fn serial_to_hms(serial: f64) -> (u32, u32, u32) {
    let frac = serial - serial.floor();
    // A time that rounds up to 24:00:00 wraps to midnight.
    let secs = ((frac * 86400.0).round() as i64).rem_euclid(86400);
    (
        (secs / 3600) as u32,
        ((secs % 3600) / 60) as u32,
        (secs % 60) as u32,
    )
}

/// Day of week for a serial number: 0 = Sunday .. 6 = Saturday.
pub fn weekday(serial: f64) -> u32 {
    let s = serial.floor() as i64;
    // Serial 1 (1900-01-01) is treated as a Sunday (Excel's 1900 bug shifts
    // the real weekday for dates before March 1900).
    ((s - 1).rem_euclid(7)) as u32
}

pub fn time_to_serial(h: f64, m: f64, s: f64) -> f64 {
    (h * 3600.0 + m * 60.0 + s) / 86400.0
}

/// Current local date and time as a serial number.
pub fn now_serial() -> f64 {
    use chrono::{Datelike, Timelike};
    let now = chrono::Local::now();
    let date =
        date_to_serial(now.year() as i64, now.month() as i64, now.day() as i64).unwrap_or(0.0);
    date + time_to_serial(now.hour() as f64, now.minute() as f64, now.second() as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serials() {
        assert_eq!(date_to_serial(1900, 1, 1), Some(1.0));
        assert_eq!(date_to_serial(1900, 2, 28), Some(59.0));
        assert_eq!(date_to_serial(1900, 3, 1), Some(61.0));
        assert_eq!(date_to_serial(2000, 1, 1), Some(36526.0));
        assert_eq!(date_to_serial(2024, 2, 29), Some(45351.0));
        assert_eq!(date_to_serial(2025, 13, 1), date_to_serial(2026, 1, 1));
        assert_eq!(date_to_serial(2025, 3, 0), date_to_serial(2025, 2, 28));
        assert_eq!(date_to_serial(1900, 2, 29), Some(60.0));
        assert_eq!(date_to_serial(1900, 3, 0), Some(60.0));
        assert_eq!(date_to_serial(1900, 1, 0), Some(0.0));
        assert_eq!(date_to_serial(1899, 12, 31), Some(0.0));
        assert_eq!(date_to_serial(1899, 12, 30), None);
        assert_eq!(serial_to_ymd(1.0), (1900, 1, 1));
        assert_eq!(serial_to_ymd(59.0), (1900, 2, 28));
        assert_eq!(serial_to_ymd(60.0), (1900, 2, 29));
        assert_eq!(serial_to_ymd(61.0), (1900, 3, 1));
        assert_eq!(serial_to_ymd(45351.0), (2024, 2, 29));
        assert_eq!(serial_to_hms(0.5), (12, 0, 0));
        // 2024-02-29 was a Thursday.
        assert_eq!(weekday(45351.0), 4);
    }
}
