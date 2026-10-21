//! What the Format pane reads from and writes to a chart: for every
//! element, the fill and the line it is drawn with, and for every field of
//! the pane, the text it shows and what typing into it changes. No
//! interface here, so that all of it can be tested.

use crate::i18n::{Lang, lang};
use crate::widgets::number_text;
use opensheet_engine::chart::{
    AxisCrossing, Chart, ChartTitle, DataLabels, ErrorAmount, Family, Fill, Line, Marker,
    MarkerShape, TextFormat, TrendKind, View3D,
};
use opensheet_engine::chart_layout::{ChartElement, Scale};
use opensheet_engine::style::Color;
use opensheet_engine::{Locale, format};

/// A field of the pane in which something is typed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    /// Width of the line of the selected element, in points.
    LineWidth,
    MarkerSize,
    Overlap,
    GapWidth,
    FirstSlice,
    Explosion,
    HoleSize,
    BubbleScale,
    AxisMin,
    AxisMax,
    AxisMajor,
    AxisMinor,
    /// "Axis value" / "At category number" of the crossing.
    AxisCross,
    LogBase,
    /// Format code of the labels of an axis or of data labels.
    NumberFormat,
    TrendOrder,
    TrendPeriod,
    TrendName,
    Forward,
    Backward,
    Intercept,
    ErrorFixed,
    ErrorPercent,
    ErrorStdDev,
    /// "X Rotation": around the vertical axis.
    RotationX,
    /// "Y Rotation": the elevation.
    RotationY,
    Perspective,
    Depth,
    Height,
    Width,
    /// The text of a title.
    Text,
    /// Font and size, in points, of the text of the selected element.
    FontName,
    FontSize,
}

impl Field {
    /// Range and step of the spinner of a numeric field.
    pub fn spin(self) -> Option<(f64, f64, f64)> {
        Some(match self {
            Field::LineWidth => (0.0, 100.0, 0.25),
            Field::MarkerSize => (2.0, 72.0, 1.0),
            Field::Overlap => (-100.0, 100.0, 1.0),
            Field::GapWidth => (0.0, 500.0, 1.0),
            Field::FirstSlice => (0.0, 360.0, 1.0),
            Field::Explosion => (0.0, 400.0, 1.0),
            Field::HoleSize => (10.0, 90.0, 1.0),
            Field::BubbleScale => (0.0, 300.0, 1.0),
            Field::LogBase => (2.0, 1000.0, 1.0),
            Field::TrendOrder => (2.0, 6.0, 1.0),
            Field::TrendPeriod => (2.0, 255.0, 1.0),
            Field::Forward | Field::Backward => (0.0, 1.0e6, 0.5),
            Field::ErrorFixed | Field::ErrorStdDev => (0.0, 1.0e9, 0.1),
            Field::ErrorPercent => (0.0, 1000.0, 1.0),
            Field::RotationX => (0.0, 360.0, 10.0),
            Field::RotationY => (-90.0, 90.0, 10.0),
            Field::Perspective => (0.1, 120.0, 5.0),
            Field::Depth => (20.0, 2000.0, 10.0),
            Field::Height | Field::Width => (0.5, 200.0, 0.1),
            Field::FontSize => (1.0, 409.0, 1.0),
            Field::AxisMin
            | Field::AxisMax
            | Field::AxisMajor
            | Field::AxisMinor
            | Field::AxisCross
            | Field::Intercept
            | Field::NumberFormat
            | Field::TrendName
            | Field::FontName
            | Field::Text => return None,
        })
    }

    /// What follows the number in the field.
    fn unit(self) -> &'static str {
        match self {
            Field::LineWidth | Field::FontSize => " pt",
            Field::Overlap
            | Field::GapWidth
            | Field::Explosion
            | Field::HoleSize
            | Field::ErrorPercent
            | Field::Depth => " %",
            Field::FirstSlice | Field::RotationX | Field::RotationY | Field::Perspective => "°",
            Field::Height | Field::Width => length_unit().1,
            _ => "",
        }
    }
}

/// Pixels per unit of length and the unit as written: centimetres, inches
/// where the interface is in English.
fn length_unit() -> (f64, &'static str) {
    match lang() {
        Lang::En => (96.0, "\""),
        _ => (96.0 / 2.54, " cm"),
    }
}

