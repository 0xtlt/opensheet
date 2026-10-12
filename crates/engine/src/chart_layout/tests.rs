//! The layout, checked with a text measurer that needs no font.

use super::*;
use crate::chart::{
    Axis, AxisCrossing, DataLabels, DisplayUnit, ErrorAmount, ErrorBars, ErrorDirection, LabelPosition, Marker,
    TickLabels, Trendline, View3D,
};
use crate::refs::Rect;
use crate::tests::wb;
use crate::workbook::Workbook;

/// Every character is half as wide as the font is tall.
struct Fixed;

impl TextMeasure for Fixed {
    fn width(&self, text: &str, size: f32, _bold: bool) -> f32 {
        text.chars().count() as f32 * size * 0.5
    }
}

const SIZE: (f32, f32) = (480.0, 288.0);

fn sales() -> Workbook {
    wb(&[
        ("A1", "Month"),
        ("B1", "North"),
        ("C1", "South"),
        ("D1", "West"),
        ("A2", "Jan"),
        ("B2", "12"),
        ("C2", "8"),
        ("D2", "5"),
        ("A3", "Feb"),
        ("B3", "15"),
        ("C3", "11"),
        ("D3", "7"),
        ("A4", "Mar"),
        ("B4", "9"),
        ("C4", "14"),
        ("D4", "10"),
        ("A5", "Apr"),
        ("B5", "18"),
        ("C5", "10"),
        ("D5", "12"),
    ])
}

fn chart_of(w: &Workbook, range: &str, kind: ChartKind) -> Chart {
    w.chart_from_range(0, Rect::parse(range).unwrap(), kind)
}

fn lay(w: &Workbook, chart: &Chart) -> Vec<Prim> {
    lay_at(w, chart, SIZE, 1.0)
}

fn lay_at(w: &Workbook, chart: &Chart, size: (f32, f32), scale: f32) -> Vec<Prim> {
    let data = w.chart_data(0, chart);
    let options = LayoutOptions {
        scale,
        locale: w.locale,
        labels: Labels::default(),
    };
    layout_chart(chart, &data, size, &options, &Fixed)
}

fn texts(prims: &[Prim]) -> Vec<&TextRun> {
    prims
        .iter()
        .filter_map(|p| match p {
            Prim::Text(t) => Some(t),
            _ => None,
        })
        .collect()
}

fn has_text(prims: &[Prim], text: &str) -> bool {
    texts(prims).iter().any(|t| t.text == text)
}

fn find_text<'a>(prims: &'a [Prim], text: &str) -> &'a TextRun {
    texts(prims).into_iter().find(|t| t.text == text).unwrap_or_else(|| panic!("no text {text:?}"))
}

/// Filled rectangles of a colour: (x, y, w, h).
fn rects(prims: &[Prim], color: u32) -> Vec<(f32, f32, f32, f32)> {
    prims
        .iter()
        .filter_map(|p| match p {
            Prim::Rect { x, y, w, h, fill: Some(ink), .. } if ink.color == Color(color) => Some((*x, *y, *w, *h)),
            _ => None,
        })
        .collect()
}

fn lines(prims: &[Prim], color: Color) -> Vec<(f32, f32, f32, f32)> {
    prims
        .iter()
        .filter_map(|p| match p {
            Prim::Line { x0, y0, x1, y1, stroke } if stroke.ink.color == color => Some((*x0, *y0, *x1, *y1)),
            _ => None,
        })
        .collect()
}

/// Paths with a stroke of a colour: their points (ends of each step).
fn stroked(prims: &[Prim], color: u32) -> Vec<(Vec<(f32, f32)>, Stroke)> {
    prims
        .iter()
        .filter_map(|p| match p {
            Prim::Path { segs, stroke: Some(stroke), .. } if stroke.ink.color == Color(color) => {
                let pts = segs
                    .iter()
                    .filter_map(|s| match s {
                        Seg::Move(x, y) | Seg::Line(x, y) | Seg::Cubic(_, _, _, _, x, y) => Some((*x, *y)),
                        Seg::Close => None,
                    })
                    .collect();
                Some((pts, *stroke))
            }
            _ => None,
        })
        .collect()
}

fn filled_paths(prims: &[Prim], color: u32) -> usize {
    prims
        .iter()
        .filter(|p| matches!(p, Prim::Path { fill: Some(ink), .. } if ink.color == Color(color)))
        .count()
}

/// Every number of every primitive is finite and (texts aside) inside the
/// chart, give or take the stroke of the frame.
fn assert_sane(prims: &[Prim], size: (f32, f32), what: &str) {
    let inside = |x: f32, y: f32| {
        assert!(x.is_finite() && y.is_finite(), "{what}: not a number");
        assert!(x >= -2.0 && x <= size.0 + 2.0 && y >= -2.0 && y <= size.1 + 2.0, "{what}: ({x}, {y}) outside");
    };
    for p in prims {
        match p {
            Prim::Rect { x, y, w, h, .. } => {
                assert!(*w >= 0.0 && *h >= 0.0, "{what}: negative rectangle");
                inside(*x, *y);
                inside(x + w, y + h);
            }
            Prim::Line { x0, y0, x1, y1, .. } => {
                inside(*x0, *y0);
                inside(*x1, *y1);
            }
            Prim::Path { segs, .. } => {
                for s in segs {
                    match s {
                        Seg::Move(x, y) | Seg::Line(x, y) => inside(*x, *y),
                        Seg::Cubic(a, b, c, d, x, y) => {
                            assert!(a.is_finite() && b.is_finite() && c.is_finite() && d.is_finite());
                            inside(*x, *y);
                        }
                        Seg::Close => {}
                    }
                }
            }
            Prim::Text(t) => assert!(t.x.is_finite() && t.y.is_finite() && t.size > 0.0, "{what}: bad text"),
        }
    }
}

const NORTH: u32 = 0x156082;
const SOUTH: u32 = 0xE97132;
const WEST: u32 = 0x196B24;

