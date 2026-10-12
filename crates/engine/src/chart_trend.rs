//! Trendlines, computed the way Excel computes them: least squares, on
//! the data itself for linear and polynomial trends, on its logarithms for
//! exponential, logarithmic and power trends (which is why those need
//! positive values). The equations and R² values were checked against the
//! labels Excel 365 prints on the same data.

use crate::chart::TrendKind;
use crate::format::format_general;
use crate::locale::Locale;

/// A fitted trend (every kind but the moving average).
#[derive(Clone, Debug, PartialEq)]
pub struct Fit {
    pub kind: TrendKind,
    /// Linear and polynomial: y = c₀ + c₁x + c₂x² + ...
    /// Exponential: y = c₀·e^(c₁x). Logarithmic: y = c₀ + c₁·ln x.
    /// Power: y = c₀·x^c₁.
    pub coefficients: Vec<f64>,
    /// NaN when the data has no variation to explain.
    pub r_squared: f64,
}

/// A piece of the text of an equation; raised pieces are exponents.
#[derive(Clone, Debug, PartialEq)]
pub struct EquationPart {
    pub text: String,
    pub raised: bool,
}

/// Least squares solution of `rows · c = targets` by Householder
/// reflections (the normal equations lose too many digits on the powers of
/// a polynomial). `None` when the columns are not independent.
fn least_squares(mut rows: Vec<Vec<f64>>, mut targets: Vec<f64>) -> Option<Vec<f64>> {
    let n = rows.len();
    let k = rows.first()?.len();
    if n < k || k == 0 {
        return None;
    }
    let scale = rows.iter().flatten().fold(0.0f64, |m, v| m.max(v.abs()));
    for col in 0..k {
        let norm = (col..n).map(|r| rows[r][col] * rows[r][col]).sum::<f64>().sqrt();
        if norm <= scale * 1e-13 {
            return None;
        }
        let alpha = if rows[col][col] > 0.0 { -norm } else { norm };
        let mut v: Vec<f64> = (col..n).map(|r| rows[r][col]).collect();
        v[0] -= alpha;
        let vv: f64 = v.iter().map(|x| x * x).sum();
        if vv > 0.0 {
            for c in col..k {
                let dot: f64 = v.iter().enumerate().map(|(i, x)| x * rows[col + i][c]).sum();
                let f = 2.0 * dot / vv;
                for (i, x) in v.iter().enumerate() {
                    rows[col + i][c] -= f * x;
                }
            }
            let dot: f64 = v.iter().enumerate().map(|(i, x)| x * targets[col + i]).sum();
            let f = 2.0 * dot / vv;
            for (i, x) in v.iter().enumerate() {
                targets[col + i] -= f * x;
            }
        }
    }
    let mut out = vec![0.0; k];
    for col in (0..k).rev() {
        let mut sum = targets[col];
        for c in col + 1..k {
            sum -= rows[col][c] * out[c];
        }
        out[col] = sum / rows[col][col];
    }
    out.iter().all(|v| v.is_finite()).then_some(out)
}