/// The number typed in a field, whatever unit follows it.
pub fn parse_number(text: &str, decimal_sep: char) -> Option<f64> {
    let cleaned: String = text
        .trim()
        .trim_end_matches(|c: char| !c.is_ascii_digit())
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '\u{a0}' && *c != '\u{202f}')
        .map(|c| if c == decimal_sep { '.' } else { c })
        .collect();
    cleaned.parse::<f64>().ok().filter(|v| v.is_finite())
}

// --------------------------------------------------------------- fill and line

/// The format of the text of an element. The chart area stands for
/// every text of the chart: it shows what they all agree on.
pub fn text_of(chart: &Chart, element: ChartElement) -> Option<TextFormat> {
    if element != ChartElement::ChartArea {
        return element.text(chart).cloned();
    }
    let mut all = chart.clone();
    let all = all.text_formats_mut();
    let first = all.first()?;
    let shared = |same: &dyn Fn(&TextFormat) -> bool| all.iter().all(|text| same(text));
    Some(TextFormat {
        font: first
            .font
            .clone()
            .filter(|_| shared(&|text| text.font == first.font)),
        size: first
            .size
            .filter(|_| shared(&|text| text.size == first.size)),
        bold: first
            .bold
            .filter(|_| shared(&|text| text.bold == first.bold)),
        italic: first
            .italic
            .filter(|_| shared(&|text| text.italic == first.italic)),
        underline: first
            .underline
            .filter(|_| shared(&|text| text.underline == first.underline)),
        color: first
            .color
            .filter(|_| shared(&|text| text.color == first.color)),
    })
}

/// The size, in points, of the text of an element; of a chart area whose
/// texts differ, the size Excel gives its labels.
pub fn text_size(chart: &Chart, element: ChartElement) -> Option<f32> {
    let text = text_of(chart, element)?;
    Some(text.size.unwrap_or(element.text_points()))
}

/// The line an element is drawn with, when the model has one for it: the
/// border of the chart and plot areas, the line or the outline of a
/// series, a trendline, an axis, its gridlines.
pub fn line_of(chart: &Chart, element: ChartElement) -> Option<&Line> {
    match element {
        ChartElement::Axis(id) => Some(&id.of(&chart.axes).line),
        ChartElement::MajorGridlines(id) => Some(&id.of(&chart.axes).major_gridline),
        ChartElement::MinorGridlines(id) => Some(&id.of(&chart.axes).minor_gridline),
        ChartElement::ChartArea => Some(&chart.chart_area.border),
        ChartElement::PlotArea => Some(&chart.plot_area.border),
        ChartElement::Series(s) => chart.series.get(s as usize).map(|x| &x.line),
        ChartElement::Trendline(s, i) => chart
            .series
            .get(s as usize)?
            .trendlines
            .get(i as usize)
            .map(|t| &t.line),
        _ => None,
    }
}

pub fn line_of_mut(chart: &mut Chart, element: ChartElement) -> Option<&mut Line> {
    match element {
        ChartElement::Axis(id) => Some(&mut id.of_mut(&mut chart.axes).line),
        ChartElement::MajorGridlines(id) => Some(&mut id.of_mut(&mut chart.axes).major_gridline),
        ChartElement::MinorGridlines(id) => Some(&mut id.of_mut(&mut chart.axes).minor_gridline),
        ChartElement::ChartArea => Some(&mut chart.chart_area.border),
        ChartElement::PlotArea => Some(&mut chart.plot_area.border),
        ChartElement::Series(s) => chart.series.get_mut(s as usize).map(|x| &mut x.line),
        ChartElement::Trendline(s, i) => chart
            .series
            .get_mut(s as usize)?
            .trendlines
            .get_mut(i as usize)
            .map(|t| &mut t.line),
        _ => None,
    }
}

/// Whether a series is drawn as a line (its `Line` is that line) rather
/// than as a shape (its `Line` is the outline of the shape).
pub fn is_lined(chart: &Chart, series: usize) -> bool {
    chart.series.get(series).is_some_and(|s| {
        matches!(
            chart.series_kind(s).family(),
            Family::Line | Family::Scatter | Family::Radar
        )
    })
}