#[test]
fn clustered_columns_keep_their_look() {
    let w = sales();
    let chart = chart_of(&w, "A1:D5", ChartKind::Column);
    let prims = lay(&w, &chart);
    assert_sane(&prims, SIZE, "column");
    // The frame, then the plot: 12 bars, 5 gridlines (0 to 20 by 5).
    assert!(matches!(&prims[0], Prim::Rect { x, y, w, h, fill: Some(_), stroke: Some(_) } if (*x, *y, *w, *h) == (0.0, 0.0, 480.0, 288.0)));
    let north = rects(&prims, NORTH);
    // Four bars and the legend swatch.
    assert_eq!(north.len(), 5);
    assert_eq!(rects(&prims, SOUTH).len(), 5);
    for label in ["0", "5", "10", "15", "20", "Jan", "Feb", "Mar", "Apr", "North", "South", "West", "Chart Title"] {
        assert!(has_text(&prims, label), "{label}");
    }
    assert_eq!(lines(&prims, GRIDLINE).len(), 5);

    // The geometry of the plot, as the chart was drawn before the layout
    // moved here: title 18.67 px, legend at the bottom, labels 12 px.
    let label_h = 15.0;
    let top = 10.0 + line_height(18.67) + 4.0;
    let bottom = 288.0 - 10.0 - label_h - 6.0;
    let px0 = 10.0 + 12.0 + 8.0; // "20" is two characters of 6 px
    let (px1, py0, py1) = (470.0, top + label_h / 2.0, bottom - label_h - 6.0);
    let slot = (px1 - px0) / 4.0;
    let bar = slot / (3.0 + 0.27 * 2.0 + 2.19);
    let cluster = bar * (3.0 + 0.27 * 2.0);
    let y_of = |v: f32| py1 - v / 20.0 * (py1 - py0);
    let first = north.iter().find(|r| r.3 > 20.0).unwrap();
    assert!((first.0 - (px0 + (slot - cluster) / 2.0)).abs() < 0.01, "{first:?}");
    assert!((first.2 - bar).abs() < 0.01);
    assert!((first.1 - y_of(12.0)).abs() < 0.01);
    assert!((first.1 + first.3 - y_of(0.0).round()).abs() < 0.01);
    let second = rects(&prims, SOUTH).into_iter().find(|r| r.3 > 20.0).unwrap();
    assert!((second.0 - first.0 - bar * 1.27).abs() < 0.01);
    // Value labels end 6 px left of the plot, centred on their gridline.
    let twenty = find_text(&prims, "20");
    assert_eq!((twenty.anchor, twenty.x), (TextAnchor::End, px0 - 6.0));
    assert_eq!(twenty.y, y_of(20.0).round());
    // Category labels sit 5 px under the plot.
    let jan = find_text(&prims, "Jan");
    assert!((jan.y - (py1 + 5.0 + label_h / 2.0)).abs() < 0.01);
    assert!((jan.x - (px0 + slot / 2.0 - 9.0)).abs() < 0.01);
    // The title is centred.
    let title = find_text(&prims, "Chart Title");
    assert!((title.x - (480.0 - 11.0 * 18.67 * 0.5) / 2.0).abs() < 0.01);
}

#[test]
fn zoom_scales_everything() {
    let w = sales();
    let chart = chart_of(&w, "A1:D5", ChartKind::Column);
    let small = lay(&w, &chart);
    let big = lay_at(&w, &chart, (960.0, 576.0), 2.0);
    assert_eq!(small.len(), big.len());
    assert_sane(&big, (960.0, 576.0), "zoomed");
    assert_eq!(find_text(&big, "Jan").size, 24.0);
    let (a, b) = (rects(&small, NORTH), rects(&big, NORTH));
    assert!((b[0].2 - 2.0 * a[0].2).abs() < 1.0);
}

#[test]
fn bars_stacks_and_percent() {
    let w = sales();
    // Horizontal bars: the first category at the bottom, values along the bottom.
    let prims = lay(&w, &chart_of(&w, "A1:D5", ChartKind::Bar));
    assert_sane(&prims, SIZE, "bar");
    let north: Vec<_> = rects(&prims, NORTH).into_iter().filter(|r| r.2 > 20.0).collect();
    assert_eq!(north.len(), 4);
    assert!(north[0].1 > north[3].1, "Jan is under Apr");
    assert!(north[3].2 > north[0].2, "18 is longer than 12");
    let jan = find_text(&prims, "Jan");
    let apr = find_text(&prims, "Apr");
    assert!(jan.y > apr.y && jan.x == apr.x);

    // Stacked: one bar per category, pieces on top of each other.
    let prims = lay(&w, &chart_of(&w, "A1:D5", ChartKind::StackedColumn));
    let (n, s) = (rects(&prims, NORTH), rects(&prims, SOUTH));
    let (n0, s0) = (n.iter().find(|r| r.3 > 20.0).unwrap(), s.iter().find(|r| r.3 > 20.0).unwrap());
    assert_eq!(n0.0, s0.0);
    assert!((s0.1 + s0.3 - n0.1).abs() < 0.01, "South stands on North");
    assert!(has_text(&prims, "40") && !has_text(&prims, "100%"));

    // To 100 %: every stack is as tall as the plot, the axis counts in percent.
    let prims = lay(&w, &chart_of(&w, "A1:D5", ChartKind::PercentStackedColumn));
    for label in ["0%", "50%", "100%"] {
        assert!(has_text(&prims, label), "{label}");
    }
    let tops: Vec<f32> = rects(&prims, WEST).iter().filter(|r| r.3 > 20.0).map(|r| r.1).collect();
    assert_eq!(tops.len(), 4);
    assert!(tops.iter().all(|t| (t - tops[0]).abs() < 0.01));
    for kind in [ChartKind::StackedBar, ChartKind::PercentStackedBar, ChartKind::StackedArea, ChartKind::PercentStackedArea, ChartKind::StackedLine, ChartKind::PercentStackedLineMarkers] {
        assert_sane(&lay(&w, &chart_of(&w, "A1:D5", kind)), SIZE, &format!("{kind:?}"));
    }
}

