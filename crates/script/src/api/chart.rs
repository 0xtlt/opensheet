//! `Chart`, its title and its legend.

use serde_json::Value as J;

use opensheet_engine::chart::{Chart, ChartKind, ChartTitle, LegendPos};
use opensheet_engine::protect::Action;
use opensheet_engine::refs::{MAX_COLS, MAX_ROWS};
use opensheet_engine::workbook::Sheet;

use super::util::range_arg;
use super::{ApiClass, Call, Res, Ret, class, enum_value, object};
use crate::host::Host;

/// Pixels (the unit of the engine) per point.
const PX_PER_PT: f64 = 96.0 / 72.0;

/// The chart kinds of `ExcelScript.ChartType` that OpenSheet draws.
const KINDS: &[(&str, ChartKind)] = &[
    ("ColumnClustered", ChartKind::Column),
    ("ColumnStacked", ChartKind::StackedColumn),
    ("ColumnStacked100", ChartKind::PercentStackedColumn),
    ("3DColumnClustered", ChartKind::Column3D),
    ("3DColumnStacked", ChartKind::StackedColumn3D),
    ("3DColumnStacked100", ChartKind::PercentStackedColumn3D),
    ("Line", ChartKind::Line),
    ("LineStacked", ChartKind::StackedLine),
    ("LineStacked100", ChartKind::PercentStackedLine),
    ("LineMarkers", ChartKind::LineMarkers),
    ("LineMarkersStacked", ChartKind::StackedLineMarkers),
    ("LineMarkersStacked100", ChartKind::PercentStackedLineMarkers),
    ("3DLine", ChartKind::Line3D),
    ("Pie", ChartKind::Pie),
    ("3DPie", ChartKind::Pie3D),
    ("Doughnut", ChartKind::Doughnut),
    ("BarClustered", ChartKind::Bar),
    ("BarStacked", ChartKind::StackedBar),
    ("BarStacked100", ChartKind::PercentStackedBar),
    ("3DBarClustered", ChartKind::Bar3D),
    ("3DBarStacked", ChartKind::StackedBar3D),
    ("3DBarStacked100", ChartKind::PercentStackedBar3D),
    ("Area", ChartKind::Area),
    ("AreaStacked", ChartKind::StackedArea),
    ("AreaStacked100", ChartKind::PercentStackedArea),
    ("3DArea", ChartKind::Area3D),
    ("3DAreaStacked", ChartKind::StackedArea3D),
    ("3DAreaStacked100", ChartKind::PercentStackedArea3D),
    ("XYScatter", ChartKind::Scatter),
    ("XYScatterSmooth", ChartKind::ScatterSmoothMarkers),
    ("XYScatterSmoothNoMarkers", ChartKind::ScatterSmooth),
    ("XYScatterLines", ChartKind::ScatterLinesMarkers),
    ("XYScatterLinesNoMarkers", ChartKind::ScatterLines),
    ("Bubble", ChartKind::Bubble),
    ("Radar", ChartKind::Radar),
    ("RadarMarkers", ChartKind::RadarMarkers),
    ("RadarFilled", ChartKind::RadarFilled),
];

fn kind_arg(call: &Call, index: usize) -> Res<ChartKind> {
    let name = enum_value(call, index, "ChartType")?.ok_or_else(|| call.invalid())?;
    KINDS.iter().find(|(value, _)| *value == name).map(|(_, kind)| *kind).ok_or_else(|| call.invalid())
}

fn chart_object(h: &Host, sheet: usize, id: u32) -> J {
    object("Chart", vec![h.sheet_id(sheet).into(), id.into()])
}

pub(super) fn charts_of(h: &Host, sheet: usize) -> J {
    J::from(h.wb.sheets[sheet].charts.iter().map(|chart| chart_object(h, sheet, chart.id)).collect::<Vec<J>>())
}

pub(super) fn find(h: &Host, call: &Call, sheet: usize) -> Res<Ret> {
    let name = call.str(0)?;
    let found = h.wb.sheets[sheet].charts.iter().find(|chart| chart.name.eq_ignore_ascii_case(name));
    Ok(Ret::maybe(found.map(|chart| chart_object(h, sheet, chart.id))))
}