/// The automatic width of the line of an element, in points.
pub fn auto_line_width(chart: &Chart, element: ChartElement) -> f32 {
    match element {
        ChartElement::Series(s) if is_lined(chart, s as usize) => chart
            .series_kind(&chart.series[s as usize])
            .default_line_width(),
        ChartElement::Series(_) if chart.kind.is_round() => 1.5,
        ChartElement::Trendline(..) => 1.5,
        _ => 0.75,
    }
}

/// The fill of an element, when the model has one for it. A series or a
/// point is filled with its colour: automatic, or a colour of its own.
pub fn fill_of(chart: &Chart, element: ChartElement) -> Option<Fill> {
    let solid = |color: Option<Color>| color.map_or(Fill::Auto, Fill::Solid);
    match element {
        ChartElement::ChartArea => Some(chart.chart_area.fill),
        ChartElement::PlotArea => Some(chart.plot_area.fill),
        ChartElement::Series(s) => chart.series.get(s as usize).map(|x| solid(x.color)),
        ChartElement::Point(s, k) => chart
            .series
            .get(s as usize)
            .map(|x| solid(x.point(k as usize).and_then(|p| p.color))),
        _ => None,
    }
}

/// Fill an element. A series cannot be left unfilled: "no fill" gives it
/// its automatic colour back.
pub fn set_fill(chart: &mut Chart, element: ChartElement, fill: Fill) {
    let color = match fill {
        Fill::Solid(color) => Some(color),
        _ => None,
    };
    match element {
        ChartElement::ChartArea => chart.chart_area.fill = fill,
        ChartElement::PlotArea => chart.plot_area.fill = fill,
        ChartElement::Series(s) => {
            if let Some(series) = chart.series.get_mut(s as usize) {
                series.color = color;
            }
        }
        ChartElement::Point(s, k) => {
            if let Some(series) = chart.series.get_mut(s as usize) {
                series.point_mut(k as usize).color = color;
                series
                    .points
                    .retain(|p| p.color.is_some() || p.explosion.is_some());
            }
        }
        _ => {}
    }
}

/// The marker of a series as it is drawn, to change one of its traits.
pub fn marker_mut(chart: &mut Chart, series: usize) -> Option<&mut Marker> {
    let kind = chart.series_kind(chart.series.get(series)?);
    let series = chart.series.get_mut(series)?;
    let current = series.marker_for(kind);
    Some(series.marker.get_or_insert(current))
}

pub const MARKER_SHAPES: [MarkerShape; 9] = [
    MarkerShape::Square,
    MarkerShape::Diamond,
    MarkerShape::Triangle,
    MarkerShape::X,
    MarkerShape::Star,
    MarkerShape::Dot,
    MarkerShape::Dash,
    MarkerShape::Circle,
    MarkerShape::Plus,
];

/// The labels of a series as they are shown, to change one of their
/// options on that series alone.
pub fn labels_mut(chart: &mut Chart, element: ChartElement) -> Option<&mut DataLabels> {
    let ChartElement::DataLabels(s) = element else {
        return None;
    };
    crate::features::charts::elements::own_labels(chart, s as usize)
}

fn view(chart: &Chart) -> View3D {
    chart.view3d_or_default()
}

// ---------------------------------------------------------------------- fields