/// Fit a trend of the given kind through the points. `intercept` is
/// Excel's "Set intercept" (linear, polynomial and exponential trends).
/// `None` when the kind cannot be fitted to this data: too few points,
/// values that have no logarithm, a moving average.
pub fn fit(kind: TrendKind, points: &[(f64, f64)], intercept: Option<f64>) -> Option<Fit> {
    let positive_x = points.iter().all(|(x, _)| *x > 0.0);
    let positive_y = points.iter().all(|(_, y)| *y > 0.0);
    // The fit is a polynomial of `degree` in u = f(x) for t = g(y).
    let (degree, log_x, log_y) = match kind {
        TrendKind::Linear => (1, false, false),
        TrendKind::Polynomial(order) => (order.clamp(2, 6) as usize, false, false),
        TrendKind::Exponential if positive_y => (1, false, true),
        TrendKind::Logarithmic if positive_x => (1, true, false),
        TrendKind::Power if positive_x && positive_y => (1, true, true),
        _ => return None,
    };
    // Only these accept a forced intercept, as in Excel's dialog.
    let fixed = match kind {
        TrendKind::Linear | TrendKind::Polynomial(_) => intercept,
        TrendKind::Exponential => intercept.filter(|c| *c > 0.0).map(f64::ln),
        _ => None,
    };
    if points.len() < degree + 1 - fixed.is_some() as usize || points.len() < 2 {
        return None;
    }
    let us: Vec<f64> = points.iter().map(|(x, _)| if log_x { x.ln() } else { *x }).collect();
    let ts: Vec<f64> = points.iter().map(|(_, y)| if log_y { y.ln() } else { *y }).collect();
    let first_power = fixed.is_some() as i32;
    let rows: Vec<Vec<f64>> = us
        .iter()
        .map(|u| (first_power..=degree as i32).map(|p| u.powi(p)).collect())
        .collect();
    let targets: Vec<f64> = ts.iter().map(|t| t - fixed.unwrap_or(0.0)).collect();
    let mut c = least_squares(rows, targets.clone())?;
    if let Some(c0) = fixed {
        c.insert(0, c0);
    }
    let predicted = |u: f64| c.iter().rev().fold(0.0, |acc, k| acc * u + k);
    let r_squared = if fixed.is_some() {
        // Through a forced intercept the variation is measured from it, as
        // LINEST does without a constant.
        let sse: f64 = us.iter().zip(&ts).map(|(u, t)| (t - predicted(*u)).powi(2)).sum();
        let sst: f64 = targets.iter().map(|t| t * t).sum();
        if sst > 0.0 { 1.0 - sse / sst } else { f64::NAN }
    } else {
        // What Excel prints: the squared correlation of the values with
        // the trend, in the units of the chart. For linear and polynomial
        // trends that is the share of the variation the trend explains;
        // for exponential and power trends it is not the R² of the line
        // fitted through the logarithms.
        let fitted: Vec<f64> = us.iter().map(|u| if log_y { predicted(*u).exp() } else { predicted(*u) }).collect();
        let ys: Vec<f64> = points.iter().map(|(_, y)| *y).collect();
        let n = ys.len() as f64;
        let (my, mf) = (ys.iter().sum::<f64>() / n, fitted.iter().sum::<f64>() / n);
        let cov: f64 = ys.iter().zip(&fitted).map(|(y, f)| (y - my) * (f - mf)).sum();
        let (vy, vf): (f64, f64) =
            (ys.iter().map(|y| (y - my).powi(2)).sum(), fitted.iter().map(|f| (f - mf).powi(2)).sum());
        if vy > 0.0 && vf > 0.0 { cov * cov / (vy * vf) } else { f64::NAN }
    };
    if log_y {
        c[0] = c[0].exp();
    }
    Some(Fit {
        kind,
        coefficients: c,
        r_squared,
    })
}

/// The moving average of the points, in the order given: one point for
/// every point that has `period - 1` points before it.
pub fn moving_average(points: &[(f64, f64)], period: u32) -> Vec<(f64, f64)> {
    let p = period.max(2) as usize;
    if points.len() < p {
        return Vec::new();
    }
    (p - 1..points.len())
        .map(|i| {
            let sum: f64 = points[i + 1 - p..=i].iter().map(|(_, y)| y).sum();
            (points[i].0, sum / p as f64)
        })
        .collect()
}

/// A number as trendline labels show it: General, in six characters.
fn number(v: f64, locale: &Locale) -> String {
    format_general(v.abs(), locale, 6)
}

fn plain(text: impl Into<String>) -> EquationPart {
    EquationPart {
        text: text.into(),
        raised: false,
    }
}

fn raised(text: impl Into<String>) -> EquationPart {
    EquationPart {
        text: text.into(),
        raised: true,
    }
}