/// `Worksheet.addChart`: a chart of a range, placed as a new chart is.
pub(super) fn add(h: &mut Host, call: &Call, sheet: usize) -> Res<Ret> {
    let kind = kind_arg(call, 0)?;
    let (from, rect) = range_arg(h, call, 1, sheet)?;
    let by_rows = match enum_value(call, 2, "ChartSeriesBy")? {
        Some("Rows") => Some(true),
        Some("Columns") => Some(false),
        _ => None,
    };
    h.permit(call, &Action::EditObject { sheet, locked: true })?;
    h.calculate();
    let rect = if rect.is_single() { h.wb.current_region(from, rect.r0, rect.c0) } else { h.wb.clamp_to_used(from, rect) };
    let mut chart = h.wb.chart_from_block(from, rect, kind, by_rows);
    if chart.series.is_empty() {
        return Err(call.fail("The range holds nothing to chart."));
    }
    if from != sheet {
        // The series name their sheet when the chart is on another one.
        let name: std::sync::Arc<str> = h.wb.sheets[from].name.as_str().into();
        chart.map_refs(&mut |r| {
            let mut r = r.clone();
            r.sheet.get_or_insert_with(|| name.clone());
            Some(r)
        });
    }
    let id = h.wb.add_chart(sheet, chart).ok_or_else(|| call.gone())?;
    Ok(Ret::Object(chart_object(h, sheet, id)))
}

/// The sheet and the chart a Chart object (or its title, its legend)
/// stands for.
fn this_chart(h: &Host, call: &Call) -> Res<(usize, Chart)> {
    let sheet = h.sheet(call, call.this_int(0))?;
    let id = u32::try_from(call.this_int(1)).map_err(|_| call.gone())?;
    let chart = h.wb.chart(sheet, id).ok_or_else(|| call.gone())?;
    Ok((sheet, chart.clone()))
}

fn read(h: &mut Host, call: &Call, get: fn(&Chart) -> J) -> Res<Ret> {
    Ok(Ret::Value(get(&this_chart(h, call)?.1)))
}

/// Change a chart as the Chart Design and Format tabs do.
fn change(h: &mut Host, call: &Call, f: impl FnOnce(&mut Chart)) -> Res<Ret> {
    let (sheet, chart) = this_chart(h, call)?;
    h.permit(call, &Action::EditObject { sheet, locked: true })?;
    h.wb.update_chart(sheet, chart.id, f);
    Ok(Ret::Nothing)
}

/// Where a cell starts on the sheet, in pixels.
fn origin(sheet: &Sheet, row: u32, col: u32) -> (f64, f64) {
    let x: f64 = (0..col).map(|c| sheet.col_width(c) as f64).sum();
    let y: f64 = (0..row).map(|r| sheet.row_height(r) as f64).sum();
    (x, y)
}

/// The cell holding a distance from the edge of the sheet, and what is
/// left of the distance inside it.
fn cell_at(size: impl Fn(u32) -> f32, limit: u32, mut distance: f64) -> (u32, f32) {
    let mut index = 0;
    while index + 1 < limit {
        let step = size(index) as f64;
        if distance < step {
            break;
        }
        distance -= step;
        index += 1;
    }
    (index, distance.max(0.0) as f32)
}

/// Where a chart is, in pixels from the top left of its sheet.
fn corner(h: &Host, sheet: usize, chart: &Chart) -> (f64, f64) {
    let (x, y) = origin(&h.wb.sheets[sheet], chart.anchor.0, chart.anchor.1);
    (x + chart.offset.0 as f64, y + chart.offset.1 as f64)
}

/// Move a chart to a place in points; `None` keeps a coordinate.
fn move_to(h: &mut Host, call: &Call, left: Option<f64>, top: Option<f64>) -> Res<Ret> {
    let (sheet, chart) = this_chart(h, call)?;
    let (x, y) = corner(h, sheet, &chart);
    let x = left.map_or(x, |points| points * PX_PER_PT);
    let y = top.map_or(y, |points| points * PX_PER_PT);
    if !(x >= 0.0 && y >= 0.0) {
        return Err(call.invalid());
    }
    let sh = &h.wb.sheets[sheet];
    let (col, dx) = cell_at(|c| sh.col_width(c), MAX_COLS, x);
    let (row, dy) = cell_at(|r| sh.row_height(r), MAX_ROWS, y);
    change(h, call, |chart| {
        chart.anchor = (row, col);
        chart.offset = (dx, dy);
    })
}

fn size_arg(call: &Call) -> Res<f32> {
    let points = call.num(0)?;
    if !(points > 0.0 && points <= 10_000.0) {
        return Err(call.invalid());
    }
    Ok((points * PX_PER_PT) as f32)
}

