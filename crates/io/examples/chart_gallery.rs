//! Writes a workbook with a chart of every kind and of every option, to
//! look at in OpenSheet or in Excel:
//!
//! `cargo run -p opensheet_io --example chart_gallery -- out.xlsx [svg-dir]`
//!
//! With a second argument, every chart is also laid out by the engine and
//! written as an SVG file in that directory: the same primitives the
//! application paints, without the application.

use opensheet_engine::Color;
use opensheet_engine::chart::{
    Axis, AxisCrossing, BlanksAs, Chart, ChartKind, ChartTitle, Dash, DataLabels, DataTable, DisplayUnit, ErrorAmount,
    ErrorBars, ErrorDirection, Fill, LabelPosition, LegendPos, Line, Marker, MarkerShape, TickLabels, TickMark,
    TrendKind, Trendline, View3D,
};
use opensheet_engine::chart_layout::{
    Labels, LayoutOptions, Prim, Seg, TextAnchor, TextMeasure, layout_chart,
};
use opensheet_engine::locale::Locale;
use opensheet_engine::refs::Rect;
use opensheet_engine::workbook::Workbook;
use std::fmt::Write as _;

/// Months and three regions: the data of most charts.
const SALES: [[&str; 4]; 6] = [
    ["Month", "North", "South", "West"],
    ["Jan", "12", "8", "5"],
    ["Feb", "15", "11", "7"],
    ["Mar", "9", "14", "10"],
    ["Apr", "18", "10", "12"],
    ["May", "21", "13", "9"],
];

/// X, Y and a size, for scatter and bubble charts and for trendlines.
const POINTS: [[&str; 3]; 8] = [
    ["x", "y", "size"],
    ["1", "2.1", "4"],
    ["2", "3.9", "9"],
    ["3", "6.2", "2"],
    ["4", "7.8", "12"],
    ["5", "10.5", "6"],
    ["6", "11.6", "3"],
    ["7", "14.4", "8"],
];

/// Units sold and a price: two scales that need two axes; large numbers
/// with a sign for display units and crossings.
const MIXED: [[&str; 4]; 6] = [
    ["Quarter", "Units", "Price", "Result"],
    ["Q1", "1200", "2.5", "-150000"],
    ["Q2", "1850", "2.2", "240000"],
    ["Q3", "900", "3.1", "1250000"],
    ["Q4", "2400", "2.8", "-420000"],
    ["Q5", "3100", "3.4", "3600000"],
];

const WIDTH: f32 = 400.0;
const HEIGHT: f32 = 240.0;
/// Charts in a row of the gallery.
const PER_ROW: u32 = 3;

struct Gallery {
    wb: Workbook,
    /// Charts placed on each sheet so far.
    placed: Vec<u32>,
}

impl Gallery {
    /// A new sheet with the three tables in A1:D6, F1:H8 and J1:M6.
    fn sheet(&mut self, name: &str) -> usize {
        let index = self.placed.len();
        if index > 0 {
            self.wb.add_sheet(index);
        }
        assert!(self.wb.rename_sheet(index, name));
        let tables: [(u32, Vec<Vec<&str>>); 3] = [
            (0, SALES.iter().map(|r| r.to_vec()).collect()),
            (5, POINTS.iter().map(|r| r.to_vec()).collect()),
            (9, MIXED.iter().map(|r| r.to_vec()).collect()),
        ];
        for (col, table) in tables {
            for (r, row) in table.iter().enumerate() {
                for (c, text) in row.iter().enumerate() {
                    self.wb.set_cell_text(index, r as u32, col + c as u32, text).expect("plain values");
                }
            }
        }
        self.placed.push(0);
        index
    }

    /// A chart of a range of the sheet, titled, changed by `edit`, placed
    /// in the next free slot under the tables.
    fn add(&mut self, sheet: usize, title: &str, range: &str, kind: ChartKind, edit: impl FnOnce(&mut Chart)) {
        self.wb.recalc();
        let mut chart = self.wb.chart_from_range(sheet, Rect::parse(range).expect("a range"), kind);
        chart.title = ChartTitle::Text(title.to_string());
        edit(&mut chart);
        let slot = self.placed[sheet];
        chart.anchor = (9 + 13 * (slot / PER_ROW), 1 + 7 * (slot % PER_ROW));
        chart.offset = (0.0, 0.0);
        chart.size = (WIDTH, HEIGHT);
        self.placed[sheet] += 1;
        self.wb.add_chart(sheet, chart);
    }
}

