//! The Format tab and the Format pane of charts driven in GPUI's headless
//! window: every element gets its pane, fields store what is typed, the
//! commands of the tab act on the selected element. How the pane looks is
//! not what these tests see.

use super::pane::FormatPane;
use super::*;
use crate::features::charts::elements::{element_name, listed};
use crate::i18n::{Lang, set_lang};
use gpui::{Modifiers, TestAppContext, VisualTestContext, point};
use opensheet_engine::Rect;
use opensheet_engine::chart::{
    Chart, ChartKind, Dash, DataTable, ErrorAmount, ErrorBars, ErrorDirection, Line, ManualLayout,
    TextFormat, TrendKind, Trendline,
};
use opensheet_engine::chart_layout::AxisId;

fn open(cx: &mut TestAppContext) -> (Entity<ExcelApp>, &mut VisualTestContext) {
    set_lang(Lang::En);
    cx.update(crate::bind_keys);
    let (app, cx) = cx.add_window_view(ExcelApp::new);
    redraw(cx);
    (app, cx)
}

fn redraw(cx: &mut VisualTestContext) {
    cx.run_until_parked();
    cx.simulate_mouse_move(point(px(2.), px(2.)), None, Modifiers::none());
    cx.run_until_parked();
}

/// A chart of a small table, selected.
fn with_chart(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, kind: ChartKind) -> u32 {
    let id = app.update_in(cx, |app, window, cx| {
        let rows = [
            ["Month", "North", "South", "West"],
            ["Jan", "12", "8", "5"],
            ["Feb", "15", "11", "7"],
            ["Mar", "9", "14", "10"],
        ];
        for (r, row) in rows.iter().enumerate() {
            for (c, text) in row.iter().enumerate() {
                let _ = app.wb.set_cell_text(0, r as u32, c as u32, text);
            }
        }
        app.commit("setup", cx);
        app.select_rect(Rect::new(0, 0, 3, 3), cx);
        app.insert_chart(kind, window, cx);
        app.selected_chart().expect("a chart was inserted").id
    });
    redraw(cx);
    id
}