pub(super) const CHART: ApiClass = class!("Chart", "A chart on a worksheet.", {
    getName "(): string" "The name of the chart (Chart 1)." => |h, c| read(h, c, |chart| chart.name.as_str().into());
    setName "(name: string): void" "Renames the chart." => |h, c| {
        let (sheet, chart) = this_chart(h, c)?;
        let name = c.str(0)?.trim().to_string();
        let taken = h.wb.sheets[sheet].charts.iter().any(|other| other.id != chart.id && other.name.eq_ignore_ascii_case(&name));
        if name.is_empty() || taken {
            return Err(c.fail("A resource with the same name or identifier already exists."));
        }
        change(h, c, |chart| chart.name = name)
    };
    getId "(): string" "An identifier that stays the same when the chart is renamed." => |h, c| read(h, c, |chart| chart.id.to_string().into());
    getWorksheet "(): Worksheet" "The worksheet the chart is on." => |h, c| {
        let (sheet, _) = this_chart(h, c)?;
        Ok(Ret::Object(h.worksheet(sheet)))
    };
    getChartType "(): ExcelScript.ChartType" "The type of the chart: ColumnClustered, Line, Pie…" => |h, c| {
        read(h, c, |chart| KINDS.iter().find(|(_, kind)| *kind == chart.kind).map_or("Invalid", |(name, _)| *name).into())
    };
    setChartType "(chartType: ExcelScript.ChartType): void" "Changes the type of the chart; every series takes it." => |h, c| {
        let kind = kind_arg(c, 0)?;
        change(h, c, |chart| chart.set_kind(kind))
    };
    getTitle "(): ChartTitle" "The title of the chart." => |h, c| {
        this_chart(h, c)?;
        Ok(Ret::object("ChartTitle", c.this.to_vec()))
    };
    getLegend "(): ChartLegend" "The legend of the chart." => |h, c| {
        this_chart(h, c)?;
        Ok(Ret::object("ChartLegend", c.this.to_vec()))
    };
    setPosition "(startCell: Range | string, endCell?: Range | string): void" "Puts the top left corner of the chart on a cell and, with an end cell, sizes it to end with that cell." => |h, c| {
        let (sheet, chart) = this_chart(h, c)?;
        let (on, start) = range_arg(h, c, 0, sheet)?;
        let end = if c.has(1) { Some(range_arg(h, c, 1, sheet)?) } else { None };
        if on != sheet || end.is_some_and(|(other, _)| other != sheet) {
            return Err(c.invalid());
        }
        let end = end.map(|(_, cell)| cell).or((!start.is_single()).then_some(start));
        let size = match end {
            Some(end) => {
                let sh = &h.wb.sheets[sheet];
                let from = origin(sh, start.r0, start.c0);
                let to = origin(sh, end.r1 + 1, end.c1 + 1);
                if to.0 <= from.0 || to.1 <= from.1 {
                    return Err(c.invalid());
                }
                ((to.0 - from.0) as f32, (to.1 - from.1) as f32)
            }
            None => chart.size,
        };
        change(h, c, |chart| {
            chart.anchor = (start.r0, start.c0);
            chart.offset = (0.0, 0.0);
            chart.size = size;
        })
    };
    getLeft "(): number" "The distance from the left edge of the sheet to the chart, in points." => |h, c| {
        let (sheet, chart) = this_chart(h, c)?;
        Ok((corner(h, sheet, &chart).0 / PX_PER_PT).into())
    };
    setLeft "(left: number): void" "Moves the chart to a distance from the left edge of the sheet, in points." => |h, c| move_to(h, c, Some(c.num(0)?), None);
    getTop "(): number" "The distance from the top edge of the sheet to the chart, in points." => |h, c| {
        let (sheet, chart) = this_chart(h, c)?;
        Ok((corner(h, sheet, &chart).1 / PX_PER_PT).into())
    };
    setTop "(top: number): void" "Moves the chart to a distance from the top edge of the sheet, in points." => |h, c| move_to(h, c, None, Some(c.num(0)?));
    getWidth "(): number" "The width of the chart, in points." => |h, c| read(h, c, |chart| (chart.size.0 as f64 / PX_PER_PT).into());
    setWidth "(width: number): void" "Gives the chart a width, in points." => |h, c| {
        let width = size_arg(c)?;
        change(h, c, |chart| chart.size.0 = width)
    };
    getHeight "(): number" "The height of the chart, in points." => |h, c| read(h, c, |chart| (chart.size.1 as f64 / PX_PER_PT).into());
    setHeight "(height: number): void" "Gives the chart a height, in points." => |h, c| {
        let height = size_arg(c)?;
        change(h, c, |chart| chart.size.1 = height)
    };
    delete "(): void" "Deletes the chart." => |h, c| {
        let (sheet, chart) = this_chart(h, c)?;
        h.permit(c, &Action::EditObject { sheet, locked: true })?;
        h.wb.remove_chart(sheet, chart.id);
        Ok(Ret::Nothing)
    };
});