impl Fit {
    /// The value of the trend at `x`; NaN where it has none (the logarithm
    /// of a negative number).
    pub fn value_at(&self, x: f64) -> f64 {
        let c = &self.coefficients;
        match self.kind {
            TrendKind::Exponential => c[0] * (c[1] * x).exp(),
            TrendKind::Logarithmic => c[0] + c[1] * x.ln(),
            TrendKind::Power => c[0] * x.powf(c[1]),
            _ => c.iter().rev().fold(0.0, |acc, k| acc * x + k),
        }
    }

    /// "y = 2.5x + 1": the equation as Excel writes it on the chart.
    pub fn equation(&self, locale: &Locale) -> Vec<EquationPart> {
        let c = &self.coefficients;
        let minus = |v: f64| if v < 0.0 { "-" } else { "" };
        match self.kind {
            TrendKind::Exponential => vec![
                plain(format!("y = {}{}e", minus(c[0]), number(c[0], locale))),
                raised(format!("{}{}x", minus(c[1]), number(c[1], locale))),
            ],
            TrendKind::Power => vec![
                plain(format!("y = {}{}x", minus(c[0]), number(c[0], locale))),
                raised(format!("{}{}", minus(c[1]), number(c[1], locale))),
            ],
            _ => {
                let variable = if self.kind == TrendKind::Logarithmic { "ln(x)" } else { "x" };
                let mut parts = vec![plain("y = ")];
                let mut first = true;
                for power in (0..c.len()).rev() {
                    let k = c[power];
                    if k == 0.0 {
                        continue;
                    }
                    let sign = match (first, k < 0.0) {
                        (true, true) => "-",
                        (true, false) => "",
                        (false, true) => " - ",
                        (false, false) => " + ",
                    };
                    let digits = number(k, locale);
                    // "x", not "1x".
                    let factor = if power > 0 && digits == "1" { "" } else { digits.as_str() };
                    let term = if power > 0 { variable } else { "" };
                    parts.push(plain(format!("{sign}{factor}{term}")));
                    if power > 1 {
                        parts.push(raised(power.to_string()));
                    }
                    first = false;
                }
                if first {
                    parts.push(plain("0"));
                }
                parts
            }
        }
    }

    /// "R² = 0.9846".
    pub fn r_squared_label(&self, locale: &Locale) -> Vec<EquationPart> {
        let value = if self.r_squared.is_finite() {
            let sign = if self.r_squared < 0.0 { "-" } else { "" };
            format!("{sign}{}", number(self.r_squared, locale))
        } else {
            "#N/A".to_string()
        };
        vec![plain(format!("R² = {value}"))]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * (1.0 + b.abs())
    }

    fn text(parts: &[EquationPart]) -> String {
        parts
            .iter()
            .map(|p| if p.raised { format!("^{}", p.text) } else { p.text.clone() })
            .collect()
    }

    const XY: [(f64, f64); 6] = [(1.0, 2.0), (2.0, 3.5), (3.0, 3.0), (4.0, 6.0), (5.0, 7.5), (6.0, 7.0)];

    #[test]
    fn linear_matches_slope_and_intercept() {
        // SLOPE = 1.1428571..., INTERCEPT = 0.8333333..., RSQ = 0.8679927...
        let f = fit(TrendKind::Linear, &XY, None).unwrap();
        assert!(close(f.coefficients[1], 1.1428571428571428));
        assert!(close(f.coefficients[0], 0.8333333333333334));
        assert!(close(f.r_squared, 0.8679927667269439));
        assert_eq!(text(&f.equation(Locale::en())), "y = 1.1429x + 0.8333");
        assert_eq!(text(&f.r_squared_label(Locale::en())), "R² = 0.868");
        assert_eq!(text(&f.equation(Locale::fr())), "y = 1,1429x + 0,8333");

        // Through the origin: Σxy / Σx², R² as LINEST(y, x, FALSE) gives it.
        let f = fit(TrendKind::Linear, &XY, Some(0.0)).unwrap();
        assert!(close(f.coefficients[1], 121.5 / 91.0));
        assert_eq!(f.coefficients[0], 0.0);
        let sse: f64 = XY.iter().map(|(x, y)| (y - 121.5 / 91.0 * x).powi(2)).sum();
        let syy: f64 = XY.iter().map(|(_, y)| y * y).sum();
        assert!(close(f.r_squared, 1.0 - sse / syy));
        assert_eq!(text(&f.equation(Locale::en())), "y = 1.3352x");
        let f = fit(TrendKind::Linear, &XY, Some(2.0)).unwrap();
        assert_eq!(f.coefficients[0], 2.0);
        assert!(close(f.coefficients[1], (121.5 - 2.0 * 21.0) / 91.0));
    }