fn chart(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Chart {
    app.update(cx, |app, _| {
        app.selected_chart().cloned().expect("a selected chart")
    })
}

fn select(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, element: ChartElement) {
    app.update_in(cx, |app, window, cx| {
        app.select_chart_element(element, cx);
        open_pane(app, window, cx);
    });
    redraw(cx);
}

fn pane(app: &Entity<ExcelApp>, cx: &mut VisualTestContext) -> Entity<FormatPane> {
    app.update(cx, |app, _| {
        let entry = app
            .panes
            .iter()
            .find(|p| p.id == PANE)
            .expect("the pane is open");
        entry
            .view
            .clone()
            .downcast::<FormatPane>()
            .expect("the Format pane")
    })
}

/// Type into a field of the pane and leave it with Return.
fn type_in(app: &Entity<ExcelApp>, cx: &mut VisualTestContext, field: Field, text: &str) {
    let pane = pane(app, cx);
    pane.update(cx, |pane, cx| {
        let input = pane.field_input(field).expect("the field is shown");
        input.update(cx, |input, cx| input.set_text(text, cx));
        pane.commit_field(field, cx);
    });
    redraw(cx);
}

#[gpui::test]
fn every_element_of_every_family_has_its_pane(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    for kind in [
        ChartKind::Column,
        ChartKind::StackedBar,
        ChartKind::LineMarkers,
        ChartKind::Area,
        ChartKind::Pie,
        ChartKind::Doughnut,
        ChartKind::Scatter,
        ChartKind::Bubble,
        ChartKind::RadarMarkers,
        ChartKind::Column3D,
        ChartKind::Pie3D,
    ] {
        with_chart(&app, cx, kind);
        // Everything a chart can show, where the kind allows it.
        app.update(cx, |app, cx| {
            app.edit_chart("setup", cx, |c| {
                c.data_labels = true;
                c.axes.category.title = Some("Months".into());
                c.axes.value.title = Some("Units".into());
                c.axes.value.minor_gridlines = true;
                c.data_table = Some(DataTable::default());
                if let Some(last) = c.series.last_mut() {
                    last.secondary = true;
                }
                c.series[0]
                    .trendlines
                    .push(Trendline::new(TrendKind::Polynomial(2)));
                c.series[0].trendlines.push(Trendline {
                    name: Some("Mine".into()),
                    intercept: Some(1.0),
                    ..Trendline::new(TrendKind::Linear)
                });
                for (i, amount) in [
                    ErrorAmount::Fixed(1.0),
                    ErrorAmount::Percent(5.0),
                    ErrorAmount::StdDev(1.0),
                    ErrorAmount::StdErr,
                ]
                .into_iter()
                .enumerate()
                {
                    c.series[0].error_bars.push(ErrorBars {
                        horizontal: i == 1,
                        direction: ErrorDirection::Both,
                        amount,
                        end_cap: true,
                    });
                }
            });
        });
        redraw(cx);
        let mut all = app.update(cx, |app, _| {
            let chart = app.selected_chart().unwrap();
            listed(chart, &app.wb.chart_data(app.sheet, chart))
        });
        all.extend([ChartElement::Point(0, 1), ChartElement::LegendEntry(0)]);
        assert!(all.len() >= 5, "{kind:?}: {all:?}");
        for element in all {
            select(&app, cx, element);
            let title = app.update(cx, |app, _| {
                app.panes
                    .iter()
                    .find(|p| p.id == PANE)
                    .map(|p| p.title.to_string())
            });
            assert_eq!(
                title.as_deref(),
                Some(elements::pane_title(element)),
                "{kind:?} {element:?}"
            );
            // Every tab of the pane draws.
            let pane = pane(&app, cx);
            for tab in 0..3 {
                pane.update(cx, |pane, cx| pane.show_tab(tab, cx));
                redraw(cx);
            }
            pane.update(cx, |pane, cx| pane.show_tab(0, cx));
        }
        app.update(cx, |app, cx| app.delete_selected_object(cx));
        redraw(cx);
    }
}

#[gpui::test]
fn fields_store_what_is_typed_as_one_undo_step(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    with_chart(&app, cx, ChartKind::Column);
    select(&app, cx, ChartElement::Series(0));
    pane(&app, cx).update(cx, |pane, cx| pane.show_tab(1, cx));
    redraw(cx);
    // The field shows what Excel does for the kind until it is changed.
    let shown = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext, field: Field| {
        let pane = pane(app, cx);
        pane.update(cx, |pane, cx| {
            pane.field_input(field)
                .map(|input| input.read(cx).text().to_string())
        })
    };
    assert_eq!(shown(&app, cx, Field::GapWidth).as_deref(), Some("219 %"));
    type_in(&app, cx, Field::GapWidth, "80");
    assert_eq!(chart(&app, cx).gap_width, Some(80));
    assert_eq!(shown(&app, cx, Field::GapWidth).as_deref(), Some("80 %"));
    // Leaving the field without a change is not an undo step, and what
    // cannot be read leaves the chart alone: one undo takes it all back.
    type_in(&app, cx, Field::GapWidth, "80 %");
    type_in(&app, cx, Field::GapWidth, "wide");
    assert_eq!(chart(&app, cx).gap_width, Some(80));
    assert_eq!(shown(&app, cx, Field::GapWidth).as_deref(), Some("80 %"));
    cx.simulate_keystrokes("cmd-z");
    redraw(cx);
    assert_eq!(chart(&app, cx).gap_width, None);
    type_in(&app, cx, Field::GapWidth, "80");
    // A step of the spinner applies at once, unit or not in the field.
    pane(&app, cx).update(cx, |pane, cx| pane.step_field(Field::GapWidth, 5.0, cx));
    redraw(cx);
    assert_eq!(chart(&app, cx).gap_width, Some(85));
    assert_eq!(shown(&app, cx, Field::GapWidth).as_deref(), Some("85 %"));
    cx.simulate_keystrokes("cmd-z");
    redraw(cx);
    assert_eq!(chart(&app, cx).gap_width, Some(80));

    // An axis: the bounds show the automatic scale, typing sets them,
    // emptying the field hands them back.
    select(&app, cx, ChartElement::Axis(AxisId::Value));
    assert_eq!(shown(&app, cx, Field::AxisMax).as_deref(), Some("16"));
    assert_eq!(shown(&app, cx, Field::AxisMajor).as_deref(), Some("2"));
    type_in(&app, cx, Field::AxisMax, "40");
    assert_eq!(chart(&app, cx).axes.value.max, Some(40.0));
    type_in(&app, cx, Field::AxisMax, "");
    assert_eq!(chart(&app, cx).axes.value.max, None);
    assert_eq!(shown(&app, cx, Field::AxisMax).as_deref(), Some("16"));
    type_in(&app, cx, Field::NumberFormat, "0.0");
    assert_eq!(
        chart(&app, cx).axes.value.number_format.as_deref(),
        Some("0.0")
    );
}

#[gpui::test]
fn the_pane_waits_for_a_chart(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    with_chart(&app, cx, ChartKind::Line);
    select(&app, cx, ChartElement::Legend);
    app.update(cx, |app, cx| app.deselect_object(cx));
    redraw(cx);
    // Still open, with nothing to format; a chart selected again fills it.
    assert!(app.update(cx, |app, _| app.pane_open(PANE)));
    app.update(cx, |app, cx| app.delete_selected_object(cx));
    redraw(cx);
}

#[gpui::test]
fn the_format_tab_acts_on_the_selected_element(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    with_chart(&app, cx, ChartKind::Column);
    // The Chart Elements box lists the elements by name and selects them.
    let entries = app.update(cx, |app, _| element_menu(app));
    let labels: Vec<String> = entries.iter().map(|e| e.label.to_string()).collect();
    assert_eq!(labels[0], "Chart Area");
    assert!(
        labels.contains(&"Vertical (Value) Axis Major Gridlines".to_string()),
        "{labels:?}"
    );
    assert!(
        labels.contains(&"Series \"South\"".to_string()),
        "{labels:?}"
    );
    assert!(entries[0].checked && entries.iter().filter(|e| e.checked).count() == 1);
    let at = labels.iter().position(|l| l == "Series \"South\"").unwrap();
    let action = entries[at].action.clone().unwrap();
    app.update_in(cx, |app, window, cx| action(app, window, cx));
    assert_eq!(
        app.update(cx, |app, _| app.chart_element()),
        ChartElement::Series(1)
    );
    let name = app.update(cx, |app, _| {
        let chart = app.selected_chart().unwrap();
        element_name(
            chart,
            &app.wb.chart_data(app.sheet, chart),
            app.chart_element(),
        )
    });
    assert_eq!(name, "Series \"South\"");

    // Shape Fill and Shape Outline colour the series; Reset undoes both.
    let enabled = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext, cmd: Cmd| {
        app.update(cx, |app, _| cmd.enabled(app))
    };
    assert!(
        enabled(&app, cx, Cmd::ShapeFill)
            && enabled(&app, cx, Cmd::ShapeOutline)
            && !enabled(&app, cx, Cmd::ShapeEffects)
    );
    app.update_in(cx, |app, window, cx| {
        app.run_command(Cmd::ShapeFill, window, cx)
    });
    redraw(cx);
    assert!(app.update(cx, |app, _| app.popup.is_some()));
    app.update_in(cx, |app, window, cx| {
        app.close_popup(window, cx);
        apply_shape_color(app, Paints::Fill, Some(Color(0xC00000)), cx);
        apply_shape_color(app, Paints::Outline, Some(Color(0x000000)), cx);
    });
    let series = chart(&app, cx).series[1].clone();
    assert_eq!(series.color, Some(Color(0xC00000)));
    assert_eq!(
        (series.line.shown, series.line.color),
        (Some(true), Some(Color(0x000000)))
    );
    app.update_in(cx, |app, window, cx| {
        app.run_command(Cmd::ResetStyle, window, cx)
    });
    let series = chart(&app, cx).series[1].clone();
    assert_eq!((series.color, series.line), (None, Line::default()));
    // A legend has neither fill nor outline in the model: both are greyed.
    app.update(cx, |app, cx| {
        app.select_chart_element(ChartElement::Legend, cx)
    });
    assert!(!enabled(&app, cx, Cmd::ShapeFill) && !enabled(&app, cx, Cmd::ShapeOutline));
    // "No Fill" on the plot area, a colour on the chart area.
    app.update(cx, |app, cx| {
        app.select_chart_element(ChartElement::PlotArea, cx);
        apply_shape_color(app, Paints::Fill, None, cx);
        app.select_chart_element(ChartElement::ChartArea, cx);
        apply_shape_color(app, Paints::Fill, Some(Color(0xF2F2F2)), cx);
    });
    let c = chart(&app, cx);
    assert_eq!(
        (c.plot_area.fill, c.chart_area.fill),
        (Fill::None, Fill::Solid(Color(0xF2F2F2)))
    );

    // Size: the fields of the tab show the chart's size and set it.
    app.update(cx, |app, _| app.context_tab = Some(FORMAT_TAB));
    redraw(cx);
    let (height, width) = app.update_in(cx, |app, window, cx| size_fields(app, window, cx));
    assert_eq!(
        height.read_with(cx, |input, _| input.text().to_string()),
        "3\""
    );
    assert_eq!(
        width.read_with(cx, |input, _| input.text().to_string()),
        "5\""
    );
    width.update(cx, |input, cx| input.set_text("6", cx));
    app.update_in(cx, |app, window, cx| {
        on_size_event(
            app,
            Field::Width,
            &width,
            &InputEvent::Enter { shift: false },
            window,
            cx,
        )
    });
    assert_eq!(chart(&app, cx).size, (576.0, 288.0));
    assert_eq!(
        app.update(cx, |app, _| app.wb.undo_label().map(str::to_string))
            .as_deref(),
        Some("Resize Chart")
    );
}

#[gpui::test]
fn a_field_never_writes_to_another_element_nor_for_nothing(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    with_chart(&app, cx, ChartKind::Line);
    select(&app, cx, ChartElement::ChartArea);
    let field_text = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| {
        pane(app, cx).update(cx, |pane, cx| {
            pane.field_input(Field::LineWidth)
                .map(|i| i.read(cx).text().to_string())
        })
    };
    // A width typed for the border of the chart, then a click on a series
    // before the field is left: the series does not get it.
    pane(&app, cx).update(cx, |pane, cx| {
        let input = pane
            .field_input(Field::LineWidth)
            .expect("the width of the border");
        input.update(cx, |input, cx| input.set_text("3", cx));
    });
    app.update(cx, |app, cx| {
        app.select_chart_element(ChartElement::Series(0), cx)
    });
    redraw(cx);
    pane(&app, cx).update(cx, |pane, cx| pane.commit_field(Field::LineWidth, cx));
    redraw(cx);
    assert_eq!(chart(&app, cx).series[0].line.width, None);
    // The field now reads the automatic width of the line; leaving it as
    // it reads is not a change.
    assert_eq!(field_text(&app, cx).as_deref(), Some("2.25 pt"));
    pane(&app, cx).update(cx, |pane, cx| pane.commit_field(Field::LineWidth, cx));
    assert_eq!(chart(&app, cx).series[0].line.width, None);
    assert_eq!(
        app.update(cx, |app, _| app.wb.undo_label().map(str::to_string))
            .as_deref(),
        Some("Insert Chart")
    );
    type_in(&app, cx, Field::LineWidth, "1.5");
    let c = chart(&app, cx);
    assert_eq!(
        (c.chart_area.border.width, c.series[0].line.width),
        (None, Some(1.5))
    );
}

#[gpui::test]
fn text_options_set_the_font_of_the_selected_element(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    with_chart(&app, cx, ChartKind::Column);
    select(&app, cx, ChartElement::Title);
    // "Text Options" is the last page of every pane of an element with text.
    let show_text = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| {
        pane(app, cx).update(cx, |pane, cx| pane.show_tab(9, cx));
        redraw(cx);
    };
    show_text(&app, cx);
    let reads = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext, field: Field| {
        pane(app, cx).update(cx, |pane, cx| {
            pane.field_input(field)
                .map(|i| i.read(cx).text().to_string())
        })
    };
    // The size Excel gives a title, and the font of the chart.
    assert_eq!(reads(&app, cx, Field::FontSize).as_deref(), Some("14 pt"));
    assert_eq!(reads(&app, cx, Field::FontName).as_deref(), Some(""));
    type_in(&app, cx, Field::FontSize, "20");
    type_in(&app, cx, Field::FontName, "Georgia");
    let text = chart(&app, cx).title_text;
    assert_eq!(
        (text.size, text.font.as_deref()),
        (Some(20.0), Some("Georgia"))
    );
    assert_eq!(
        app.update(cx, |app, _| app.wb.undo_label().map(str::to_string))
            .as_deref(),
        Some("Format Chart")
    );
    // The arrows beside the size step it; an emptied font is the chart's again.
    pane(&app, cx).update(cx, |pane, cx| pane.step_field(Field::FontSize, 1.0, cx));
    type_in(&app, cx, Field::FontName, "");
    let text = chart(&app, cx).title_text;
    assert_eq!((text.size, text.font), (Some(21.0), None));
    // A field left as it reads is no change.
    app.update(cx, |app, cx| {
        app.edit_chart("mark", cx, |c| c.title_overlay = true)
    });
    pane(&app, cx).update(cx, |pane, cx| pane.commit_field(Field::FontSize, cx));
    assert_eq!(
        app.update(cx, |app, _| app.wb.undo_label().map(str::to_string))
            .as_deref(),
        Some("mark")
    );

    // The labels of an axis and data labels have the page too; the
    // labels of one series become its own.
    app.update(cx, |app, cx| {
        app.edit_chart("labels", cx, |c| c.data_labels = true)
    });
    for element in [
        ChartElement::Axis(AxisId::Value),
        ChartElement::DataLabels(1),
        ChartElement::Legend,
        ChartElement::ChartArea,
    ] {
        select(&app, cx, element);
        show_text(&app, cx);
        assert!(reads(&app, cx, Field::FontSize).is_some(), "{element:?}");
        type_in(&app, cx, Field::FontSize, "8");
    }
    let c = chart(&app, cx);
    assert_eq!(c.axes.value.text.size, Some(8.0));
    assert_eq!(
        c.series[1].labels.as_ref().map(|l| l.text.size),
        Some(Some(8.0))
    );
    // The chart area set them all, the title as well.
    assert_eq!(
        (
            c.title_text.size,
            c.axes.category.text.size,
            c.label_options.text.size
        ),
        (Some(8.0), Some(8.0), Some(8.0))
    );
    // A series has no text: no such page, and Text Fill of the Format
    // tab, which colours the text of the selection, is greyed.
    select(&app, cx, ChartElement::Series(0));
    assert!(app.update(cx, |app, _| {
        text_of(app.selected_chart().unwrap(), app.chart_element()).is_none()
    }));
    let enabled = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext| {
        app.update(cx, |app, _| Cmd::TextFill.enabled(app))
    };
    assert!(!enabled(&app, cx));
    app.update(cx, |app, cx| {
        app.select_chart_element(ChartElement::Legend, cx)
    });
    assert!(enabled(&app, cx));
    app.update_in(cx, |app, window, cx| {
        app.run_command(Cmd::TextFill, window, cx)
    });
    redraw(cx);
    assert!(app.update(cx, |app, _| app.popup.is_some()));
    app.update_in(cx, |app, window, cx| {
        app.close_popup(window, cx);
        apply_shape_color(app, Paints::Text, Some(Color(0x0070C0)), cx);
    });
    assert_eq!(chart(&app, cx).legend_text.color, Some(Color(0x0070C0)));
    assert_eq!(
        app.update(cx, |app, _| app.wb.undo_label().map(str::to_string))
            .as_deref(),
        Some("Text Fill")
    );
}

#[gpui::test]
fn gridlines_and_axis_lines_are_formatted_and_reset(cx: &mut TestAppContext) {
    let (app, cx) = open(cx);
    with_chart(&app, cx, ChartKind::Column);
    let grid = ChartElement::MajorGridlines(AxisId::Value);
    select(&app, cx, grid);
    // "Format Major Gridlines" opens on the Line section.
    let width = pane(&app, cx).update(cx, |pane, cx| {
        pane.field_input(Field::LineWidth)
            .map(|i| i.read(cx).text().to_string())
    });
    assert_eq!(width.as_deref(), Some("0.75 pt"));
    type_in(&app, cx, Field::LineWidth, "2");
    // Shape Outline of the Format tab gives them a colour.
    assert!(app.update(cx, |app, _| Cmd::ShapeOutline.enabled(app)
        && !Cmd::ShapeFill.enabled(app)));
    app.update(cx, |app, cx| {
        apply_shape_color(app, Paints::Outline, Some(Color(0xFF0000)), cx);
        app.edit_chart("Line", cx, |c| {
            if let Some(line) = line_of_mut(c, grid) {
                line.dash = Dash::Dash;
            }
        });
    });
    let expected = Line {
        shown: Some(true),
        color: Some(Color(0xFF0000)),
        width: Some(2.0),
        dash: Dash::Dash,
    };
    assert_eq!(chart(&app, cx).axes.value.major_gridline, expected);
    // The line of the axis itself, on the second page of its pane.
    select(&app, cx, ChartElement::Axis(AxisId::Category));
    pane(&app, cx).update(cx, |pane, cx| pane.show_tab(1, cx));
    redraw(cx);
    type_in(&app, cx, Field::LineWidth, "1.5");
    assert_eq!(chart(&app, cx).axes.category.line.width, Some(1.5));

    // "Reset to Match Style" gives each element its own look back, and
    // the place the chart gives it.
    app.update(cx, |app, cx| {
        app.edit_chart("setup", cx, |c| {
            c.title_layout = Some(ManualLayout::at(0.1, 0.1));
            c.title_text.bold = Some(true);
            c.legend_layout = Some(ManualLayout::at(0.5, 0.5));
            c.plot_layout = Some(ManualLayout {
                x: 0.2,
                y: 0.2,
                w: Some(0.5),
                h: Some(0.5),
            });
            c.axes.value.text.italic = Some(true);
        })
    });
    let reset = |app: &Entity<ExcelApp>, cx: &mut VisualTestContext, element: ChartElement| {
        app.update_in(cx, |app, window, cx| {
            app.select_chart_element(element, cx);
            app.run_command(Cmd::ResetStyle, window, cx);
        });
        chart(app, cx)
    };
    let c = reset(&app, cx, grid);
    assert_eq!(c.axes.value.major_gridline, Line::default());
    assert!(c.axes.value.major_gridlines && c.title_layout.is_some());
    let c = reset(&app, cx, ChartElement::Title);
    assert_eq!(
        (c.title_layout, c.title_text.clone()),
        (None, TextFormat::default())
    );
    assert!(c.legend_layout.is_some() && c.plot_layout.is_some());
    let c = reset(&app, cx, ChartElement::PlotArea);
    assert_eq!(c.plot_layout, None);
    // The axis: its line and its labels, not its gridlines' or its title's.
    let c = reset(&app, cx, ChartElement::Axis(AxisId::Category));
    assert_eq!(c.axes.category.line, Line::default());
    // The chart area: everything that was moved, every font.
    let c = reset(&app, cx, ChartElement::ChartArea);
    assert_eq!(c.legend_layout, None);
    assert!(c.axes.value.text.is_auto());
}

#[gpui::test]
fn shape_effects_give_the_areas_a_shadow_and_a_glow(cx: &mut TestAppContext) {
    use opensheet_engine::drawing::ShadowPreset;
    let (app, cx) = open(cx);
    with_chart(&app, cx, ChartKind::Column);
    // A series has no effects here; the plot area and the chart area do.
    app.update(cx, |app, cx| {
        app.select_chart_element(ChartElement::Series(0), cx)
    });
    assert!(!app.update(cx, |app, _| Cmd::ShapeEffects.enabled(app)));
    app.update(cx, |app, cx| {
        app.select_chart_element(ChartElement::PlotArea, cx)
    });
    assert!(app.update(cx, |app, _| Cmd::ShapeEffects.enabled(app)));
    // The menu: Shadow and Glow open lists, the rest is greyed out.
    let entries = app.update(cx, |app, _| effects_menu(app));
    let labels: Vec<(String, bool, bool)> = entries
        .iter()
        .map(|entry| {
            (
                entry.label.to_string(),
                entry.disabled,
                entry.submenu.is_some(),
            )
        })
        .collect();
    assert_eq!(labels[0], ("Shadow".to_string(), false, true));
    assert_eq!(labels[2], ("Glow".to_string(), false, true));
    assert!(labels.iter().filter(|(_, disabled, _)| *disabled).count() == 4);
    // "Offset: Bottom Right" from the Shadow list.
    let pick = |app: &Entity<ExcelApp>,
                cx: &mut VisualTestContext,
                path: [&str; 2],
                deeper: Option<&str>| {
        app.update_in(cx, |app, window, cx| {
            let entries = effects_menu(app);
            let list = entries
                .iter()
                .find(|entry| entry.label.as_ref() == path[0])
                .and_then(|entry| entry.submenu.as_ref())
                .expect("a list");
            let mut entry = list
                .iter()
                .find(|entry| entry.label.as_ref() == path[1])
                .expect("an entry");
            if let Some(deeper) = deeper {
                entry = entry
                    .submenu
                    .as_ref()
                    .and_then(|list| list.iter().find(|entry| entry.label.as_ref() == deeper))
                    .expect("an entry");
            }
            let action = entry.action.clone().expect("an action");
            action(app, window, cx);
        });
        redraw(cx);
    };
    app.update_in(cx, |app, window, cx| {
        app.run_command(Cmd::ShapeEffects, window, cx)
    });
    redraw(cx);
    assert!(app.update(cx, |app, _| app.popup.take().is_some()));
    pick(&app, cx, ["Shadow", "Offset: Bottom Right"], None);
    assert_eq!(
        chart(&app, cx).plot_area.effects.shadow,
        Some(ShadowPreset::BottomRight.shadow())
    );
    assert_eq!(
        app.update(cx, |app, _| app.wb.undo_label().map(str::to_string))
            .as_deref(),
        Some("Shadow")
    );
    // The menu ticks it.
    let ticked = app.update(cx, |app, _| {
        effects_menu(app)[0]
            .submenu
            .as_ref()
            .unwrap()
            .iter()
            .filter(|entry| entry.checked)
            .map(|entry| entry.label.to_string())
            .collect::<Vec<_>>()
    });
    assert_eq!(ticked, vec!["Offset: Bottom Right".to_string()]);
    // A glow on the chart area, in the second accent of the theme.
    app.update(cx, |app, cx| {
        app.select_chart_element(ChartElement::ChartArea, cx)
    });
    pick(&app, cx, ["Glow", "8 pt"], Some("Accent 2"));
    let glow = chart(&app, cx).chart_area.effects.glow.expect("a glow");
    assert_eq!((glow.radius, glow.transparency), (8.0, 0.6));
    assert_eq!(
        glow.color,
        app.update(cx, |app, _| app.wb.theme().accent(2, 0.0))
    );
    // The plot area kept its shadow; each change is one step back.
    assert!(chart(&app, cx).plot_area.effects.shadow.is_some());
    app.update(cx, |app, cx| app.undo(cx));
    assert_eq!(chart(&app, cx).chart_area.effects.glow, None);
    pick(&app, cx, ["Glow", "No Glow"], None);
    app.update(cx, |app, cx| {
        app.select_chart_element(ChartElement::PlotArea, cx)
    });
    pick(&app, cx, ["Shadow", "No Shadow"], None);
    assert!(chart(&app, cx).plot_area.effects.is_none());
    // The outline and the effects of text stay greyed out.
    assert!(!app.update(cx, |app, _| Cmd::TextOutline.enabled(app)
        || Cmd::TextEffects.enabled(app)));
}