#[test]
fn axis_options() {
    let w = sales();
    let mut chart = chart_of(&w, "A1:D5", ChartKind::Column);
    chart.axes.value = Axis {
        min: Some(4.0),
        max: Some(16.0),
        major_unit: Some(4.0),
        minor_gridlines: true,
        title: Some("Units".into()),
        number_format: Some("0.0".into()),
        ..chart.axes.value.clone()
    };
    chart.axes.category.title = Some("Month".into());
    let prims = lay(&w, &chart);
    assert_sane(&prims, SIZE, "manual scale");
    for label in ["4.0", "8.0", "12.0", "16.0"] {
        assert!(has_text(&prims, label), "{label}");
    }
    assert!(!has_text(&prims, "0.0") && !has_text(&prims, "20.0"));
    assert_eq!(lines(&prims, GRIDLINE).len(), 4);
    assert_eq!(lines(&prims, MINOR_GRIDLINE).len(), 12);
    // The 18 of April is cut off at the top of the plot.
    let plot_top = lines(&prims, GRIDLINE).iter().map(|l| l.1).fold(f32::INFINITY, f32::min) - 0.5;
    assert!(rects(&prims, NORTH).iter().all(|r| r.1 >= plot_top - 0.51));
    // Titles: the value axis' reads upwards, left of the labels.
    let units = find_text(&prims, "Units");
    assert_eq!((units.rotation, units.anchor), (-90.0, TextAnchor::Middle));
    assert!(units.x < find_text(&prims, "16.0").x - 20.0);
    let month = find_text(&prims, "Month");
    assert_eq!(month.rotation, 0.0);
    assert!(month.y > find_text(&prims, "Jan").y);

    // Reversed values: the category axis moves to the top, bars hang from it.
    let mut chart = chart_of(&w, "A1:D5", ChartKind::Column);
    chart.axes.value.reversed = true;
    let prims = lay(&w, &chart);
    assert_sane(&prims, SIZE, "reversed");
    assert!(find_text(&prims, "0").y < find_text(&prims, "20").y);
    assert!(find_text(&prims, "Jan").y < find_text(&prims, "0").y, "labels above the plot");
    let bars: Vec<_> = rects(&prims, NORTH).into_iter().filter(|r| r.3 > 20.0).collect();
    assert!(bars.iter().all(|r| (r.1 - bars[0].1).abs() < 0.01), "bars share their top");

    // Reversed categories: April first, the value axis on the right.
    let mut chart = chart_of(&w, "A1:D5", ChartKind::Column);
    chart.axes.category.reversed = true;
    let prims = lay(&w, &chart);
    assert!(find_text(&prims, "Apr").x < find_text(&prims, "Jan").x);
    assert_eq!(find_text(&prims, "20").anchor, TextAnchor::Start);

    // Hidden axes leave their room to the plot; labels can be switched off.
    let mut chart = chart_of(&w, "A1:D5", ChartKind::Column);
    chart.axes.value.visible = false;
    chart.axes.category.labels = TickLabels::None;
    let prims = lay(&w, &chart);
    assert!(!has_text(&prims, "20") && !has_text(&prims, "Jan"));
    assert_eq!(lines(&prims, GRIDLINE).len(), 5);
    assert_eq!(lines(&prims, GRIDLINE)[0].0, 10.0);

    // The category axis crossing at 10: bars go up and down from there.
    let mut chart = chart_of(&w, "A1:D5", ChartKind::Column);
    chart.axes.value.crossing = AxisCrossing::At(10.0);
    let prims = lay(&w, &chart);
    let bars: Vec<_> = rects(&prims, NORTH).into_iter().filter(|r| r.3 > 5.0).collect();
    assert!(bars[2].1 > bars[0].1, "9 hangs under the axis: {bars:?}");
    assert!((bars[2].1 - (bars[0].1 + bars[0].3)).abs() < 0.01);

    // Display units and a logarithmic scale.
    let big = wb(&[("A1", "a"), ("B1", "2000"), ("A2", "b"), ("B2", "150000"), ("A3", "c"), ("B3", "30")]);
    let mut chart = chart_of(&big, "A1:B3", ChartKind::Column);
    chart.axes.value.display_unit = DisplayUnit::Thousands;
    let prims = lay(&big, &chart);
    assert!(has_text(&prims, "Thousands") && has_text(&prims, "200") && !has_text(&prims, "200000"));
    chart.axes.value.display_unit = DisplayUnit::None;
    chart.axes.value.log_base = Some(10.0);
    let prims = lay(&big, &chart);
    assert_sane(&prims, SIZE, "log");
    for label in ["10", "100", "1000", "10000", "100000", "1000000"] {
        assert!(has_text(&prims, label), "{label}");
    }
    let ys: Vec<f32> = ["10", "100", "1000"].iter().map(|t| find_text(&prims, t).y).collect();
    assert!(((ys[0] - ys[1]) - (ys[1] - ys[2])).abs() <= 1.0, "decades are evenly spaced");
}

#[test]
fn combination_with_secondary_axis() {
    let w = wb(&[
        ("A1", "Month"),
        ("B1", "Units"),
        ("C1", "Price"),
        ("A2", "Jan"),
        ("B2", "120"),
        ("C2", "1.5"),
        ("A3", "Feb"),
        ("B3", "180"),
        ("C3", "2.5"),
        ("A4", "Mar"),
        ("B4", "90"),
        ("C4", "2"),
    ]);
    let mut chart = chart_of(&w, "A1:C4", ChartKind::Column);
    chart.series[1].kind = Some(ChartKind::LineMarkers);
    chart.series[1].secondary = true;
    chart.axes.secondary_value.title = Some("Price".into());
    let prims = lay(&w, &chart);
    assert_sane(&prims, SIZE, "combo");
    // Columns of the first series only, at the full width of a lone series.
    let bars: Vec<_> = rects(&prims, NORTH).into_iter().filter(|r| r.3 > 20.0).collect();
    assert_eq!(bars.len(), 3);
    assert!(rects(&prims, SOUTH).iter().all(|r| r.3 <= 20.0));
    // The line: one path of three points, 3 px wide, with three markers.
    let line = stroked(&prims, SOUTH);
    let (pts, stroke) = line.iter().find(|(pts, _)| pts.len() == 3).expect("the line");
    assert_eq!(stroke.width, 3.0);
    assert!(pts[1].1 < pts[0].1 && pts[1].1 < pts[2].1);
    assert_eq!(filled_paths(&prims, SOUTH), 3 + 1, "markers, and the one of the legend");
    // Its scale stands on the right, with its own numbers.
    let primary = find_text(&prims, "200");
    let secondary = find_text(&prims, "2.5");
    assert_eq!((primary.anchor, secondary.anchor), (TextAnchor::End, TextAnchor::Start));
    assert!(secondary.x > 400.0 && primary.x < 60.0);
    // 2.5 on the secondary scale (0 to 3) is higher than 180 on the primary (0 to 200).
    let top_of_bar = bars.iter().map(|r| r.1).fold(f32::INFINITY, f32::min);
    assert!(pts[1].1 > top_of_bar - 40.0);
    assert_eq!(find_text(&prims, "Price").rotation, -90.0);
    assert!(find_text(&prims, "Price").x > secondary.x);

    // A series of a kind that does not go with columns is drawn as a column.
    chart.series[1].kind = Some(ChartKind::Pie);
    chart.series[1].secondary = false;
    let prims = lay(&w, &chart);
    assert_eq!(rects(&prims, SOUTH).len(), 3 + 1);
}

