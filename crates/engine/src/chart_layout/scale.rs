//! The scale of an axis that measures numbers: automatic or manual
//! bounds and units, linear or logarithmic, in either direction.

use crate::chart::{Axis, AxisCrossing, nice_axis};

#[derive(Clone, Debug, PartialEq)]
pub struct Scale {
    pub min: f64,
    pub max: f64,
    /// Distance between major gridlines; on a logarithmic scale, the
    /// number of powers of the base between them.
    pub major: f64,
    pub minor: f64,
    pub log_base: Option<f64>,
    pub reversed: bool,
}

/// A round step that cuts `span` into about five parts.
fn nice_step(span: f64) -> f64 {
    if span <= 0.0 || !span.is_finite() {
        return 1.0;
    }
    let raw = span / 5.0;
    let mag = 10f64.powf(raw.log10().floor());
    let norm = raw / mag;
    mag * if norm <= 1.0 {
        1.0
    } else if norm <= 2.0 {
        2.0
    } else if norm <= 5.0 {
        5.0
    } else {
        10.0
    }
}

impl Scale {
    /// The scale of `axis` for data between `lo` and `hi`. `percent`: the
    /// data are shares of a whole (0 to 1), shown from 0 % to 100 %.
    pub fn new(axis: &Axis, lo: f64, hi: f64, percent: bool) -> Scale {
        let (lo, hi) = if lo.is_finite() && hi.is_finite() && lo <= hi { (lo, hi) } else { (0.0, 1.0) };
        if let Some(base) = axis.log_base.filter(|b| b.is_finite() && *b >= 2.0) {
            return Scale::logarithmic(axis, lo, hi, base);
        }
        let fixed = |v: Option<f64>| v.filter(|v| v.is_finite());
        let (min, max, step) = if percent {
            (if lo < 0.0 { -1.0 } else { 0.0 }, if hi > 0.0 { 1.0 } else { 0.0 }, 0.1)
        } else {
            // The automatic end is chosen for the data and the end that was set.
            let auto = nice_axis(fixed(axis.min).map_or(lo, |m| m.min(hi)), fixed(axis.max).map_or(hi, |m| m.max(lo)));
            (auto.min, auto.max, auto.step)
        };
        let mut min = fixed(axis.min).unwrap_or(min);
        let mut max = fixed(axis.max).unwrap_or(max);
        if max <= min {
            // Bounds that leave nothing to show: keep the one that was set.
            match (fixed(axis.min), fixed(axis.max)) {
                (Some(_), None) => max = min + step.max(f64::MIN_POSITIVE),
                (None, Some(_)) => min = max - step.max(f64::MIN_POSITIVE),
                _ => max = min + 1.0,
            }
        }
        let auto_step = if axis.min.is_none() && axis.max.is_none() { step } else { nice_step(max - min) };
        let major = fixed(axis.major_unit).filter(|u| *u > 0.0).unwrap_or(auto_step);
        // Never more gridlines than anyone could count.
        let major = if (max - min) / major > 200.0 { (max - min) / 200.0 } else { major };
        let minor = fixed(axis.minor_unit).filter(|u| *u > 0.0).unwrap_or(major / 5.0);
        Scale {
            min,
            max,
            major,
            minor: if (max - min) / minor > 1000.0 { major } else { minor },
            log_base: None,
            reversed: axis.reversed,
        }
    }

    fn logarithmic(axis: &Axis, lo: f64, hi: f64, base: f64) -> Scale {
        let positive = |v: Option<f64>| v.filter(|v| v.is_finite() && *v > 0.0);
        // Only what is above zero can be shown.
        let hi = if hi > 0.0 { hi } else { base };
        let lo = if lo > 0.0 { lo.min(hi) } else { hi.min(1.0) };
        let min = positive(axis.min).unwrap_or_else(|| base.powf((lo.ln() / base.ln() + 1e-9).floor()));
        let mut max = positive(axis.max).unwrap_or_else(|| base.powf((hi.ln() / base.ln() - 1e-9).ceil()));
        if max <= min {
            max = min * base;
        }
        Scale {
            min,
            max,
            major: 1.0,
            minor: 1.0,
            log_base: Some(base),
            reversed: axis.reversed,
        }
    }

    /// Position of a value along the scale without regard for its
    /// direction: 0 at `min`, 1 at `max`. Values outside fall outside.
    fn raw_fraction(&self, v: f64) -> f64 {
        match self.log_base {
            Some(_) => {
                if v <= 0.0 {
                    return 0.0;
                }
                (v.ln() - self.min.ln()) / (self.max.ln() - self.min.ln())
            }
            None if self.max == self.min => 0.0,
            None => (v - self.min) / (self.max - self.min),
        }
    }

    /// Position of a value along the axis as drawn: 0 at the start of the
    /// axis (bottom or left), 1 at its end.
    pub fn fraction(&self, v: f64) -> f64 {
        let f = self.raw_fraction(v);
        if self.reversed { 1.0 - f } else { f }
    }

    /// `fraction`, kept on the axis.
    pub fn clamped(&self, v: f64) -> f64 {
        self.fraction(v).clamp(0.0, 1.0)
    }

    pub fn contains(&self, v: f64) -> bool {
        let slack = (self.max - self.min).abs() * 1e-9;
        v >= self.min - slack && v <= self.max + slack
    }