pub(super) const CHART_TITLE: ApiClass = class!("ChartTitle", "The title of a chart.", {
    getText "(): string" "The text of the title; the one Excel makes up (the name of the only series) when none was given." => |h, c| {
        let (sheet, chart) = this_chart(h, c)?;
        Ok(match &chart.title {
            ChartTitle::Text(text) => text.clone(),
            ChartTitle::None => String::new(),
            ChartTitle::Auto => {
                h.calculate();
                h.wb.chart_data(sheet, &chart).title.unwrap_or_default()
            }
        }.into())
    };
    setText "(text: string): void" "Gives the chart a title." => |h, c| {
        let text = c.str(0)?.to_string();
        change(h, c, |chart| chart.title = ChartTitle::Text(text))
    };
    getVisible "(): boolean" "Whether the chart shows a title." => |h, c| read(h, c, |chart| (chart.title != ChartTitle::None).into());
    setVisible "(visible: boolean): void" "Shows or hides the title." => |h, c| {
        let visible = c.bool(0)?;
        change(h, c, |chart| match (visible, &chart.title) {
            (false, _) => chart.title = ChartTitle::None,
            (true, ChartTitle::None) => chart.title = ChartTitle::Auto,
            (true, _) => {}
        })
    };
    getOverlay "(): boolean" "Whether the title is drawn over the plot." => |h, c| read(h, c, |chart| chart.title_overlay.into());
    setOverlay "(overlay: boolean): void" "Draws the title over the plot, or above it." => |h, c| {
        let overlay = c.bool(0)?;
        change(h, c, |chart| chart.title_overlay = overlay)
    };
});

pub(super) const CHART_LEGEND: ApiClass = class!("ChartLegend", "The legend of a chart.", {
    getVisible "(): boolean" "Whether the chart shows a legend." => |h, c| read(h, c, |chart| (chart.legend != LegendPos::None).into());
    setVisible "(visible: boolean): void" "Shows the legend (on the right when it had no place) or hides it." => |h, c| {
        let visible = c.bool(0)?;
        change(h, c, |chart| match (visible, chart.legend) {
            (false, _) => chart.legend = LegendPos::None,
            (true, LegendPos::None) => chart.legend = LegendPos::Right,
            (true, _) => {}
        })
    };
    getPosition "(): ExcelScript.ChartLegendPosition" "Where the legend is: Top, Bottom, Left, Right or Corner; Invalid when it is hidden." => |h, c| {
        read(h, c, |chart| match chart.legend {
            LegendPos::None => "Invalid",
            LegendPos::Bottom => "Bottom",
            LegendPos::Right => "Right",
            LegendPos::Top => "Top",
            LegendPos::Left => "Left",
            LegendPos::TopRight => "Corner",
        }.into())
    };
    setPosition "(position: ExcelScript.ChartLegendPosition): void" "Shows the legend on a side of the chart: Top, Bottom, Left, Right or Corner." => |h, c| {
        let legend = match enum_value(c, 0, "ChartLegendPosition")?.ok_or_else(|| c.invalid())? {
            "Top" => LegendPos::Top,
            "Bottom" => LegendPos::Bottom,
            "Left" => LegendPos::Left,
            "Right" => LegendPos::Right,
            "Corner" => LegendPos::TopRight,
            "Custom" => return Err(c.unsupported("ExcelScript.ChartLegendPosition.custom")),
            _ => return Err(c.invalid()),
        };
        change(h, c, |chart| {
            chart.legend = legend;
            chart.legend_layout = None;
        })
    };
    getOverlay "(): boolean" "Whether the legend is drawn over the plot." => |h, c| read(h, c, |chart| chart.legend_overlay.into());
    setOverlay "(overlay: boolean): void" "Draws the legend over the plot, or beside it." => |h, c| {
        let overlay = c.bool(0)?;
        change(h, c, |chart| chart.legend_overlay = overlay)
    };
});