#[test]
fn lines_markers_and_gaps() {
    let w = wb(&[("A1", "x"), ("B1", "y"), ("A2", "a"), ("B2", "1"), ("A3", "b"), ("A4", "c"), ("B4", "3"), ("A5", "d"), ("B5", "2")]);
    let mut chart = chart_of(&w, "A1:B5", ChartKind::Line);
    // A blank cell breaks the line; the point left alone still shows.
    let prims = lay(&w, &chart);
    let paths = stroked(&prims, NORTH);
    assert_eq!(paths.iter().filter(|(pts, _)| pts.len() == 2).count(), 1);
    assert_eq!(filled_paths(&prims, NORTH), 1, "the lone first point");
    chart.blanks = crate::chart::BlanksAs::Span;
    assert!(stroked(&lay(&w, &chart), NORTH).iter().any(|(pts, _)| pts.len() == 3));
    chart.blanks = crate::chart::BlanksAs::Zero;
    assert!(stroked(&lay(&w, &chart), NORTH).iter().any(|(pts, _)| pts.len() == 4));

    // Markers, dashes, widths, smoothing.
    chart.kind = ChartKind::LineMarkers;
    chart.series[0].marker = Some(Marker {
        fill: Some(Color(0xFF0000)),
        size: 9,
        ..Marker::new(MarkerShape::Square)
    });
    chart.series[0].line.width = Some(1.5);
    chart.series[0].line.dash = Dash::Dash;
    let prims = lay(&w, &chart);
    assert_eq!(rects(&prims, 0xFF0000).iter().filter(|r| (r.2 - 12.6).abs() < 0.01).count(), 4);
    let (_, stroke) = stroked(&prims, NORTH).into_iter().find(|(pts, _)| pts.len() == 4).unwrap();
    assert_eq!((stroke.width, stroke.dash), (2.0, Dash::Dash));
    chart.series[0].smooth = Some(true);
    let smooth = stroked(&lay(&w, &chart), NORTH);
    // The curve dips under the axis between 1 and 0, where it is cut.
    assert!(smooth.iter().map(|(pts, _)| pts.len()).sum::<usize>() > 30);
    // No line at all: markers only.
    chart.series[0].line.shown = Some(false);
    assert!(stroked(&lay(&w, &chart), NORTH).is_empty());
}

#[test]
fn scatter_and_bubbles() {
    let w = wb(&[
        ("A1", "x"),
        ("B1", "y"),
        ("C1", "size"),
        ("A2", "1"),
        ("B2", "10"),
        ("C2", "4"),
        ("A3", "2"),
        ("B3", "30"),
        ("C3", "16"),
        ("A4", "4"),
        ("B4", "20"),
        ("C4", "1"),
    ]);
    let chart = chart_of(&w, "A1:B4", ChartKind::Scatter);
    let prims = lay(&w, &chart);
    assert_sane(&prims, SIZE, "scatter");
    assert_eq!(filled_paths(&prims, NORTH), 3);
    assert!(stroked(&prims, NORTH).is_empty(), "markers only");
    // Both scales have gridlines and a line of their own.
    assert!(lines(&prims, GRIDLINE).len() >= 10);
    assert_eq!(lines(&prims, AXIS).len(), 2);
    for kind in [ChartKind::ScatterLines, ChartKind::ScatterLinesMarkers, ChartKind::ScatterSmooth, ChartKind::ScatterSmoothMarkers] {
        let prims = lay(&w, &chart_of(&w, "A1:B4", kind));
        let paths = stroked(&prims, NORTH);
        assert_eq!(paths.len(), 1, "{kind:?}");
        assert_eq!(paths[0].1.width, 2.0, "1.5 pt");
        assert_eq!(paths[0].0.len() > 3, kind.is_smooth());
        assert_eq!(filled_paths(&prims, NORTH), if kind.has_markers() { 3 } else { 0 });
    }

    let chart = chart_of(&w, "A1:C4", ChartKind::Bubble);
    assert_eq!(chart.series.len(), 1);
    assert!(chart.series[0].bubble_sizes.is_some());
    let prims = lay(&w, &chart);
    assert_sane(&prims, SIZE, "bubble");
    // Areas follow the sizes: 16 is four times 4, so twice the radius.
    let radii: Vec<f32> = prims
        .iter()
        .filter_map(|p| match p {
            Prim::Path { segs, fill: Some(ink), .. } if ink.opacity < 1.0 => {
                let xs: Vec<f32> = segs
                    .iter()
                    .filter_map(|s| match s {
                        Seg::Move(x, _) | Seg::Cubic(_, _, _, _, x, _) => Some(*x),
                        _ => None,
                    })
                    .collect();
                let (lo, hi) = xs.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |a, x| (a.0.min(*x), a.1.max(*x)));
                Some((hi - lo) / 2.0)
            }
            _ => None,
        })
        .collect();
    assert_eq!(radii.len(), 3);
    assert!((radii[1] / radii[0] - 2.0).abs() < 0.01 && (radii[0] / radii[2] - 2.0).abs() < 0.01);
}