    /// The values that get a gridline and a label.
    pub fn major_ticks(&self) -> Vec<f64> {
        let mut out = Vec::new();
        if let Some(base) = self.log_base {
            let mut v = self.min;
            while v <= self.max * (1.0 + 1e-9) && out.len() < 400 {
                out.push(v);
                v *= base;
            }
            return out;
        }
        let n = ((self.max - self.min) / self.major + 1e-9).floor() as i64;
        for i in 0..=n.clamp(0, 400) {
            let v = self.min + self.major * i as f64;
            // Remove float noise (0.30000000000000004).
            let rounded = (v / self.major).round() * self.major;
            out.push(if (rounded - v).abs() < self.major * 1e-9 { rounded } else { v });
        }
        out
    }

    /// The values of the minor gridlines that are not major ones.
    pub fn minor_ticks(&self) -> Vec<f64> {
        let mut out = Vec::new();
        if let Some(base) = self.log_base {
            let mut decade = self.min;
            while decade < self.max && out.len() < 2000 {
                let mut k = 2.0;
                while k < base && out.len() < 2000 {
                    let v = decade * k;
                    if v < self.max {
                        out.push(v);
                    }
                    k += 1.0;
                }
                decade *= base;
            }
            return out;
        }
        let n = ((self.max - self.min) / self.minor + 1e-9).floor() as i64;
        for i in 0..=n.clamp(0, 2000) {
            let v = self.min + self.minor * i as f64;
            let on_major = ((v - self.min) / self.major).round() * self.major + self.min;
            if (on_major - v).abs() > self.minor * 1e-6 {
                out.push(v);
            }
        }
        out
    }

    /// The value at which the other axis crosses this one.
    pub fn crossing(&self, crossing: AxisCrossing) -> f64 {
        match crossing {
            AxisCrossing::Min => self.min,
            AxisCrossing::Max => self.max,
            AxisCrossing::At(v) if v.is_finite() => v.clamp(self.min, self.max),
            // At zero when zero is on the scale, else at the end nearest to it.
            _ if self.log_base.is_some() => self.min,
            _ => 0.0f64.clamp(self.min, self.max),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_and_manual_bounds() {
        let auto = Scale::new(&Axis::new(), 5.0, 45.0, false);
        assert_eq!((auto.min, auto.max, auto.major), (0.0, 50.0, 10.0));
        assert_eq!(auto.major_ticks(), vec![0.0, 10.0, 20.0, 30.0, 40.0, 50.0]);
        assert_eq!(auto.minor, 2.0);
        assert_eq!(auto.minor_ticks().len(), 20);
        assert_eq!(auto.fraction(25.0), 0.5);

        let axis = Axis {
            min: Some(10.0),
            max: Some(40.0),
            major_unit: Some(7.5),
            reversed: true,
            ..Axis::new()
        };
        let s = Scale::new(&axis, 5.0, 45.0, false);
        assert_eq!(s.major_ticks(), vec![10.0, 17.5, 25.0, 32.5, 40.0]);
        assert_eq!(s.fraction(10.0), 1.0);
        assert_eq!(s.fraction(40.0), 0.0);
        assert_eq!(s.clamped(100.0), 0.0);
        assert_eq!(s.crossing(AxisCrossing::Auto), 10.0);
        assert_eq!(s.crossing(AxisCrossing::Max), 40.0);
        assert_eq!(s.crossing(AxisCrossing::At(25.0)), 25.0);

        // One end set: the other stays automatic.
        let s = Scale::new(&Axis { min: Some(20.0), ..Axis::new() }, 5.0, 45.0, false);
        assert_eq!((s.min, s.max), (20.0, 50.0));
        // Bounds the wrong way round still make a scale.
        let s = Scale::new(&Axis { min: Some(60.0), ..Axis::new() }, 5.0, 45.0, false);
        assert!(s.max > s.min);
        let s = Scale::new(&Axis { min: Some(3.0), max: Some(3.0), ..Axis::new() }, 0.0, 1.0, false);
        assert!(s.max > s.min);
    }

    #[test]
    fn percent_and_logarithmic() {
        let s = Scale::new(&Axis::new(), 0.0, 1.0, true);
        assert_eq!((s.min, s.max), (0.0, 1.0));
        assert_eq!(s.major_ticks().len(), 11);
        let s = Scale::new(&Axis::new(), -0.4, 0.6, true);
        assert_eq!((s.min, s.max), (-1.0, 1.0));

        let log = Axis { log_base: Some(10.0), ..Axis::new() };
        let s = Scale::new(&log, 3.0, 4200.0, false);
        assert_eq!((s.min, s.max), (1.0, 10000.0));
        assert_eq!(s.major_ticks(), vec![1.0, 10.0, 100.0, 1000.0, 10000.0]);
        assert!((s.fraction(100.0) - 0.5).abs() < 1e-12);
        assert_eq!(s.minor_ticks().len(), 4 * 8);
        assert_eq!(s.fraction(-5.0), 0.0);
        assert_eq!(s.crossing(AxisCrossing::Auto), 1.0);
        // Exact powers stay where they are.
        let s = Scale::new(&log, 10.0, 1000.0, false);
        assert_eq!((s.min, s.max), (10.0, 1000.0));
        // Nothing positive to show.
        let s = Scale::new(&log, -5.0, 0.0, false);
        assert!(s.min > 0.0 && s.max > s.min);
        let s = Scale::new(&Axis { log_base: Some(2.0), ..Axis::new() }, 1.0, 30.0, false);
        assert_eq!(s.major_ticks(), vec![1.0, 2.0, 4.0, 8.0, 16.0, 32.0]);
    }
}
