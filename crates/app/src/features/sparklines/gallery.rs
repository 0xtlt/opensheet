//! The drop-downs of the Sparkline tab that show pictures: the gallery
//! of the 36 styles, and the colour pickers of the line and of the
//! points (with the weights of the line under the colours).

use super::{active_group, change, dialogs, drop_position, labels};
use crate::app::ExcelApp;
use crate::features::Cx;
use crate::i18n::t;
use crate::theme::{self, c};
use crate::view::tooltip;
use crate::widgets::{ColorGrid, preview};
use gpui::{Context, Render, SharedString, WeakEntity, Window, div, prelude::*, px};
use opensheet_engine::Color;
use opensheet_engine::sparkline::{
    STYLE_COUNT, SparkShape, SparklineColors, SparklineGroup, SparklineType, shapes, style,
};

/// One of the colours of a group.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Slot {
    Series,
    Negative,
    Markers,
    High,
    Low,
    First,
    Last,
}

impl Slot {
    /// The entries of the Marker Color menu, in Excel's order.
    pub(super) const POINTS: [Slot; 6] = [
        Slot::Negative,
        Slot::Markers,
        Slot::High,
        Slot::Low,
        Slot::First,
        Slot::Last,
    ];

    pub(super) fn label(self) -> &'static str {
        match self {
            Slot::Series => t("Sparkline Color", "Couleur de sparkline"),
            Slot::Negative => t("Negative Points", "Points négatifs"),
            Slot::Markers => t("Markers", "Marqueurs"),
            Slot::High => t("High Point", "Point haut"),
            Slot::Low => t("Low Point", "Point bas"),
            Slot::First => t("First Point", "Premier point"),
            Slot::Last => t("Last Point", "Dernier point"),
        }
    }

    pub(super) fn of(self, colors: &mut SparklineColors) -> &mut Color {
        match self {
            Slot::Series => &mut colors.series,
            Slot::Negative => &mut colors.negative,
            Slot::Markers => &mut colors.markers,
            Slot::High => &mut colors.high,
            Slot::Low => &mut colors.low,
            Slot::First => &mut colors.first,
            Slot::Last => &mut colors.last,
        }
    }
}

/// The name Excel gives the style at `index` of the gallery.
pub(super) fn style_name(index: usize) -> String {
    let n = (index % 6 + 1).to_string();
    let pattern = match index / 6 {
        0 => t(
            "Sparkline Style Accent |, Darker 50%",
            "Style Sparkline accentué |, plus foncé 50 %",
        ),
        1 => t(
            "Sparkline Style Accent |, Darker 25%",
            "Style Sparkline accentué |, plus foncé 25 %",
        ),
        2 => t(
            "Sparkline Style Accent |, (no dark or light)",
            "Style Sparkline accentué |, (ni foncé ni clair)",
        ),
        3 => t(
            "Sparkline Style Accent |, Lighter 40%",
            "Style Sparkline accentué |, plus clair 40 %",
        ),
        4 => t("Sparkline Style Dark #|", "Style Sparkline foncé #|"),
        _ => t("Sparkline Style Colorful #|", "Style Sparkline coloré # |"),
    };
    pattern.replace('|', &n)
}

/// Give the selected groups a style of the gallery.
pub(super) fn apply_style(app: &mut ExcelApp, index: usize, cx: &mut Cx) {
    let colors = style(index);
    change(app, labels::style(), cx, |group| group.colors = colors);
}

/// Give one colour to the selected groups.
pub(super) fn apply_color(app: &mut ExcelApp, slot: Slot, color: Color, cx: &mut Cx) {
    change(app, labels::color(), cx, |group| {
        *slot.of(&mut group.colors) = color
    });
}

pub(super) fn apply_weight(app: &mut ExcelApp, weight: f64, cx: &mut Cx) {
    change(app, labels::color(), cx, |group| group.line_weight = weight);
}

// ------------------------------------------------------------------- styles

/// The numbers the thumbnails plot: ups and downs with a low point, a
/// high point and negative values, so that every colour shows.
const SAMPLE: [Option<f64>; 8] = [
    Some(2.0),
    Some(4.0),
    Some(-2.0),
    Some(1.0),
    Some(5.0),
    Some(-1.0),
    Some(3.0),
    Some(2.5),
];

struct StyleGallery {
    app: WeakEntity<ExcelApp>,
    /// The group of the active cell: thumbnails show its type and options.
    model: SparklineGroup,
}

pub(super) fn open_styles(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let Some(group) = active_group(app) else {
        return;
    };
    // Thumbnails show every kind of point in its colour, as Excel's do.
    let model = SparklineGroup {
        sparklines: Vec::new(),
        high: true,
        low: true,
        first: true,
        last: true,
        negative: true,
        markers: group.kind == SparklineType::Line,
        line_weight: 1.0,
        ..group.clone()
    };
    let position = drop_position(app, window);
    let weak = cx.weak_entity();
    let view = cx.new(|_| StyleGallery { app: weak, model });
    app.open_popup_view(position, view, cx);
}