#[test]
fn pies_and_doughnuts() {
    let w = sales();
    let mut chart = chart_of(&w, "A1:B5", ChartKind::Pie);
    let prims = lay(&w, &chart);
    assert_sane(&prims, SIZE, "pie");
    // One slice per point, each in its colour, named in the legend.
    for color in [NORTH, SOUTH, WEST, 0x0F9ED5] {
        assert_eq!(filled_paths(&prims, color), 1);
    }
    assert!(has_text(&prims, "Jan") && has_text(&prims, "North"));
    let slice = |prims: &[Prim], color: u32| -> Vec<Seg> {
        prims
            .iter()
            .find_map(|p| match p {
                Prim::Path { segs, fill: Some(ink), .. } if ink.color == Color(color) => Some(segs.clone()),
                _ => None,
            })
            .unwrap()
    };
    // The first slice starts at 12 o'clock: centre, then straight up.
    let first = slice(&prims, NORTH);
    let (Seg::Move(cx, cy), Seg::Line(x, y)) = (first[0], first[1]) else { panic!("{first:?}") };
    assert!((x - cx).abs() < 0.01 && y < cy);
    let r = cy - y;

    // Turned by a quarter, it starts at 3 o'clock.
    chart.first_slice_angle = 90;
    let turned = slice(&lay(&w, &chart), NORTH);
    let (Seg::Move(cx2, cy2), Seg::Line(x2, y2)) = (turned[0], turned[1]) else { panic!() };
    assert!((y2 - cy2).abs() < 0.01 && x2 > cx2);
    chart.first_slice_angle = 0;

    // A slice pulled out moves away from the centre; the pie shrinks to make room.
    chart.series[0].point_mut(0).explosion = Some(25);
    let burst = lay(&w, &chart);
    assert_sane(&burst, SIZE, "exploded");
    let moved = slice(&burst, NORTH);
    let (Seg::Move(mx, my), Seg::Line(_, ey)) = (moved[0], moved[1]) else { panic!() };
    let small = my - ey;
    assert!((small - r / 1.25).abs() < 0.01);
    assert!((((mx - cx).powi(2) + (my - cy).powi(2)).sqrt() - small * 0.25).abs() < 0.01);
    chart.series[0].points.clear();

    // One colour for all, or a colour of its own for one slice.
    chart.vary_colors = Some(false);
    assert_eq!(filled_paths(&lay(&w, &chart), NORTH), 4);
    chart.vary_colors = None;
    chart.series[0].point_mut(2).color = Some(Color(0x123456));
    let prims = lay(&w, &chart);
    assert_eq!((filled_paths(&prims, 0x123456), filled_paths(&prims, WEST)), (1, 0));
    assert_eq!(rects(&prims, 0x123456).len(), 1, "the legend follows");

    // Doughnut: a ring per series, the hole as wide as asked.
    let mut ring = chart_of(&w, "A1:C5", ChartKind::Doughnut);
    assert_eq!(ring.series.len(), 2);
    ring.hole_size = 50;
    let prims = lay(&w, &ring);
    assert_sane(&prims, SIZE, "doughnut");
    assert_eq!(filled_paths(&prims, NORTH), 2);
    // Where the first slice of each ring starts (12 o'clock, outer edge)
    // and ends (12 o'clock again, inner edge).
    let edges = |prims: &[Prim]| -> Vec<(f32, f32)> {
        prims
            .iter()
            .filter_map(|p| match p {
                Prim::Path { segs, fill: Some(ink), .. } if ink.color == Color(NORTH) => {
                    let (Seg::Move(_, outer), Seg::Cubic(.., inner)) = (segs[0], segs[segs.len() - 2]) else { panic!() };
                    Some((outer, inner))
                }
                _ => None,
            })
            .collect()
    };
    let e = edges(&prims);
    // The first series is the inner ring; the rings are equally wide.
    let width = e[0].1 - e[0].0;
    assert!((e[1].1 - e[0].0).abs() < 0.01 && (e[1].1 - e[1].0 - width).abs() < 0.01, "{e:?}");
    // A hole of 80 % leaves two fifths of the room a hole of 50 % leaves.
    ring.hole_size = 80;
    let thin = edges(&lay(&w, &ring));
    assert!((thin[1].0 - e[1].0).abs() < 0.01);
    assert!(((thin[0].1 - thin[0].0) * 2.5 - width).abs() < 0.01, "{thin:?}");

    let pie3d = chart_of(&w, "A1:B5", ChartKind::Pie3D);
    let prims = lay(&w, &pie3d);
    assert_sane(&prims, SIZE, "3-D pie");
    assert!(filled_paths(&prims, shade(Color(SOUTH), -0.36).0) >= 1, "the rim of a front slice");
}

#[test]
fn data_labels() {
    let w = sales();
    let mut chart = chart_of(&w, "A1:C5", ChartKind::Column);
    chart.data_labels = true;
    let prims = lay(&w, &chart);
    // Values over the bars.
    let twelve = find_text(&prims, "12");
    let bar = rects(&prims, NORTH).into_iter().find(|r| r.3 > 20.0).unwrap();
    assert_eq!(twelve.anchor, TextAnchor::Middle);
    assert!((twelve.x - (bar.0 + bar.2 / 2.0)).abs() < 0.01 && twelve.y < bar.1);
    // Inside, at the foot of the bar.
    chart.label_options.position = LabelPosition::InsideBase;
    let prims = lay(&w, &chart);
    let twelve = find_text(&prims, "12");
    assert!(twelve.y > bar.1 + bar.3 - 20.0 && twelve.y < bar.1 + bar.3);

    // What the labels say; a series may differ, or have none.
    chart.label_options = DataLabels {
        series_name: true,
        category: true,
        number_format: Some("0.0".into()),
        separator: Some(" / ".into()),
        ..DataLabels::default()
    };
    chart.series[1].labels = Some(DataLabels::none());
    let prims = lay(&w, &chart);
    assert!(has_text(&prims, "North / Jan / 12.0"));
    assert!(!texts(&prims).iter().any(|t| t.text.starts_with("South /")));
    chart.data_labels = false;
    chart.series[1].labels = Some(DataLabels::default());
    let prims = lay(&w, &chart);
    assert!(has_text(&prims, "14") && !has_text(&prims, "18"));

    // Pies: shares, inside when there is room, outside when asked.
    let mut pie = chart_of(&w, "A1:B5", ChartKind::Pie);
    pie.data_labels = true;
    pie.label_options = DataLabels {
        value: false,
        percent: true,
        category: true,
        ..DataLabels::default()
    };
    let prims = lay(&w, &pie);
    for label in ["Jan, 22%", "Feb, 28%", "Mar, 17%", "Apr, 33%"] {
        assert!(has_text(&prims, label), "{label}: {:?}", texts(&prims).iter().map(|t| &t.text).collect::<Vec<_>>());
    }
    pie.label_options.position = LabelPosition::OutsideEnd;
    assert_sane(&lay(&w, &pie), SIZE, "outside labels");
    // French lists are separated by semicolons.
    let mut fr = sales();
    fr.locale = crate::Locale::fr();
    pie.label_options.position = LabelPosition::Auto;
    assert!(has_text(&lay(&fr, &pie), "Jan; 22%"));

    // Lines: to the right of the point unless told otherwise.
    let mut line = chart_of(&w, "A1:B5", ChartKind::Line);
    line.data_labels = true;
    let prims = lay(&w, &line);
    assert_eq!(find_text(&prims, "12").anchor, TextAnchor::Start);
    line.label_options.position = LabelPosition::Above;
    let above = lay(&w, &line);
    assert_eq!(find_text(&above, "12").anchor, TextAnchor::Middle);
    assert!(find_text(&above, "12").y < find_text(&prims, "12").y);
}