fn kind_name(kind: ChartKind) -> String {
    // "PercentStackedColumn3D" → "Percent stacked column 3D".
    let mut out = String::new();
    for (i, ch) in format!("{kind:?}").chars().enumerate() {
        if ch.is_uppercase() && i > 0 && !out.ends_with('3') {
            out.push(' ');
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

fn range_for(kind: ChartKind) -> &'static str {
    match kind {
        ChartKind::Bubble => "F1:H8",
        k if k.is_xy() => "F1:G8",
        ChartKind::Pie | ChartKind::Pie3D => "A1:B6",
        _ => "A1:D6",
    }
}

fn build() -> Workbook {
    let mut g = Gallery {
        wb: Workbook::new(Locale::en()),
        placed: Vec::new(),
    };

    // Every kind, as inserted.
    let kinds = g.sheet("Kinds");
    for kind in ChartKind::ALL {
        g.add(kinds, &kind_name(kind), range_for(kind), kind, |_| {});
    }

    let axes = g.sheet("Axes");
    g.add(axes, "Axis titles", "A1:D6", ChartKind::Column, |c| {
        c.axes.category.title = Some("Month".into());
        c.axes.value.title = Some("Units sold".into());
    });
    g.add(axes, "Scale 5 to 25 by 2.5, minor gridlines", "A1:D6", ChartKind::Line, |c| {
        c.axes.value = Axis {
            min: Some(5.0),
            max: Some(25.0),
            major_unit: Some(2.5),
            minor_unit: Some(0.5),
            minor_gridlines: true,
            ..c.axes.value.clone()
        };
    });
    g.add(axes, "Logarithmic scale", "J1:K6", ChartKind::Column, |c| {
        c.axes.value.log_base = Some(10.0);
        c.axes.value.minor_gridlines = true;
    });
    g.add(axes, "Values in reverse order", "A1:D6", ChartKind::Column, |c| c.axes.value.reversed = true);
    g.add(axes, "Categories in reverse order", "A1:D6", ChartKind::Bar, |c| c.axes.category.reversed = true);
    g.add(axes, "Number format 0.0, tick marks", "A1:C6", ChartKind::Column, |c| {
        c.axes.value.number_format = Some("0.0".into());
        c.axes.value.major_ticks = TickMark::Outside;
        c.axes.value.minor_ticks = TickMark::Inside;
        c.axes.category.major_ticks = TickMark::Cross;
    });
    g.add(axes, "No value axis, gridlines on both", "A1:C6", ChartKind::Line, |c| {
        c.axes.value.visible = false;
        c.axes.category.major_gridlines = true;
    });
    g.add(axes, "No category axis, no gridlines", "A1:C6", ChartKind::Column, |c| {
        c.axes.category.visible = false;
        c.axes.value.major_gridlines = false;
    });
    g.add(axes, "Horizontal axis crosses at 10", "A1:C6", ChartKind::Column, |c| {
        c.axes.value.crossing = AxisCrossing::At(10.0);
    });
    g.add(axes, "Negative values, labels low", "J1:K6", ChartKind::Column, |c| {
        *c = rebuilt(c, &["M"]);
        c.axes.category.labels = TickLabels::Low;
    });
    g.add(axes, "Display units: thousands", "J1:K6", ChartKind::Column, |c| {
        *c = rebuilt(c, &["M"]);
        c.axes.value.display_unit = DisplayUnit::Thousands;
        c.axes.category.labels = TickLabels::Low;
    });
    g.add(axes, "Vertical axis crosses at maximum", "A1:C6", ChartKind::Line, |c| {
        c.axes.category.crossing = AxisCrossing::Max;
    });
    g.add(axes, "Scatter: both scales set, axes at 4 and 8", "F1:G8", ChartKind::ScatterLinesMarkers, |c| {
        c.axes.category = Axis {
            min: Some(0.0),
            max: Some(8.0),
            major_unit: Some(2.0),
            crossing: AxisCrossing::At(4.0),
            title: Some("x".into()),
            ..c.axes.category.clone()
        };
        c.axes.value = Axis {
            crossing: AxisCrossing::At(8.0),
            title: Some("y".into()),
            ..c.axes.value.clone()
        };
    });

    let series = g.sheet("Series");
    // A hole in the West column, for the charts that show how blanks are drawn.
    g.wb.set_cell_text(series, 3, 3, "").expect("an empty cell");
    g.add(series, "Colours per series", "A1:D6", ChartKind::Column, |c| {
        for (s, color) in c.series.iter_mut().zip([0x7030A0, 0xFFC000, 0x00B0F0]) {
            s.color = Some(Color(color));
        }
    });
    g.add(series, "One point in another colour", "A1:B6", ChartKind::Column, |c| {
        c.series[0].point_mut(3).color = Some(Color(0xC00000));
    });
    g.add(series, "Vary colours by point", "A1:B6", ChartKind::Column, |c| c.vary_colors = Some(true));
    g.add(series, "Line width and dash", "A1:D6", ChartKind::Line, |c| {
        c.series[0].line = Line {
            width: Some(4.5),
            ..Line::default()
        };
        c.series[1].line = Line {
            dash: Dash::Dash,
            width: Some(1.5),
            ..Line::default()
        };
        c.series[2].color = Some(Color(0x7F7F7F));
        c.series[2].line = Line {
            dash: Dash::RoundDot,
            ..Line::default()
        };
    });
    g.add(series, "Markers", "A1:D6", ChartKind::LineMarkers, |c| {
        c.series[0].marker = Some(Marker {
            size: 9,
            ..Marker::new(MarkerShape::Diamond)
        });
        c.series[1].marker = Some(Marker {
            size: 8,
            fill: Some(Color::WHITE),
            ..Marker::new(MarkerShape::Square)
        });
        c.series[2].marker = Some(Marker::new(MarkerShape::None));
    });
    g.add(series, "Marker shapes, no lines", "A1:D6", ChartKind::LineMarkers, |c| {
        for (s, shape) in c.series.iter_mut().zip([MarkerShape::Triangle, MarkerShape::X, MarkerShape::Plus]) {
            s.line.shown = Some(false);
            s.marker = Some(Marker {
                size: 8,
                ..Marker::new(shape)
            });
        }
    });
    g.add(series, "Smoothed lines", "A1:C6", ChartKind::Line, |c| {
        c.series[0].smooth = Some(true);
        c.series[1].smooth = Some(true);
    });
    g.add(series, "Gap width 50 %, overlap 30 %", "A1:D6", ChartKind::Column, |c| {
        c.gap_width = Some(50);
        c.overlap = Some(30);
    });
    g.add(series, "Gap width 300 %", "A1:C6", ChartKind::Bar, |c| c.gap_width = Some(300));
    g.add(series, "Exploded pie, first slice at 90°", "A1:B6", ChartKind::Pie, |c| {
        c.series[0].explosion = 10;
        c.first_slice_angle = 90;
    });
    g.add(series, "One slice pulled out, one recoloured", "A1:B6", ChartKind::Pie, |c| {
        c.series[0].point_mut(1).explosion = Some(25);
        c.series[0].point_mut(4).color = Some(Color(0x7F7F7F));
    });
    g.add(series, "Doughnut, hole 40 %", "A1:C6", ChartKind::Doughnut, |c| c.hole_size = 40);
    g.add(series, "Outlines", "A1:C6", ChartKind::Area, |c| {
        for s in &mut c.series {
            s.line = Line {
                color: Some(Color(0x404040)),
                width: Some(1.5),
                ..Line::default()
            };
        }
    });
    g.add(series, "Blanks shown as zero", "A1:D6", ChartKind::Line, |c| c.blanks = BlanksAs::Zero);
    g.add(series, "Blanks spanned", "A1:D6", ChartKind::LineMarkers, |c| c.blanks = BlanksAs::Span);
    g.add(series, "Bubbles at 50 %", "F1:H8", ChartKind::Bubble, |c| c.bubble_scale = 50);

    let labels = g.sheet("Labels");
    let label_cases: [(&str, ChartKind, LabelPosition); 12] = [
        ("Outside end", ChartKind::Column, LabelPosition::OutsideEnd),
        ("Inside end", ChartKind::Column, LabelPosition::InsideEnd),
        ("Inside base", ChartKind::Bar, LabelPosition::InsideBase),
        ("Centre", ChartKind::StackedColumn, LabelPosition::Center),
        ("Above", ChartKind::Line, LabelPosition::Above),
        ("Below", ChartKind::LineMarkers, LabelPosition::Below),
        ("Left", ChartKind::Scatter, LabelPosition::Left),
        ("Right", ChartKind::Scatter, LabelPosition::Right),
        ("Best fit", ChartKind::Pie, LabelPosition::BestFit),
        ("Pie, outside end", ChartKind::Pie, LabelPosition::OutsideEnd),
        ("Pie, inside end", ChartKind::Pie, LabelPosition::InsideEnd),
        ("Pie, centre", ChartKind::Pie3D, LabelPosition::Center),
    ];
    for (name, kind, position) in label_cases {
        let range = if kind.is_round() || kind.is_xy() { range_for(kind) } else { "A1:C6" };
        g.add(labels, &format!("Labels: {name}"), range, kind, |c| {
            c.data_labels = true;
            c.label_options.position = position;
        });
    }
    g.add(labels, "Category name and percentage", "A1:B6", ChartKind::Pie, |c| {
        c.data_labels = true;
        c.legend = LegendPos::None;
        c.label_options = DataLabels {
            value: false,
            category: true,
            percent: true,
            position: LabelPosition::OutsideEnd,
            ..DataLabels::default()
        };
    });
    g.add(labels, "Series name, value, own separator", "A1:C6", ChartKind::Bar, |c| {
        c.data_labels = true;
        c.label_options = DataLabels {
            series_name: true,
            separator: Some(": ".into()),
            ..DataLabels::default()
        };
    });
    g.add(labels, "Number format 0.00, legend key", "A1:C6", ChartKind::Column, |c| {
        c.data_labels = true;
        c.label_options = DataLabels {
            number_format: Some("0.00".into()),
            legend_key: true,
            ..DataLabels::default()
        };
    });
    g.add(labels, "Labels on one series only", "A1:D6", ChartKind::Line, |c| {
        c.series[1].labels = Some(DataLabels {
            position: LabelPosition::Above,
            ..DataLabels::default()
        });
    });
    g.add(labels, "All but one series", "A1:D6", ChartKind::Column, |c| {
        c.data_labels = true;
        c.series[2].labels = Some(DataLabels::none());
    });
    g.add(labels, "Doughnut with percentages", "A1:B6", ChartKind::Doughnut, |c| {
        c.data_labels = true;
        c.hole_size = 50;
        c.label_options = DataLabels {
            value: false,
            percent: true,
            ..DataLabels::default()
        };
    });
    g.add(labels, "Bubble sizes", "F1:H8", ChartKind::Bubble, |c| {
        c.data_labels = true;
        c.label_options = DataLabels {
            value: false,
            bubble_size: true,
            position: LabelPosition::Center,
            ..DataLabels::default()
        };
    });

    let combo = g.sheet("Combo");
    g.add(combo, "Clustered column and line", "A1:D6", ChartKind::Column, |c| {
        c.series[2].kind = Some(ChartKind::LineMarkers);
    });
    g.add(combo, "Line on the secondary axis", "J1:L6", ChartKind::Column, |c| {
        c.series[1].kind = Some(ChartKind::LineMarkers);
        c.series[1].secondary = true;
        c.axes.value.title = Some("Units".into());
        c.axes.secondary_value.title = Some("Price".into());
        c.axes.secondary_value.number_format = Some("0.00".into());
    });
    g.add(combo, "Stacked columns, area and line", "A1:D6", ChartKind::StackedColumn, |c| {
        c.series[1].kind = Some(ChartKind::Area);
        c.series[2].kind = Some(ChartKind::Line);
    });
    g.add(combo, "Columns on both axes", "J1:L6", ChartKind::Column, |c| {
        c.series[1].secondary = true;
        c.gap_width = Some(300);
    });
    g.add(combo, "Scatter: markers and smooth lines", "F1:H8", ChartKind::Scatter, |c| {
        c.series[1].kind = Some(ChartKind::ScatterSmooth);
    });
    g.add(combo, "Secondary axis shown at the top too", "J1:L6", ChartKind::Line, |c| {
        c.series[1].secondary = true;
        c.axes.secondary_category.visible = true;
    });

    let trend = g.sheet("Trend");
    let trend_cases: [(&str, TrendKind); 6] = [
        ("Linear", TrendKind::Linear),
        ("Exponential", TrendKind::Exponential),
        ("Logarithmic", TrendKind::Logarithmic),
        ("Polynomial of order 3", TrendKind::Polynomial(3)),
        ("Power", TrendKind::Power),
        ("Moving average of 3", TrendKind::MovingAverage(3)),
    ];
    for (name, kind) in trend_cases {
        g.add(trend, name, "F1:G8", ChartKind::Scatter, |c| {
            c.legend = LegendPos::Bottom;
            c.series[0].trendlines.push(Trendline {
                show_equation: !matches!(kind, TrendKind::MovingAverage(_)),
                show_r2: !matches!(kind, TrendKind::MovingAverage(_)),
                ..Trendline::new(kind)
            });
        });
    }
    g.add(trend, "Forecast 3 forward, 1 backward", "F1:G8", ChartKind::Scatter, |c| {
        c.series[0].trendlines.push(Trendline {
            forward: 3.0,
            backward: 1.0,
            show_equation: true,
            ..Trendline::new(TrendKind::Linear)
        });
    });
    g.add(trend, "Intercept set to 0, named, solid red", "F1:G8", ChartKind::Scatter, |c| {
        c.legend = LegendPos::Right;
        c.series[0].trendlines.push(Trendline {
            intercept: Some(0.0),
            name: Some("Through the origin".into()),
            show_equation: true,
            show_r2: true,
            line: Line {
                color: Some(Color(0xC00000)),
                width: Some(2.25),
                dash: Dash::Solid,
                shown: None,
            },
            ..Trendline::new(TrendKind::Linear)
        });
    });
    g.add(trend, "On columns, forecast 2 periods", "A1:B6", ChartKind::Column, |c| {
        c.series[0].trendlines.push(Trendline {
            forward: 2.0,
            show_equation: true,
            ..Trendline::new(TrendKind::Linear)
        });
    });
    g.add(trend, "Two trends on a line", "A1:B6", ChartKind::LineMarkers, |c| {
        c.legend = LegendPos::Bottom;
        c.series[0].trendlines.push(Trendline::new(TrendKind::Polynomial(2)));
        c.series[0].trendlines.push(Trendline::new(TrendKind::MovingAverage(2)));
    });
    let bars = |horizontal: bool, direction: ErrorDirection, amount: ErrorAmount| ErrorBars {
        horizontal,
        direction,
        amount,
        end_cap: true,
    };
    g.add(trend, "Error bars: fixed 2", "A1:B6", ChartKind::Column, |c| {
        c.series[0].error_bars.push(bars(false, ErrorDirection::Both, ErrorAmount::Fixed(2.0)));
    });
    g.add(trend, "Error bars: 10 %, plus only", "A1:C6", ChartKind::Line, |c| {
        c.series[0].error_bars.push(bars(false, ErrorDirection::Plus, ErrorAmount::Percent(10.0)));
    });
    g.add(trend, "Error bars: x and y, standard error", "F1:G8", ChartKind::Scatter, |c| {
        c.series[0].error_bars.push(bars(true, ErrorDirection::Both, ErrorAmount::Fixed(0.4)));
        c.series[0].error_bars.push(bars(false, ErrorDirection::Both, ErrorAmount::StdErr));
    });
    g.add(trend, "Error bars: one standard deviation", "A1:B6", ChartKind::LineMarkers, |c| {
        c.series[0].error_bars.push(ErrorBars {
            end_cap: false,
            ..bars(false, ErrorDirection::Both, ErrorAmount::StdDev(1.0))
        });
    });

    let depth = g.sheet("3-D");
    for kind in ChartKind::ALL.into_iter().filter(|k| k.is_3d()) {
        g.add(depth, &kind_name(kind), range_for(kind), kind, |_| {});
    }
    g.add(depth, "Rotation 40°, elevation 30°", "A1:D6", ChartKind::Column3D, |c| {
        c.view3d = Some(View3D {
            rot_x: 30,
            rot_y: 40,
            ..View3D::for_kind(c.kind)
        });
    });
    g.add(depth, "Perspective, depth 300 %", "A1:D6", ChartKind::Column3D, |c| {
        c.view3d = Some(View3D {
            right_angle_axes: false,
            perspective: 60,
            depth_percent: 300,
            ..View3D::for_kind(c.kind)
        });
    });
    g.add(depth, "Seen from the left", "A1:D6", ChartKind::StackedColumn3D, |c| {
        c.view3d = Some(View3D {
            rot_y: 340,
            ..View3D::for_kind(c.kind)
        });
    });
    g.add(depth, "Pie, elevation 60°, turned 120°", "A1:B6", ChartKind::Pie3D, |c| {
        c.view3d = Some(View3D {
            rot_x: 60,
            ..View3D::for_kind(c.kind)
        });
        c.first_slice_angle = 120;
        c.series[0].point_mut(0).explosion = Some(20);
    });
    g.add(depth, "3-D columns with labels and titles", "A1:C6", ChartKind::Column3D, |c| {
        c.data_labels = true;
        c.axes.value.title = Some("Units".into());
        c.axes.category.title = Some("Month".into());
    });

    let elements = g.sheet("Elements");
    g.add(elements, "Chart and plot area", "A1:D6", ChartKind::Column, |c| {
        c.chart_area.fill = Fill::Solid(Color(0xFFF2CC));
        c.chart_area.border = Line {
            color: Some(Color(0xBF8F00)),
            width: Some(2.25),
            ..Line::default()
        };
        c.plot_area.fill = Fill::Solid(Color(0xFFFFFF));
        c.plot_area.border = Line {
            color: Some(Color(0x7F7F7F)),
            ..Line::default()
        };
    });
    g.add(elements, "No fill, no border", "A1:D6", ChartKind::Line, |c| {
        c.chart_area.fill = Fill::None;
        c.chart_area.border.shown = Some(false);
    });
    g.add(elements, "Legend at top right", "A1:D6", ChartKind::Column, |c| c.legend = LegendPos::TopRight);
    g.add(elements, "Legend at left", "A1:D6", ChartKind::Area, |c| c.legend = LegendPos::Left);
    g.add(elements, "Legend at top", "A1:D6", ChartKind::Bar, |c| c.legend = LegendPos::Top);
    g.add(elements, "South removed from the legend", "A1:D6", ChartKind::Column, |c| {
        c.legend = LegendPos::Right;
        c.legend_hidden = vec![1];
    });
    g.add(elements, "Legend over the plot", "A1:D6", ChartKind::Line, |c| {
        c.legend = LegendPos::Right;
        c.legend_overlay = true;
    });
    g.add(elements, "Title over the plot", "A1:D6", ChartKind::Column, |c| c.title_overlay = true);
    g.add(elements, "", "A1:D6", ChartKind::Column, |c| c.title = ChartTitle::None);
    g.add(elements, "Data table", "A1:D6", ChartKind::Column, |c| {
        c.legend = LegendPos::None;
        c.data_table = Some(DataTable::default());
    });
    g.add(elements, "", "A1:B6", ChartKind::Column, |c| c.title = ChartTitle::Auto);
    g.add(elements, "", "A1:D6", ChartKind::Line, |c| c.title = ChartTitle::Auto);

    g.wb.recalc();
    g.wb
}

/// The chart of one value column of the third table (`J` holds its
/// categories): `chart_from_range` reads a single rectangle.
fn rebuilt(chart: &Chart, columns: &[&str]) -> Chart {
    let mut out = chart.clone();
    let template = out.series[0].clone();
    out.series = columns
        .iter()
        .map(|col| {
            let cell = |text: &str| {
                let rect = Rect::parse(text).expect("a range");
                let mut r = template.values.clone();
                r.start.row = rect.r0;
                r.start.col = rect.c0;
                r.end = (!rect.is_single()).then(|| {
                    let mut end = r.start;
                    end.row = rect.r1;
                    end.col = rect.c1;
                    end
                });
                r
            };
            let mut s = template.clone();
            s.name = opensheet_engine::chart::SeriesName::Cell(cell(&format!("{col}1")));
            s.values = cell(&format!("{col}2:{col}6"));
            s.categories = Some(cell("J2:J6"));
            s
        })
        .collect();
    out
}

/// Widths good enough to look at a layout without a font at hand.
struct Rough;

impl TextMeasure for Rough {
    fn width(&self, text: &str, size: f32, bold: bool) -> f32 {
        let per_char = if bold { 0.56 } else { 0.5 };
        text.chars().map(|ch| if ch.is_ascii_uppercase() || ch == 'm' { 0.68 } else { per_char }).sum::<f32>() * size
    }
}

fn svg(prims: &[Prim], size: (f32, f32)) -> String {
    let paint = |fill: &Option<opensheet_engine::chart_layout::Ink>, stroke: &Option<opensheet_engine::chart_layout::Stroke>| {
        let mut out = match fill {
            Some(ink) => format!("fill=\"#{:06X}\" fill-opacity=\"{}\"", ink.color.0, ink.opacity),
            None => "fill=\"none\"".to_string(),
        };
        if let Some(s) = stroke {
            write!(out, " stroke=\"#{:06X}\" stroke-width=\"{}\" stroke-linejoin=\"round\"", s.ink.color.0, s.width).unwrap();
            let dashes: Vec<String> = s.dash.pattern().iter().map(|d| (d * s.width).to_string()).collect();
            if !dashes.is_empty() {
                write!(out, " stroke-dasharray=\"{}\"", dashes.join(" ")).unwrap();
            }
        }
        out
    };
    let mut out = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\">", size.0, size.1);
    for prim in prims {
        match prim {
            Prim::Rect { x, y, w, h, fill, stroke } => {
                write!(out, "<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{h}\" {}/>", paint(fill, stroke)).unwrap()
            }
            Prim::Line { x0, y0, x1, y1, stroke } => {
                write!(out, "<path d=\"M{x0} {y0}L{x1} {y1}\" {}/>", paint(&None, &Some(*stroke))).unwrap()
            }
            Prim::Path { segs, fill, stroke } => {
                let mut d = String::new();
                for seg in segs {
                    match seg {
                        Seg::Move(x, y) => write!(d, "M{x} {y}").unwrap(),
                        Seg::Line(x, y) => write!(d, "L{x} {y}").unwrap(),
                        Seg::Cubic(a, b, c, e, x, y) => write!(d, "C{a} {b} {c} {e} {x} {y}").unwrap(),
                        Seg::Close => d.push('Z'),
                    }
                }
                write!(out, "<path d=\"{d}\" {}/>", paint(fill, stroke)).unwrap()
            }
            Prim::Text(t) => {
                let anchor = match t.anchor {
                    TextAnchor::Start => "start",
                    TextAnchor::Middle => "middle",
                    TextAnchor::End => "end",
                };
                let text = t.text.replace('&', "&amp;").replace('<', "&lt;");
                write!(
                    out,
                    "<text x=\"{}\" y=\"{}\" font-family=\"Helvetica\" font-size=\"{}\" font-weight=\"{}\" fill=\"#{:06X}\" text-anchor=\"{anchor}\" dominant-baseline=\"central\" transform=\"rotate({} {} {})\">{text}</text>",
                    t.x,
                    t.y,
                    t.size,
                    if t.bold { "bold" } else { "normal" },
                    t.color.0,
                    t.rotation,
                    t.x,
                    t.y
                )
                .unwrap()
            }
        }
    }
    out.push_str("</svg>");
    out
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().unwrap_or_else(|| "chart_gallery.xlsx".to_string());
    let wb = build();
    opensheet_io::save_xlsx(&wb, std::path::Path::new(&path)).expect("the workbook to be written");
    println!("{path}");
    if let Some(dir) = args.next() {
        let options = LayoutOptions {
            scale: 1.0,
            locale: wb.locale,
            labels: Labels::default(),
        };
        for (index, sheet) in wb.sheets.iter().enumerate() {
            for (n, chart) in sheet.charts.iter().enumerate() {
                let data = wb.chart_data(index, chart);
                let prims = layout_chart(chart, &data, chart.size, &options, &Rough);
                let file = std::path::Path::new(&dir).join(format!("{}-{:02}.svg", sheet.name, n + 1));
                std::fs::write(&file, svg(&prims, chart.size)).expect("the drawing to be written");
            }
        }
        println!("{dir}");
    }
}