/// What a field shows for an element of a chart. `scale`: the scale the
/// selected axis was drawn with, for the bounds left automatic.
pub fn value(
    chart: &Chart,
    element: ChartElement,
    field: Field,
    scale: Option<&Scale>,
    locale: &Locale,
) -> String {
    let sep = lang().locale().decimal_sep;
    let number = |v: f64| format!("{}{}", number_text(v, sep), field.unit());
    let series = element.series().and_then(|s| chart.series.get(s));
    let axis = match element {
        ChartElement::Axis(id) => Some(id.of(&chart.axes)),
        _ => None,
    };
    let trend = match element {
        ChartElement::Trendline(s, i) => chart
            .series
            .get(s as usize)
            .and_then(|x| x.trendlines.get(i as usize)),
        _ => None,
    };
    let bars = match element {
        ChartElement::ErrorBars(s, i) => chart
            .series
            .get(s as usize)
            .and_then(|x| x.error_bars.get(i as usize)),
        _ => None,
    };
    match field {
        Field::LineWidth => {
            let width = line_of(chart, element)
                .and_then(|l| l.width)
                .unwrap_or(auto_line_width(chart, element));
            number(width as f64)
        }
        Field::MarkerSize => series
            .map(|s| number(s.marker_for(chart.series_kind(s)).size as f64))
            .unwrap_or_default(),
        Field::Overlap => {
            number(chart.overlap_for(series.map_or(chart.kind, |s| chart.series_kind(s))) as f64)
        }
        Field::GapWidth => {
            let kind = series.map_or(chart.kind, |s| chart.series_kind(s));
            number(chart.gap_width.unwrap_or(kind.default_gap_width()) as f64)
        }
        Field::FirstSlice => number(chart.first_slice_angle as f64),
        Field::Explosion => {
            let whole = series.map_or(0, |s| s.explosion);
            let own = match element {
                ChartElement::Point(_, k) => series
                    .and_then(|s| s.point(k as usize))
                    .and_then(|p| p.explosion),
                _ => None,
            };
            number(own.unwrap_or(whole) as f64)
        }
        Field::HoleSize => number(chart.hole_size as f64),
        Field::BubbleScale => number(chart.bubble_scale as f64),
        Field::AxisMin => axis
            .and_then(|a| a.min)
            .or(scale.map(|s| s.min))
            .map(number)
            .unwrap_or_default(),
        Field::AxisMax => axis
            .and_then(|a| a.max)
            .or(scale.map(|s| s.max))
            .map(number)
            .unwrap_or_default(),
        Field::AxisMajor => axis
            .and_then(|a| a.major_unit)
            .or(scale.map(|s| s.major))
            .map(number)
            .unwrap_or_default(),
        Field::AxisMinor => axis
            .and_then(|a| a.minor_unit)
            .or(scale.map(|s| s.minor))
            .map(number)
            .unwrap_or_default(),
        Field::AxisCross => match axis.map(|a| a.crossing) {
            Some(AxisCrossing::At(v)) => number(v),
            _ => String::new(),
        },
        Field::LogBase => number(axis.and_then(|a| a.log_base).unwrap_or(10.0)),
        Field::NumberFormat => {
            let code = match element {
                ChartElement::Axis(_) => axis.and_then(|a| a.number_format.clone()),
                ChartElement::DataLabels(_) => series
                    .and_then(|s| chart.labels_for(s))
                    .and_then(|l| l.number_format.clone()),
                _ => None,
            };
            format::code_to_locale(code.as_deref().unwrap_or("General"), locale)
        }
        Field::TrendOrder => match trend.map(|t| t.kind) {
            Some(TrendKind::Polynomial(order)) => number(order as f64),
            _ => number(2.0),
        },
        Field::TrendPeriod => match trend.map(|t| t.kind) {
            Some(TrendKind::MovingAverage(period)) => number(period as f64),
            _ => number(2.0),
        },
        Field::TrendName => trend.and_then(|t| t.name.clone()).unwrap_or_default(),
        Field::Forward => number(trend.map_or(0.0, |t| t.forward)),
        Field::Backward => number(trend.map_or(0.0, |t| t.backward)),
        Field::Intercept => number(trend.and_then(|t| t.intercept).unwrap_or(0.0)),
        Field::ErrorFixed => match bars.map(|b| b.amount) {
            Some(ErrorAmount::Fixed(v)) => number(v),
            _ => number(1.0),
        },
        Field::ErrorPercent => match bars.map(|b| b.amount) {
            Some(ErrorAmount::Percent(v)) => number(v),
            _ => number(5.0),
        },
        Field::ErrorStdDev => match bars.map(|b| b.amount) {
            Some(ErrorAmount::StdDev(v)) => number(v),
            _ => number(1.0),
        },
        Field::RotationX => number(view(chart).rot_y as f64),
        Field::RotationY => number(view(chart).rot_x as f64),
        // The model keeps the field of view: twice Excel's "Perspective".
        Field::Perspective => number(view(chart).perspective as f64 / 2.0),
        Field::Depth => number(view(chart).depth_percent as f64),
        Field::Height => number((chart.size.1 as f64 / length_unit().0 * 100.0).round() / 100.0),
        Field::Width => number((chart.size.0 as f64 / length_unit().0 * 100.0).round() / 100.0),
        Field::Text => match element {
            ChartElement::Title => match &chart.title {
                ChartTitle::Text(text) => text.clone(),
                _ => String::new(),
            },
            ChartElement::AxisTitle(id) => id.of(&chart.axes).title.clone().unwrap_or_default(),
            _ => String::new(),
        },
        // Empty: the font of the chart.
        Field::FontName => text_of(chart, element)
            .and_then(|text| text.font)
            .map(|font| font.to_string())
            .unwrap_or_default(),
        Field::FontSize => text_size(chart, element)
            .map(|size| number(size as f64))
            .unwrap_or_default(),
    }
}