#[test]
fn trendlines_and_error_bars() {
    let w = wb(&[("A1", "x"), ("B1", "y"), ("A2", "1"), ("B2", "3"), ("A3", "2"), ("B3", "5"), ("A4", "3"), ("B4", "7"), ("A5", "4"), ("B5", "9")]);
    let mut chart = chart_of(&w, "A1:B5", ChartKind::Scatter);
    chart.legend = LegendPos::Right;
    chart.series[0].trendlines.push(Trendline {
        show_equation: true,
        show_r2: true,
        forward: 2.0,
        ..Trendline::new(TrendKind::Linear)
    });
    let prims = lay(&w, &chart);
    assert_sane(&prims, SIZE, "trend");
    // A dotted line in the colour of the series, from x = 1 to x = 6.
    let (pts, stroke) = stroked(&prims, NORTH).into_iter().find(|(pts, _)| pts.len() == 2).expect("the trendline");
    assert_eq!((stroke.dash, stroke.width), (Dash::RoundDot, 2.0));
    assert!(pts[1].0 > pts[0].0 && pts[1].1 < pts[0].1);
    assert!(has_text(&prims, "y = 2x + 1") && has_text(&prims, "R² = 1"));
    assert!(has_text(&prims, "Linear (y)"), "named in the legend");
    // The scales make room for the forecast: y = 13 at x = 6.
    assert!(has_text(&prims, "14") || has_text(&prims, "15"));
    assert!(has_text(&prims, "6"));

    // On categories the forecast adds empty categories.
    let mut columns = chart_of(&w, "B1:B5", ChartKind::Column);
    let plain = lay(&w, &columns);
    columns.series[0].trendlines.push(Trendline {
        forward: 2.0,
        name: Some("Trend".into()),
        ..Trendline::new(TrendKind::Polynomial(2))
    });
    columns.series[0].trendlines.push(Trendline::new(TrendKind::MovingAverage(2)));
    columns.legend = LegendPos::TopRight;
    let prims = lay(&w, &columns);
    assert_sane(&prims, SIZE, "category trend");
    let width = |prims: &[Prim]| rects(prims, NORTH).into_iter().find(|r| r.3 > 20.0).unwrap().2;
    assert!(width(&prims) < width(&plain) * 0.75);
    assert!(has_text(&prims, "Trend") && has_text(&prims, "2 per. Mov. Avg. (y)"));
    let curves = stroked(&prims, NORTH);
    assert!(curves.iter().any(|(pts, _)| pts.len() > 50), "the polynomial is a curve");
    assert!(curves.iter().any(|(pts, _)| pts.len() == 3), "three averages of two");
    // One entry taken out of the legend.
    columns.legend_hidden = vec![1];
    assert!(!has_text(&lay(&w, &columns), "Trend"));

    // Error bars: a fixed amount up and down, with caps.
    let mut bars = chart_of(&w, "B1:B5", ChartKind::Column);
    let before = lines(&lay(&w, &bars), TEXT).len();
    bars.series[0].error_bars.push(ErrorBars {
        horizontal: false,
        direction: ErrorDirection::Both,
        amount: ErrorAmount::Fixed(1.0),
        end_cap: true,
    });
    let prims = lay(&w, &bars);
    assert_eq!(lines(&prims, TEXT).len() - before, 4 * 3);
    assert!(has_text(&prims, "12"), "the scale makes room for 9 + 1 and its headroom");
    // No trendline on what cannot have one.
    let mut pie = chart_of(&w, "A1:B5", ChartKind::Pie);
    pie.series[0].trendlines.push(Trendline::new(TrendKind::Linear));
    assert!(!has_text(&lay(&w, &pie), "Linear (y)"));
}

#[test]
fn legend_title_and_areas() {
    let w = sales();
    let mut chart = chart_of(&w, "A1:D5", ChartKind::Line);
    // At the top right: a column of entries starting at the top.
    chart.legend = LegendPos::TopRight;
    let prims = lay(&w, &chart);
    let (north, west) = (find_text(&prims, "North"), find_text(&prims, "West"));
    assert_eq!(north.x, west.x);
    assert!(north.x > 400.0 && north.y < 60.0 && west.y == north.y + 30.0);
    // Laid over the plot, legend and title take no room from it.
    let right_end = |prims: &[Prim]| lines(prims, GRIDLINE).iter().map(|l| l.2).fold(0.0, f32::max);
    let top_end = |prims: &[Prim]| lines(prims, GRIDLINE).iter().map(|l| l.1).fold(f32::INFINITY, f32::min);
    assert!(right_end(&prims) < 430.0);
    chart.legend_overlay = true;
    let over = lay(&w, &chart);
    assert_eq!(right_end(&over), 470.0);
    chart.title_overlay = true;
    assert!(top_end(&lay(&w, &chart)) < top_end(&over) - 20.0);
    // A legend entry removed.
    chart.legend_hidden = vec![1];
    let prims = lay(&w, &chart);
    assert!(has_text(&prims, "North") && !has_text(&prims, "South") && has_text(&prims, "West"));

    // Fills and borders of the chart and of the plot.
    chart.chart_area.fill = Fill::Solid(Color(0xFFF2CC));
    chart.chart_area.border.shown = Some(false);
    chart.plot_area.fill = Fill::Solid(Color(0xEEEEEE));
    chart.plot_area.border.color = Some(Color(0x333333));
    let prims = lay(&w, &chart);
    assert!(matches!(&prims[0], Prim::Rect { fill: Some(ink), stroke: None, .. } if ink.color == Color(0xFFF2CC)));
    assert!(prims.iter().any(|p| matches!(p, Prim::Rect { fill: Some(ink), stroke: Some(s), .. }
        if ink.color == Color(0xEEEEEE) && s.ink.color == Color(0x333333))));
    chart.chart_area.fill = Fill::None;
    assert!(!matches!(&lay(&w, &chart)[0], Prim::Rect { w, .. } if *w == 480.0));
}