    #[test]
    fn transformed_fits() {
        // y = 3·e^(0.5x), exactly.
        let pts: Vec<(f64, f64)> = (1..=5).map(|i| (i as f64, 3.0 * (0.5 * i as f64).exp())).collect();
        let f = fit(TrendKind::Exponential, &pts, None).unwrap();
        assert!(close(f.coefficients[0], 3.0) && close(f.coefficients[1], 0.5));
        assert!(close(f.r_squared, 1.0));
        assert_eq!(text(&f.equation(Locale::en())), "y = 3e^0.5x");
        assert!(close(f.value_at(7.0), 3.0 * 3.5f64.exp()));
        // A forced factor only leaves the rate to find.
        let f = fit(TrendKind::Exponential, &pts, Some(3.0)).unwrap();
        assert!(close(f.coefficients[0], 3.0) && close(f.coefficients[1], 0.5));

        // y = 2·ln(x) − 1.
        let pts: Vec<(f64, f64)> = (1..=5).map(|i| (i as f64, 2.0 * (i as f64).ln() - 1.0)).collect();
        let f = fit(TrendKind::Logarithmic, &pts, None).unwrap();
        assert!(close(f.coefficients[1], 2.0) && close(f.coefficients[0], -1.0));
        assert_eq!(text(&f.equation(Locale::en())), "y = 2ln(x) - 1");

        // y = 4·x^1.5.
        let pts: Vec<(f64, f64)> = (1..=5).map(|i| (i as f64, 4.0 * (i as f64).powf(1.5))).collect();
        let f = fit(TrendKind::Power, &pts, None).unwrap();
        assert!(close(f.coefficients[0], 4.0) && close(f.coefficients[1], 1.5));
        assert_eq!(text(&f.equation(Locale::en())), "y = 4x^1.5");

        // The exponential fit of noisy data is the line through ln(y):
        // what LOGEST(y, x) returns.
        let f = fit(TrendKind::Exponential, &XY, None).unwrap();
        let n = XY.len() as f64;
        let (sx, sl): (f64, f64) = XY.iter().fold((0.0, 0.0), |a, (x, y)| (a.0 + x, a.1 + y.ln()));
        let sxx: f64 = XY.iter().map(|(x, _)| x * x).sum();
        let sxl: f64 = XY.iter().map(|(x, y)| x * y.ln()).sum();
        let rate = (n * sxl - sx * sl) / (n * sxx - sx * sx);
        assert!(close(f.coefficients[1], rate));
        assert!(close(f.coefficients[0], ((sl - rate * sx) / n).exp()));

        // The labels Excel prints for these seven points.
        let pts = [(1.0, 2.1), (2.0, 3.9), (3.0, 6.2), (4.0, 7.8), (5.0, 10.5), (6.0, 11.6), (7.0, 14.4)];
        let label = |kind: TrendKind, intercept: Option<f64>| {
            let f = fit(kind, &pts, intercept).unwrap();
            format!("{} / {}", text(&f.equation(Locale::en())), text(&f.r_squared_label(Locale::en())))
        };
        assert_eq!(label(TrendKind::Linear, None), "y = 2.0214x - 0.0143 / R² = 0.9946");
        assert_eq!(label(TrendKind::Exponential, None), "y = 2.0335e^0.303x / R² = 0.9399");
        assert_eq!(label(TrendKind::Logarithmic, None), "y = 6.1205ln(x) + 0.6174 / R² = 0.9163");
        assert_eq!(label(TrendKind::Polynomial(3), None), "y = 0.0083x^3 - 0.0857x^2 + 2.2488x - 0.1429 / R² = 0.9949");
        assert_eq!(label(TrendKind::Power, None), "y = 2.0514x^0.9879 / R² = 0.9945");
        assert_eq!(label(TrendKind::Linear, Some(0.0)), "y = 2.0186x / R² = 0.9989");

        // No logarithm of zero or of a negative number.
        assert!(fit(TrendKind::Exponential, &[(1.0, 1.0), (2.0, -1.0), (3.0, 2.0)], None).is_none());
        assert!(fit(TrendKind::Power, &[(0.0, 1.0), (2.0, 1.0), (3.0, 2.0)], None).is_none());
        assert!(fit(TrendKind::Logarithmic, &[(0.0, 1.0), (2.0, 1.0), (3.0, 2.0)], None).is_none());
    }