impl Render for StyleGallery {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        const THUMB: (f32, f32) = (64.0, 26.0);
        let current = self.model.colors;
        let mut grid = div()
            .w(px(6.0 * (THUMB.0 + 12.0) + 16.0))
            .p(px(8.))
            .flex()
            .flex_wrap()
            .bg(c(theme::PANEL_BG));
        for index in 0..STYLE_COUNT {
            let group = SparklineGroup {
                colors: style(index),
                ..self.model.clone()
            };
            let chosen = style(index) == current;
            let picture = preview(THUMB.0, THUMB.1, move |s| {
                for shape in shapes(&group, &SAMPLE, Default::default(), THUMB.0, THUMB.1, 1.0) {
                    match shape {
                        SparkShape::Rect {
                            x0,
                            y0,
                            x1,
                            y1,
                            color,
                        } => s.rect(x0, y0, x1, y1, theme::engine_color(color)),
                        SparkShape::Line {
                            points,
                            color,
                            width,
                        } => {
                            for pair in points.windows(2) {
                                s.line(pair[0], pair[1], width, theme::engine_color(color));
                            }
                        }
                    }
                }
            });
            grid = grid.child(
                div()
                    .id(("sparkline-style", index))
                    .p(px(5.))
                    .border_1()
                    .rounded(px(3.))
                    .border_color(if chosen {
                        c(theme::ACCENT)
                    } else {
                        gpui::transparent_black()
                    })
                    .hover(|s| s.bg(c(theme::HOVER)).border_color(c(theme::BORDER_STRONG)))
                    .child(picture)
                    .tooltip(tooltip(SharedString::from(style_name(index))))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.app
                            .update(cx, |app, cx| {
                                app.close_popup(window, cx);
                                apply_style(app, index, cx);
                            })
                            .ok();
                    })),
            );
        }
        grid
    }
}

// ------------------------------------------------------------------ colours

/// The weights of the Weight list, in points.
const WEIGHTS: [(f64, &str); 9] = [
    (0.25, "¼ pt"),
    (0.5, "½ pt"),
    (0.75, "¾ pt"),
    (1.0, "1 pt"),
    (1.5, "1½ pt"),
    (2.25, "2¼ pt"),
    (3.0, "3 pt"),
    (4.5, "4½ pt"),
    (6.0, "6 pt"),
];

struct ColorPicker {
    app: WeakEntity<ExcelApp>,
    slot: Slot,
    current: Color,
    /// The weight of the line, shown under the colours of the series.
    weight: Option<f64>,
}

pub(super) fn open_color(app: &mut ExcelApp, slot: Slot, window: &mut Window, cx: &mut Cx) {
    let Some(group) = active_group(app) else {
        return;
    };
    let mut colors = group.colors;
    let current = *slot.of(&mut colors);
    let weight =
        (slot == Slot::Series && group.kind == SparklineType::Line).then_some(group.line_weight);
    let position = drop_position(app, window);
    let weak = cx.weak_entity();
    let view = cx.new(|_| ColorPicker {
        app: weak,
        slot,
        current,
        weight,
    });
    app.open_popup_view(position, view, cx);
}

impl ColorPicker {
    fn run(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
        action: impl FnOnce(&mut ExcelApp, &mut Window, &mut Cx),
    ) {
        self.app
            .update(cx, |app, cx| {
                app.close_popup(window, cx);
                action(app, window, cx);
            })
            .ok();
    }
}

impl Render for ColorPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let slot = self.slot;
        let mut body = div()
            .p(px(8.))
            .flex()
            .flex_col()
            .gap(px(4.))
            .bg(c(theme::PANEL_BG))
            .text_size(px(12.))
            .child(
                ColorGrid::new("sparkline-colors", Some(Some(self.current)))
                    .more(&self.app)
                    .on_pick(cx.listener(move |this, color: &Option<Color>, window, cx| {
                        if let Some(color) = *color {
                            this.run(window, cx, |app, _, cx| apply_color(app, slot, color, cx));
                        }
                    })),
            );
        if let Some(weight) = self.weight {
            body = body
                .child(div().h(px(1.)).my(px(2.)).bg(c(theme::BORDER)))
                .child(
                    div()
                        .px(px(4.))
                        .text_color(c(theme::TEXT_MUTED))
                        .child(t("Weight", "Épaisseur")),
                );
            for (index, (points, text)) in WEIGHTS.into_iter().enumerate() {
                let chosen = (weight - points).abs() < 1e-6;
                body = body.child(
                    div()
                        .id(("sparkline-weight", index))
                        .h(px(22.))
                        .px(px(4.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .rounded(px(3.))
                        .when(chosen, |d| d.bg(c(theme::TOGGLED)))
                        .hover(|s| s.bg(c(theme::HOVER)))
                        .child(div().w(px(44.)).child(text))
                        .child(
                            div()
                                .flex_1()
                                .h(px((points as f32 * 96.0 / 72.0).max(1.0)))
                                .bg(c(0x000000)),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.run(window, cx, |app, _, cx| apply_weight(app, points, cx))
                        })),
                );
            }
            body = body.child(
                div()
                    .id("sparkline-weight-custom")
                    .h(px(22.))
                    .px(px(4.))
                    .flex()
                    .items_center()
                    .rounded(px(3.))
                    .hover(|s| s.bg(c(theme::HOVER)))
                    .child(t("Custom Weight…", "Épaisseur personnalisée…"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.run(window, cx, |app, window, cx| {
                            dialogs::open_weight(app, window, cx);
                        })
                    })),
            );
        }
        body
    }
}