#[test]
fn radar() {
    let w = sales();
    for kind in [ChartKind::Radar, ChartKind::RadarMarkers, ChartKind::RadarFilled] {
        let prims = lay(&w, &chart_of(&w, "A1:C5", kind));
        assert_sane(&prims, SIZE, &format!("{kind:?}"));
        // Rings (0 excluded), spokes, values up the first spoke, categories around.
        assert_eq!(stroked(&prims, GRIDLINE.0).len(), 4);
        assert_eq!(lines(&prims, GRIDLINE).len(), 4);
        assert!(has_text(&prims, "20") && has_text(&prims, "Mar"));
        assert!(find_text(&prims, "Jan").y < find_text(&prims, "Mar").y);
        assert!(find_text(&prims, "Feb").x > find_text(&prims, "Apr").x);
        let closed = stroked(&prims, NORTH).iter().filter(|(pts, _)| pts.len() == 4).count();
        assert_eq!(closed, if kind == ChartKind::RadarFilled { 0 } else { 1 });
        let filled = filled_paths(&prims, NORTH);
        assert_eq!(filled, match kind {
            ChartKind::RadarMarkers => 4 + 1,
            ChartKind::RadarFilled => 1,
            _ => 0,
        });
    }
}

#[test]
fn three_d() {
    let w = sales();
    for kind in ChartKind::ALL.into_iter().filter(|k| k.is_3d()) {
        let range = if kind.is_round() { "A1:B5" } else { "A1:D5" };
        let mut chart = chart_of(&w, range, kind);
        let prims = lay(&w, &chart);
        assert_sane(&prims, SIZE, &format!("{kind:?}"));
        assert!(has_text(&prims, "Jan"), "{kind:?}");
        if kind.is_round() {
            continue;
        }
        // Front faces in the colour of the series, shaded sides and tops.
        assert!(filled_paths(&prims, NORTH) >= 1, "{kind:?}");
        if kind.family() != Family::Line {
            assert!(filled_paths(&prims, shade(Color(NORTH), -0.36).0) >= 1, "{kind:?}: sides");
            assert!(filled_paths(&prims, shade(Color(NORTH), -0.2).0) >= 1, "{kind:?}: tops");
        }
        // Another camera, perspective and from the left: still sane.
        chart.view3d = Some(View3D {
            rot_x: 40,
            rot_y: 300,
            perspective: 60,
            right_angle_axes: false,
            depth_percent: 250,
            height_percent: None,
        });
        assert_sane(&lay(&w, &chart), SIZE, &format!("{kind:?}, turned"));
        chart.view3d = Some(View3D {
            rot_x: -20,
            rot_y: 340,
            ..View3D::for_kind(kind)
        });
        assert_sane(&lay(&w, &chart), SIZE, &format!("{kind:?}, from below"));
    }
    // A 3-D column: twelve boxes of three visible faces each.
    let prims = lay(&w, &chart_of(&w, "A1:D5", ChartKind::Column3D));
    assert_eq!(filled_paths(&prims, NORTH), 4);
    assert_eq!(filled_paths(&prims, shade(Color(NORTH), -0.36).0), 4);
    // With right-angle axes the front of a box is a rectangle.
    let front = prims
        .iter()
        .find_map(|p| match p {
            Prim::Path { segs, fill: Some(ink), .. } if ink.color == Color(NORTH) => Some(segs.clone()),
            _ => None,
        })
        .unwrap();
    let (Seg::Move(x0, y0), Seg::Line(x1, y1), Seg::Line(x2, _)) = (front[0], front[1], front[2]) else { panic!() };
    assert!((y0 - y1).abs() < 0.01 && (x1 - x2).abs() < 0.01 && x1 > x0);
    // Excel's proportions: the floor runs back along (sin 20°, sin 15°),
    // two and a half times as deep as a bar is wide (a bar is as deep as
    // it is wide, with one and a half times that around it).
    let (floor, _) = stroked(&prims, AXIS.0).into_iter().find(|(pts, _)| pts.len() == 4).expect("the floor");
    let (dx, dy) = (floor[2].0 - floor[1].0, floor[1].1 - floor[2].1);
    assert!((dy / dx - 15f32.to_radians().sin() / 20f32.to_radians().sin()).abs() < 0.01, "{floor:?}");
    let depth = dx / 20f32.to_radians().sin();
    assert!((depth / (x1 - x0) - 2.5).abs() < 0.01, "{depth} {}", x1 - x0);
    // Bars are listed from the top one down in the legend of a bar chart.
    let bars = lay(&w, &chart_of(&w, "A1:D5", ChartKind::Bar3D));
    assert!(find_text(&bars, "West").x < find_text(&bars, "North").x);
    let columns = lay(&w, &chart_of(&w, "A1:D5", ChartKind::Column));
    assert!(find_text(&columns, "West").x > find_text(&columns, "North").x);
    let stacked = lay(&w, &chart_of(&w, "A1:D5", ChartKind::StackedBar));
    assert!(find_text(&stacked, "West").x > find_text(&stacked, "North").x);

    // In perspective the picture keeps its proportions: the upright edges
    // of the box stay upright and the far end of the floor is narrower.
    let lines = lay(&w, &chart_of(&w, "A1:D5", ChartKind::Line3D));
    let (floor, _) = stroked(&lines, AXIS.0).into_iter().find(|(pts, _)| pts.len() == 4).expect("the floor");
    let near = ((floor[1].0 - floor[0].0).powi(2) + (floor[1].1 - floor[0].1).powi(2)).sqrt();
    let far = ((floor[2].0 - floor[3].0).powi(2) + (floor[2].1 - floor[3].1).powi(2)).sqrt();
    assert!(far < near * 0.95 && far > near * 0.6, "{near} {far}");
    // A short value axis has fewer gridlines: by 10 instead of by 5.
    assert!(has_text(&lines, "15"));
    let small = lay_at(&w, &chart_of(&w, "A1:D5", ChartKind::Line3D), (320.0, 200.0), 1.0);
    assert!(has_text(&small, "10") && !has_text(&small, "15") && !has_text(&small, "5"));
}