    #[test]
    fn polynomials() {
        // y = 2x³ − x² + 0.5x + 7 on eight points, orders 3 to 6 all find it.
        let pts: Vec<(f64, f64)> = (1..=8)
            .map(|i| {
                let x = i as f64;
                (x, 2.0 * x * x * x - x * x + 0.5 * x + 7.0)
            })
            .collect();
        let f = fit(TrendKind::Polynomial(3), &pts, None).unwrap();
        for (got, want) in f.coefficients.iter().zip([7.0, 0.5, -1.0, 2.0]) {
            assert!((got - want).abs() < 1e-8, "{got} {want}");
        }
        assert!(close(f.r_squared, 1.0));
        assert_eq!(text(&f.equation(Locale::en())), "y = 2x^3 - x^2 + 0.5x + 7");
        let f = fit(TrendKind::Polynomial(6), &pts, None).unwrap();
        assert!((f.value_at(9.0) - (2.0 * 729.0 - 81.0 + 4.5 + 7.0)).abs() < 1e-4);
        // Order 2 through noisy data: LINEST(y, x^{1,2}) = {-1/28, 39/28, 1/2}.
        let f = fit(TrendKind::Polynomial(2), &XY, None).unwrap();
        assert!((f.coefficients[2] + 1.0 / 28.0).abs() < 1e-10);
        assert!((f.coefficients[1] - 39.0 / 28.0).abs() < 1e-9);
        assert!((f.coefficients[0] - 0.5).abs() < 1e-9);
        // Large X values (years) do not break it.
        let years: Vec<(f64, f64)> = (0..10).map(|i| (2015.0 + i as f64, 3.0 + 2.0 * i as f64)).collect();
        let f = fit(TrendKind::Polynomial(2), &years, None).unwrap();
        assert!((f.value_at(2030.0) - 33.0).abs() < 1e-3);
        // Too few points for the order.
        assert!(fit(TrendKind::Polynomial(4), &XY[..4], None).is_none());
        assert!(fit(TrendKind::Linear, &XY[..1], None).is_none());
    }

    #[test]
    fn moving_averages() {
        let ma = moving_average(&XY, 2);
        assert_eq!(ma.len(), 5);
        assert_eq!(ma[0], (2.0, 2.75));
        assert_eq!(ma[4], (6.0, 7.25));
        let ma = moving_average(&XY, 3);
        assert_eq!(ma[0], (3.0, 8.5 / 3.0));
        assert!(moving_average(&XY, 7).is_empty());
        assert!(fit(TrendKind::MovingAverage(2), &XY, None).is_none());
    }

    #[test]
    fn labels_are_short() {
        let f = Fit {
            kind: TrendKind::Linear,
            coefficients: vec![-3_456_789_012.0, 2_345_678.0],
            r_squared: 0.5,
        };
        assert_eq!(text(&f.equation(Locale::en())), "y = 2E+06x - 3E+09");
        let f = Fit {
            kind: TrendKind::Linear,
            coefficients: vec![0.0, -1.0],
            r_squared: f64::NAN,
        };
        assert_eq!(text(&f.equation(Locale::en())), "y = -x");
        assert_eq!(text(&f.r_squared_label(Locale::en())), "R² = #N/A");
    }
}