/// Store what was typed in a field. Returns false when it cannot be read
/// or does not apply to the element; the chart is then left as it was.
pub fn set(
    chart: &mut Chart,
    element: ChartElement,
    field: Field,
    text: &str,
    locale: &Locale,
) -> bool {
    let typed = parse_number(text, lang().locale().decimal_sep);
    // A number in the range of the field's spinner.
    let ranged = typed.map(|v| match field.spin() {
        Some((min, max, _)) => v.clamp(min, max),
        None => v,
    });
    let series = element.series().filter(|s| *s < chart.series.len());
    match field {
        Field::LineWidth => {
            let (Some(width), Some(line)) = (ranged, line_of_mut(chart, element)) else {
                return false;
            };
            line.width = Some(width as f32);
            // A width makes sense on a line that is drawn.
            if line.shown == Some(false) {
                line.shown = Some(true);
            }
        }
        Field::MarkerSize => {
            let (Some(size), Some(marker)) = (ranged, series.and_then(|s| marker_mut(chart, s)))
            else {
                return false;
            };
            marker.size = size.round() as u8;
        }
        Field::Overlap => match ranged {
            Some(v) => chart.overlap = Some(v.round() as i32),
            None => return false,
        },
        Field::GapWidth => match ranged {
            Some(v) => chart.gap_width = Some(v.round() as u32),
            None => return false,
        },
        Field::FirstSlice => match ranged {
            Some(v) => chart.first_slice_angle = v.round() as u32,
            None => return false,
        },
        Field::Explosion => {
            let (Some(v), Some(s)) = (ranged, series) else {
                return false;
            };
            match element {
                ChartElement::Point(_, k) => {
                    chart.series[s].point_mut(k as usize).explosion = Some(v.round() as u32)
                }
                _ => {
                    chart.series[s].explosion = v.round() as u32;
                    // "Pie Explosion" moves every slice.
                    for point in &mut chart.series[s].points {
                        point.explosion = None;
                    }
                    chart.series[s].points.retain(|p| p.color.is_some());
                }
            }
        }
        Field::HoleSize => match ranged {
            Some(v) => chart.hole_size = v.round() as u32,
            None => return false,
        },
        Field::BubbleScale => match ranged {
            Some(v) => chart.bubble_scale = v.round() as u32,
            None => return false,
        },
        Field::AxisMin
        | Field::AxisMax
        | Field::AxisMajor
        | Field::AxisMinor
        | Field::AxisCross
        | Field::LogBase => {
            let ChartElement::Axis(id) = element else {
                return false;
            };
            // An emptied field goes back to automatic.
            if typed.is_none() && !text.trim().is_empty() {
                return false;
            }
            let axis = id.of_mut(&mut chart.axes);
            match field {
                Field::AxisMin => axis.min = typed,
                Field::AxisMax => axis.max = typed,
                Field::AxisMajor => axis.major_unit = typed.filter(|v| *v > 0.0),
                Field::AxisMinor => axis.minor_unit = typed.filter(|v| *v > 0.0),
                Field::AxisCross => {
                    axis.crossing = typed.map_or(AxisCrossing::Auto, AxisCrossing::At)
                }
                _ => match ranged {
                    Some(base) => axis.log_base = Some(base),
                    None => return false,
                },
            }
        }
        Field::NumberFormat => {
            let code = format::code_from_locale(text.trim(), locale);
            let code = (!code.is_empty() && !code.eq_ignore_ascii_case("General")).then_some(code);
            match element {
                ChartElement::Axis(id) => id.of_mut(&mut chart.axes).number_format = code,
                ChartElement::DataLabels(_) => match labels_mut(chart, element) {
                    Some(labels) => labels.number_format = code,
                    None => return false,
                },
                _ => return false,
            }
        }
        Field::TrendOrder
        | Field::TrendPeriod
        | Field::TrendName
        | Field::Forward
        | Field::Backward
        | Field::Intercept => {
            let ChartElement::Trendline(s, i) = element else {
                return false;
            };
            let Some(trend) = chart
                .series
                .get_mut(s as usize)
                .and_then(|x| x.trendlines.get_mut(i as usize))
            else {
                return false;
            };
            match (field, ranged) {
                (Field::TrendName, _) => {
                    trend.name = (!text.trim().is_empty()).then(|| text.trim().to_string())
                }
                (Field::TrendOrder, Some(v)) => trend.kind = TrendKind::Polynomial(v.round() as u8),
                (Field::TrendPeriod, Some(v)) => {
                    trend.kind = TrendKind::MovingAverage(v.round() as u32)
                }
                (Field::Forward, Some(v)) => trend.forward = v,
                (Field::Backward, Some(v)) => trend.backward = v,
                (Field::Intercept, Some(v)) => trend.intercept = Some(v),
                _ => return false,
            }
        }
        Field::ErrorFixed | Field::ErrorPercent | Field::ErrorStdDev => {
            let ChartElement::ErrorBars(s, i) = element else {
                return false;
            };
            let (Some(v), Some(bars)) = (
                ranged,
                chart
                    .series
                    .get_mut(s as usize)
                    .and_then(|x| x.error_bars.get_mut(i as usize)),
            ) else {
                return false;
            };
            bars.amount = match field {
                Field::ErrorFixed => ErrorAmount::Fixed(v),
                Field::ErrorPercent => ErrorAmount::Percent(v),
                _ => ErrorAmount::StdDev(v),
            };
        }
        Field::RotationX | Field::RotationY | Field::Perspective | Field::Depth => {
            let Some(v) = ranged else { return false };
            let mut view = view(chart);
            match field {
                Field::RotationX => view.rot_y = v.round() as u32 % 360,
                Field::RotationY => view.rot_x = v.round() as i32,
                Field::Perspective => view.perspective = (v * 2.0).round() as u32,
                _ => view.depth_percent = v.round() as u32,
            }
            chart.view3d = Some(view);
        }
        Field::Height | Field::Width => {
            let Some(v) = typed.filter(|v| *v > 0.0) else {
                return false;
            };
            let pixels = (v * length_unit().0) as f32;
            if field == Field::Height {
                chart.size.1 = pixels.clamp(80.0, 4000.0);
            } else {
                chart.size.0 = pixels.clamp(120.0, 4000.0);
            }
        }
        Field::Text => {
            let text = text.trim();
            match element {
                ChartElement::Title if text.is_empty() => return false,
                ChartElement::Title => chart.title = ChartTitle::Text(text.to_string()),
                ChartElement::AxisTitle(id) if !text.is_empty() => {
                    id.of_mut(&mut chart.axes).title = Some(text.to_string())
                }
                _ => return false,
            }
        }
        Field::FontName => {
            // An emptied field goes back to the font of the chart.
            let name = text.trim();
            let font = (!name.is_empty()).then(|| std::sync::Arc::<str>::from(name));
            return element.format_text(chart, |text| text.font = font.clone());
        }
        Field::FontSize => {
            let Some(size) = ranged else { return false };
            // Half points, as everywhere in Excel.
            let size = ((size * 2.0).round() / 2.0) as f32;
            return element.format_text(chart, |text| text.size = Some(size));
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::ast::RefExpr;
    use opensheet_engine::chart::{ChartKind, ErrorBars, ErrorDirection, Series, Trendline};
    use opensheet_engine::chart_layout::AxisId;

    fn chart(kind: ChartKind) -> Chart {
        let mut chart = Chart::new(kind);
        chart.series = (0..2).map(|i| Series::new(RefExpr::cell(0, i))).collect();
        chart
    }

    fn en() -> &'static Locale {
        crate::i18n::set_lang(Lang::En);
        Locale::en()
    }

    #[test]
    fn numbers_are_read_with_their_unit() {
        assert_eq!(parse_number("2.25 pt", '.'), Some(2.25));
        assert_eq!(parse_number("150 %", '.'), Some(150.0));
        assert_eq!(parse_number("-27%", '.'), Some(-27.0));
        assert_eq!(parse_number("12,7 cm", ','), Some(12.7));
        assert_eq!(parse_number("1 234,5", ','), Some(1234.5));
        assert_eq!(parse_number("abc", '.'), None);
        assert_eq!(parse_number("", '.'), None);
    }

    #[test]
    fn axis_bounds_show_the_scale_and_reset_when_emptied() {
        let locale = en();
        let mut c = chart(ChartKind::Column);
        let axis = ChartElement::Axis(AxisId::Value);
        let scale = Scale {
            min: 0.0,
            max: 20.0,
            major: 2.0,
            minor: 0.4,
            log_base: None,
            reversed: false,
        };
        assert_eq!(value(&c, axis, Field::AxisMax, Some(&scale), locale), "20");
        assert!(set(&mut c, axis, Field::AxisMax, "50", locale));
        assert_eq!(c.axes.value.max, Some(50.0));
        assert_eq!(value(&c, axis, Field::AxisMax, Some(&scale), locale), "50");
        assert!(!set(&mut c, axis, Field::AxisMax, "fifty", locale));
        assert!(set(&mut c, axis, Field::AxisMax, "", locale));
        assert_eq!(c.axes.value.max, None);
        assert!(set(&mut c, axis, Field::AxisCross, "3", locale));
        assert_eq!(c.axes.value.crossing, AxisCrossing::At(3.0));
        // A field of the axis says nothing of another element.
        assert!(!set(
            &mut c,
            ChartElement::Title,
            Field::AxisMin,
            "1",
            locale
        ));
    }

    #[test]
    fn series_fields() {
        let locale = en();
        let mut c = chart(ChartKind::Column);
        let series = ChartElement::Series(1);
        assert_eq!(value(&c, series, Field::GapWidth, None, locale), "219 %");
        assert_eq!(value(&c, series, Field::Overlap, None, locale), "-27 %");
        assert!(set(&mut c, series, Field::GapWidth, "900 %", locale));
        assert_eq!(c.gap_width, Some(500));
        assert!(set(&mut c, series, Field::Overlap, "-150", locale));
        assert_eq!(c.overlap, Some(-100));

        let mut line = chart(ChartKind::LineMarkers);
        assert_eq!(
            value(&line, series, Field::LineWidth, None, locale),
            "2.25 pt"
        );
        assert_eq!(value(&line, series, Field::MarkerSize, None, locale), "5");
        assert!(set(&mut line, series, Field::MarkerSize, "9", locale));
        assert_eq!(
            line.series[1].marker.as_ref().map(|m| (m.shape, m.size)),
            Some((MarkerShape::Circle, 9))
        );
        assert!(set(&mut line, series, Field::LineWidth, "3 pt", locale));
        assert_eq!(line.series[1].line.width, Some(3.0));

        let mut pie = chart(ChartKind::Pie);
        assert!(set(
            &mut pie,
            ChartElement::Point(0, 2),
            Field::Explosion,
            "20",
            locale
        ));
        assert_eq!(
            value(
                &pie,
                ChartElement::Point(0, 2),
                Field::Explosion,
                None,
                locale
            ),
            "20 %"
        );
        assert_eq!(
            value(
                &pie,
                ChartElement::Series(0),
                Field::Explosion,
                None,
                locale
            ),
            "0 %"
        );
        assert!(set(
            &mut pie,
            ChartElement::Series(0),
            Field::Explosion,
            "10",
            locale
        ));
        assert!(pie.series[0].points.is_empty() && pie.series[0].explosion == 10);
    }

    #[test]
    fn fills_follow_the_element() {
        let mut c = chart(ChartKind::Column);
        let red = Color(0xFF0000);
        set_fill(&mut c, ChartElement::PlotArea, Fill::Solid(red));
        assert_eq!(fill_of(&c, ChartElement::PlotArea), Some(Fill::Solid(red)));
        set_fill(&mut c, ChartElement::Series(0), Fill::Solid(red));
        assert_eq!(c.series[0].color, Some(red));
        set_fill(&mut c, ChartElement::Point(1, 3), Fill::Solid(red));
        assert_eq!(
            fill_of(&c, ChartElement::Point(1, 3)),
            Some(Fill::Solid(red))
        );
        assert_eq!(fill_of(&c, ChartElement::Point(1, 0)), Some(Fill::Auto));
        set_fill(&mut c, ChartElement::Point(1, 3), Fill::Auto);
        assert!(c.series[1].points.is_empty());
        assert_eq!(fill_of(&c, ChartElement::Legend), None);
        assert!(line_of(&c, ChartElement::Legend).is_none());
        assert!(!is_lined(&c, 0) && is_lined(&chart(ChartKind::Scatter), 0));
    }

    #[test]
    fn trendline_error_bar_and_view_fields() {
        let locale = en();
        let mut c = chart(ChartKind::Column3D);
        c.series[0]
            .trendlines
            .push(Trendline::new(TrendKind::Linear));
        c.series[0].error_bars.push(ErrorBars {
            horizontal: false,
            direction: ErrorDirection::Both,
            amount: ErrorAmount::StdErr,
            end_cap: true,
        });
        let trend = ChartElement::Trendline(0, 0);
        assert!(set(&mut c, trend, Field::TrendOrder, "9", locale));
        assert_eq!(c.series[0].trendlines[0].kind, TrendKind::Polynomial(6));
        assert!(set(&mut c, trend, Field::Forward, "2.5", locale));
        assert!(set(&mut c, trend, Field::TrendName, " Trend ", locale));
        assert_eq!(c.series[0].trendlines[0].name.as_deref(), Some("Trend"));
        assert_eq!(value(&c, trend, Field::Forward, None, locale), "2.5");
        assert!(set(
            &mut c,
            ChartElement::ErrorBars(0, 0),
            Field::ErrorPercent,
            "12 %",
            locale
        ));
        assert_eq!(c.series[0].error_bars[0].amount, ErrorAmount::Percent(12.0));
        // Excel's X rotation turns the chart around its vertical axis.
        assert_eq!(
            value(&c, ChartElement::ChartArea, Field::RotationX, None, locale),
            "20°"
        );
        assert!(set(
            &mut c,
            ChartElement::ChartArea,
            Field::RotationX,
            "40",
            locale
        ));
        assert!(set(
            &mut c,
            ChartElement::ChartArea,
            Field::Perspective,
            "15",
            locale
        ));
        let view = c.view3d.unwrap();
        assert_eq!((view.rot_y, view.rot_x, view.perspective), (40, 15, 30));
        assert_eq!(
            value(&c, ChartElement::ChartArea, Field::Width, None, locale),
            "5\""
        );
        assert!(set(
            &mut c,
            ChartElement::ChartArea,
            Field::Height,
            "4",
            locale
        ));
        assert_eq!(c.size, (480.0, 384.0));
    }

    #[test]
    fn number_formats_and_titles() {
        let locale = en();
        let mut c = chart(ChartKind::Column);
        let axis = ChartElement::Axis(AxisId::Value);
        assert_eq!(
            value(&c, axis, Field::NumberFormat, None, locale),
            "General"
        );
        assert!(set(&mut c, axis, Field::NumberFormat, "0.0%", locale));
        assert_eq!(c.axes.value.number_format.as_deref(), Some("0.0%"));
        assert!(set(&mut c, axis, Field::NumberFormat, "General", locale));
        assert_eq!(c.axes.value.number_format, None);
        c.data_labels = true;
        assert!(set(
            &mut c,
            ChartElement::DataLabels(1),
            Field::NumberFormat,
            "0.00",
            locale
        ));
        assert_eq!(
            c.series[1]
                .labels
                .as_ref()
                .and_then(|l| l.number_format.as_deref()),
            Some("0.00")
        );
        assert!(c.series[0].labels.is_none());
        assert!(set(
            &mut c,
            ChartElement::Title,
            Field::Text,
            "Sales",
            locale
        ));
        assert_eq!(c.title, ChartTitle::Text("Sales".into()));
        assert!(!set(&mut c, ChartElement::Title, Field::Text, "  ", locale));
    }
}