#[test]
fn every_kind_survives_awkward_data() {
    let cases: Vec<Workbook> = vec![
        // Negative values, blanks, text among the numbers.
        wb(&[("A1", "a"), ("B1", "-5"), ("C1", "2"), ("A2", "b"), ("B2", "x"), ("C2", "-3"), ("A3", "c"), ("B3", "7"), ("A4", "d"), ("B4", "0"), ("C4", "4")]),
        // A single point.
        wb(&[("A1", "only"), ("B1", "3")]),
        // Nothing but zeros.
        wb(&[("A1", "a"), ("B1", "0"), ("A2", "b"), ("B2", "0")]),
        // Huge and tiny numbers side by side.
        wb(&[("A1", "a"), ("B1", "1E+15"), ("C1", "0.000001"), ("A2", "b"), ("B2", "-1E+15"), ("C2", "1")]),
    ];
    for (i, w) in cases.iter().enumerate() {
        for kind in ChartKind::ALL {
            let mut chart = chart_of(w, "A1:C4", kind);
            chart.data_labels = true;
            chart.legend = LegendPos::Right;
            for s in &mut chart.series {
                s.trendlines.push(Trendline::new(TrendKind::Exponential));
                s.trendlines.push(Trendline::new(TrendKind::MovingAverage(2)));
            }
            for size in [SIZE, (60.0, 40.0), (1200.0, 90.0)] {
                assert_sane(&lay_at(w, &chart, size, 1.0), size, &format!("case {i}, {kind:?}, {size:?}"));
            }
            chart.axes.value.log_base = Some(10.0);
            chart.axes.value.reversed = true;
            chart.axes.category.reversed = true;
            assert_sane(&lay(w, &chart), SIZE, &format!("case {i}, {kind:?}, log"));
        }
    }
    // A chart whose ranges are gone draws its frame and nothing else.
    let w = sales();
    let mut chart = chart_of(&w, "A1:D5", ChartKind::Column);
    chart.series.clear();
    assert_eq!(lay(&w, &chart).len(), 2);
}

#[test]
fn clipping() {
    let rect = (0.0, 0.0, 10.0, 10.0);
    // A triangle poking out to the right loses its tip.
    let cut = clip_polygon(&[(5.0, 2.0), (15.0, 5.0), (5.0, 8.0)], rect);
    assert_eq!(cut.len(), 4);
    assert!(cut.iter().all(|(x, _)| *x <= 10.0));
    assert!(clip_polygon(&[(20.0, 2.0), (30.0, 5.0), (20.0, 8.0)], rect).is_empty());
    // A line leaving and coming back becomes two pieces.
    let pieces = clip_polyline(&[(1.0, 5.0), (5.0, 15.0), (9.0, 5.0)], rect);
    assert_eq!(pieces.len(), 2);
    assert_eq!(pieces[0], vec![(1.0, 5.0), (3.0, 10.0)]);
    assert_eq!(pieces[1], vec![(7.0, 10.0), (9.0, 5.0)]);
    assert_eq!(clip_polyline(&[(1.0, 1.0), (2.0, 2.0), (3.0, 1.0)], rect).len(), 1);
    assert!(clip_polyline(&[(11.0, 1.0), (12.0, 2.0)], rect).is_empty());
    // The curve through three points passes through them.
    let curve = smooth_points(&[(0.0, 0.0), (5.0, 5.0), (10.0, 0.0)]);
    assert_eq!(curve.len(), 25);
    assert_eq!((curve[0], curve[12], curve[24]), ((0.0, 0.0), (5.0, 5.0), (10.0, 0.0)));
    // A quarter circle in one Bézier ends where it should.
    let mut segs = Vec::new();
    arc_segs(&mut segs, 0.0, 0.0, 2.0, 2.0, 0.0, std::f32::consts::FRAC_PI_2);
    let [Seg::Cubic(.., x, y)] = segs[..] else { panic!("{segs:?}") };
    assert!(x.abs() < 1e-5 && (y - 2.0).abs() < 1e-5);
}

/// Set `OPENSHEET_CHART_SVG` to a directory to get every kind drawn as an
/// SVG file there: the primitives are all a renderer needs.
#[test]
fn gallery_as_svg() {
    let Some(dir) = std::env::var_os("OPENSHEET_CHART_SVG") else {
        return;
    };
    let w = sales();
    for kind in ChartKind::ALL {
        let range = if kind.family() == Family::Pie { "A1:B5" } else { "A1:D5" };
        let chart = chart_of(&w, range, kind);
        let svg = to_svg(&lay(&w, &chart), SIZE);
        std::fs::write(std::path::Path::new(&dir).join(format!("{kind:?}.svg")), svg).unwrap();
    }
}

fn to_svg(prims: &[Prim], size: (f32, f32)) -> String {
    use std::fmt::Write;
    let paint = |fill: &Option<Ink>, stroke: &Option<Stroke>| {
        let mut out = match fill {
            Some(ink) => format!("fill=\"#{:06X}\" fill-opacity=\"{}\"", ink.color.0, ink.opacity),
            None => "fill=\"none\"".to_string(),
        };
        if let Some(s) = stroke {
            write!(out, " stroke=\"#{:06X}\" stroke-width=\"{}\"", s.ink.color.0, s.width).unwrap();
            let dashes: Vec<String> = s.dash.pattern().iter().map(|d| (d * s.width).to_string()).collect();
            if !dashes.is_empty() {
                write!(out, " stroke-dasharray=\"{}\"", dashes.join(" ")).unwrap();
            }
        }
        out
    };
    let mut svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\">", size.0, size.1);
    for p in prims {
        match p {
            Prim::Rect { x, y, w, h, fill, stroke } => {
                write!(svg, "<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" {}/>", paint(fill, stroke)).unwrap()
            }
            Prim::Line { x0, y0, x1, y1, stroke } => {
                write!(svg, "<path d=\"M{x0} {y0}L{x1} {y1}\" {}/>", paint(&None, &Some(*stroke))).unwrap()
            }
            Prim::Path { segs, fill, stroke } => {
                let mut d = String::new();
                for s in segs {
                    match s {
                        Seg::Move(x, y) => write!(d, "M{x} {y}").unwrap(),
                        Seg::Line(x, y) => write!(d, "L{x} {y}").unwrap(),
                        Seg::Cubic(a, b, c, e, x, y) => write!(d, "C{a} {b} {c} {e} {x} {y}").unwrap(),
                        Seg::Close => d.push('Z'),
                    }
                }
                write!(svg, "<path d=\"{d}\" {}/>", paint(fill, stroke)).unwrap()
            }
            Prim::Text(t) => {
                let anchor = match t.anchor {
                    TextAnchor::Start => "start",
                    TextAnchor::Middle => "middle",
                    TextAnchor::End => "end",
                };
                let text = t.text.replace('&', "&amp;").replace('<', "&lt;");
                write!(
                    svg,
                    "<text x=\"{}\" y=\"{}\" font-family=\"Helvetica\" font-size=\"{}\" fill=\"#{:06X}\" text-anchor=\"{anchor}\" dominant-baseline=\"central\" transform=\"rotate({} {} {})\">{text}</text>",
                    t.x, t.y, t.size, t.color.0, t.rotation, t.x, t.y
                )
                .unwrap()
            }
        }
    }
    svg.push_str("</svg>");
    svg
}
