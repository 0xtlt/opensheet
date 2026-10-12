//! Drawing charts: over the grid, and in the previews and thumbnails of
//! dialogs and galleries. The engine lays a chart out as vector primitives
//! (`opensheet_engine::chart_layout`); this module measures text for it,
//! converts the primitives to the grid's display list and adds what belongs
//! to the application: the frames and handles of what is selected.

use crate::grid::{Draw, GridLayout, Layer};
use crate::i18n::t;
use crate::theme::{c, ca};
use gpui::{
    BorderStyle, Bounds, Font, FontStyle, FontWeight, Hsla, IntoElement, PathBuilder, Pixels, Point, ShapedLine,
    SharedString, Styled, TextRun, WindowTextSystem, canvas, fill, point, px, quad, size,
};
use opensheet_engine::chart::{Chart, ChartData};
use opensheet_engine::chart_layout::{
    ChartLayout, Ink, Labels, LayoutOptions, Outline, Prim, Seg, Stroke, TextAnchor, TextMeasure, layout_chart_elements,
    line_height,
};
use opensheet_engine::{Color, Locale};
use std::rc::Rc;

/// What drawing a chart on screen needs besides the chart.
pub struct ChartStyle {
    /// Font family for every label.
    pub font: SharedString,
    pub locale: &'static Locale,
}

/// Pixel box (x0, y0, x1, y1) of a chart in grid-element coordinates.
pub fn chart_rect(layout: &GridLayout, chart: &Chart) -> (f32, f32, f32, f32) {
    let z = layout.zoom;
    let x0 = layout.col_x(chart.anchor.1) + chart.offset.0 * z;
    let y0 = layout.row_y(chart.anchor.0) + chart.offset.1 * z;
    (x0, y0, x0 + chart.size.0 * z, y0 + chart.size.1 * z)
}

/// Asset paths under this prefix are lines of text drawn as vector shapes
/// (see `text_asset`): GPUI can turn a shape, not a line of glyphs.
const TEXT_ASSET: &str = "chart-text/";

fn hex(text: &str) -> String {
    text.bytes().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<String> {
    let bytes: Option<Vec<u8>> = (0..text.len() / 2)
        .map(|i| u8::from_str_radix(text.get(2 * i..2 * i + 2)?, 16).ok())
        .collect();
    String::from_utf8(bytes?).ok()
}

/// The asset path of one line of text drawn as a shape in a box of
/// `width` × `height` pixels, its baseline `baseline` pixels under the top
/// of the box (see `text_asset`). The screen and the print preview both
/// draw turned text through it.
pub fn text_asset_path(width: f32, height: f32, baseline: f32, size: f32, bold: bool, family: &str, text: &str) -> SharedString {
    format!(
        "{TEXT_ASSET}{width:.0}/{height:.0}/{baseline:.2}/{size:.2}/{}/{}/{}.svg",
        if bold { "b" } else { "r" },
        hex(family),
        hex(text)
    )
    .into()
}

/// The SVG image behind an asset path made by `text_asset_path`: one line
/// of text in a box of its own size. `None` for any other path.
pub fn text_asset(path: &str) -> Option<Vec<u8>> {
    let mut parts = path.strip_prefix(TEXT_ASSET)?.strip_suffix(".svg")?.split('/');
    let mut number = || parts.next()?.parse::<f32>().ok().filter(|v| v.is_finite());
    let (w, h, baseline, font_size) = (number()?, number()?, number()?, number()?);
    let weight = if parts.next()? == "b" { "bold" } else { "normal" };
    let family = unhex(parts.next()?)?;
    let text = unhex(parts.next()?)?;
    let escape = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('"', "&quot;");
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}"><text x="{}" y="{baseline}" text-anchor="middle" font-family="{}" font-size="{font_size}" font-weight="{weight}" xml:space="preserve">{}</text></svg>"#,
        w / 2.0,
        escape(&family),
        escape(&text),
    );
    Some(svg.into_bytes())
}

/// Shapes text in the chart's font, for measuring and for drawing.
struct Shaper<'a> {
    ts: &'a WindowTextSystem,
    family: SharedString,
}

impl Shaper<'_> {
    fn font(&self, bold: bool, italic: bool) -> Font {
        Font {
            family: self.family.clone(),
            features: Default::default(),
            fallbacks: None,
            weight: if bold { FontWeight::BOLD } else { FontWeight::NORMAL },
            style: if italic { FontStyle::Italic } else { FontStyle::Normal },
        }
    }

    fn shape(&self, text: &str, size: f32, color: Hsla, bold: bool, italic: bool) -> ShapedLine {
        let text: SharedString = text.replace(['\n', '\r'], " ").into();
        let run = TextRun {
            len: text.len(),
            font: self.font(bold, italic),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        self.ts.shape_line(text, px(size), &[run], None)
    }

    /// The asset path of a line of text, and the box it fills (width,
    /// height), for text that is drawn turned.
    fn turned(&self, text: &str, size: f32, bold: bool) -> (SharedString, f32, f32) {
        let w = f32::from(self.shape(text, size, gpui::black(), bold, false).width).ceil() + 2.0;
        let h = line_height(size);
        let font_id = self.ts.resolve_font(&self.font(bold, false));
        let (ascent, descent) = (f32::from(self.ts.ascent(font_id, px(size))), f32::from(self.ts.descent(font_id, px(size))));
        // Where GPUI itself puts the baseline in a line of this height.
        let baseline = (h - ascent - descent) / 2.0 + ascent;
        (text_asset_path(w, h, baseline, size, bold, &self.family, text), w, h)
    }
}

impl TextMeasure for Shaper<'_> {
    fn width(&self, text: &str, size: f32, bold: bool) -> f32 {
        f32::from(self.shape(text, size, gpui::black(), bold, false).width)
    }
}

fn ink(ink: Ink) -> Hsla {
    if ink.opacity >= 1.0 { c(ink.color.value()) } else { ca(ink.color.value(), ink.opacity.max(0.0)) }
}

/// Converts the engine's primitives, whose origin is the corner of the
/// chart, into display-list items of the window.
struct Converter<'a> {
    shaper: Shaper<'a>,
    items: Vec<Draw>,
    /// Corner of the chart in window coordinates.
    origin: (f32, f32),
    clip: Bounds<Pixels>,
}

impl Converter<'_> {
    fn pt(&self, x: f32, y: f32) -> Point<Pixels> {
        point(px(self.origin.0 + x), px(self.origin.1 + y))
    }

    fn bounds(&self, x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(self.pt(x, y), size(px(w.max(0.0)), px(h.max(0.0))))
    }

    fn fill_path(&mut self, segs: &[Seg], color: Hsla) {
        let mut builder = PathBuilder::fill();
        self.trace(&mut builder, segs);
        if let Ok(path) = builder.build() {
            self.items.push(Draw::Path(path, color));
        }
    }

    fn stroke_path(&mut self, segs: &[Seg], stroke: &Stroke) {
        let mut builder = PathBuilder::stroke(px(stroke.width));
        let dashes: Vec<Pixels> = stroke.dash.pattern().iter().map(|d| px((d * stroke.width).max(1.0))).collect();
        if !dashes.is_empty() {
            builder = builder.dash_array(&dashes);
        }
        self.trace(&mut builder, segs);
        if let Ok(path) = builder.build() {
            self.items.push(Draw::Path(path, ink(stroke.ink)));
        }
    }

    fn trace(&self, builder: &mut PathBuilder, segs: &[Seg]) {
        for seg in segs {
            match *seg {
                Seg::Move(x, y) => builder.move_to(self.pt(x, y)),
                Seg::Line(x, y) => builder.line_to(self.pt(x, y)),
                Seg::Cubic(ax, ay, bx, by, x, y) => {
                    builder.cubic_bezier_to(self.pt(x, y), self.pt(ax, ay), self.pt(bx, by))
                }
                Seg::Close => builder.close(),
            }
        }
    }

    fn push(&mut self, prim: &Prim) {
        match prim {
            Prim::Rect { x, y, w, h, fill: inside, stroke } => {
                let bounds = self.bounds(*x, *y, *w, *h);
                match (inside, stroke) {
                    (Some(inside), None) => self.items.push(Draw::Quad(fill(bounds, ink(*inside)))),
                    (inside, Some(stroke)) if stroke.dash.pattern().is_empty() => {
                        self.items.push(Draw::Quad(quad(
                            bounds,
                            px(0.),
                            inside.map_or(gpui::transparent_black(), ink),
                            px(stroke.width.max(1.0).round()),
                            ink(stroke.ink),
                            BorderStyle::Solid,
                        )));
                    }
                    (inside, Some(stroke)) => {
                        if let Some(inside) = inside {
                            self.items.push(Draw::Quad(fill(bounds, ink(*inside))));
                        }
                        let outline =
                            [Seg::Move(*x, *y), Seg::Line(x + w, *y), Seg::Line(x + w, y + h), Seg::Line(*x, y + h), Seg::Close];
                        self.stroke_path(&outline, stroke);
                    }
                    (None, None) => {}
                }
            }
            Prim::Line { x0, y0, x1, y1, stroke } => {
                let solid = stroke.dash.pattern().is_empty();
                let thick = stroke.width.round().max(1.0);
                if solid && y0 == y1 {
                    // Rules along the pixel grid stay sharp as rectangles.
                    let top = (self.origin.1 + y0 - stroke.width / 2.0).round() - self.origin.1;
                    let b = self.bounds(x0.min(*x1), top, (x1 - x0).abs(), thick);
                    self.items.push(Draw::Quad(fill(b, ink(stroke.ink))));
                } else if solid && x0 == x1 {
                    let left = (self.origin.0 + x0 - stroke.width / 2.0).round() - self.origin.0;
                    let b = self.bounds(left, y0.min(*y1), thick, (y1 - y0).abs());
                    self.items.push(Draw::Quad(fill(b, ink(stroke.ink))));
                } else {
                    self.stroke_path(&[Seg::Move(*x0, *y0), Seg::Line(*x1, *y1)], stroke);
                }
            }
            Prim::Path { segs, fill: inside, stroke } => {
                if let Some(inside) = inside {
                    self.fill_path(segs, ink(*inside));
                }
                if let Some(stroke) = stroke {
                    self.stroke_path(segs, stroke);
                }
            }
            Prim::Text(run) => {
                let color = c(run.color.value());
                let line_h = line_height(run.size);
                if run.rotation != 0.0 {
                    let (path, w, h) = self.shaper.turned(&run.text, run.size, run.bold);
                    // The box of the text before it is turned around its anchor.
                    let left = match run.anchor {
                        TextAnchor::Start => run.x,
                        TextAnchor::Middle => run.x - w / 2.0,
                        TextAnchor::End => run.x - w,
                    };
                    self.items.push(Draw::Turned {
                        path,
                        bounds: self.bounds(left.round(), (run.y - h / 2.0).round(), w, h),
                        pivot: self.pt(run.x.round(), run.y.round()),
                        turn: run.rotation.to_radians(),
                        color,
                        clip: self.clip,
                    });
                    return;
                }
                let line = self.shaper.shape(&run.text, run.size, color, run.bold, run.italic);
                let w = f32::from(line.width);
                let left = match run.anchor {
                    TextAnchor::Start => run.x,
                    TextAnchor::Middle => run.x - w / 2.0,
                    TextAnchor::End => run.x - w,
                };
                // Whole pixels in the window, as the rest of the grid's text.
                let x = (self.origin.0 + left).round();
                let y = (self.origin.1 + run.y - line_h / 2.0).round();
                self.items.push(Draw::Text {
                    line,
                    origin: point(px(x), px(y)),
                    line_height: px(line_h),
                    clip: self.clip,
                });
            }
        }
    }

    fn disc(&mut self, cx: f32, cy: f32, r: f32, color: Color) {
        let mut segs = vec![Seg::Move(cx + r, cy)];
        let n = 20;
        segs.extend((1..n).map(|i| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            Seg::Line(cx + r * a.cos(), cy + r * a.sin())
        }));
        segs.push(Seg::Close);
        self.fill_path(&segs, c(color.value()));
    }
}

/// The words a chart takes from the language of the interface: on the
/// sheet, in previews and on paper.
pub fn chart_labels() -> Labels<'static> {
    Labels {
        // Shown when the chart has an automatic title and several series.
        placeholder_title: t("Chart Title", "Titre du graphique"),
        // Prefix of unnamed series ("Series" → "Series1").
        series_prefix: t("Series", "Série"),
        trend_names: [
            t("Linear", "Linéaire"),
            t("Expon.", "Expon."),
            t("Log.", "Log."),
            t("Poly.", "Poly."),
            t("Power", "Puissance"),
            t("per. Mov. Avg.", "Moy. mobile sur pér."),
        ],
        unit_names: [
            t("Hundreds", "Centaines"),
            t("Thousands", "Milliers"),
            t("Millions", "Millions"),
            t("Billions", "Milliards"),
            t("Trillions", "Billions"),
        ],
    }
}

/// Lay a chart out in a box of `size` pixels on screen, keeping track of
/// its elements. `zoom` scales what Excel measures in points.
pub fn layout(chart: &Chart, data: &ChartData, size: (f32, f32), zoom: f32, ts: &WindowTextSystem, style: &ChartStyle) -> ChartLayout {
    let shaper = Shaper {
        ts,
        family: style.font.clone(),
    };
    let options = LayoutOptions {
        scale: zoom,
        locale: style.locale,
        labels: chart_labels(),
    };
    layout_chart_elements(chart, data, size, &options, &shaper)
}

/// The display list of a chart laid out by [`layout`], its corner at
/// `origin` (window coordinates).
pub fn draws(layout: &ChartLayout, origin: Point<Pixels>, clip: Bounds<Pixels>, ts: &WindowTextSystem, style: &ChartStyle) -> Vec<Draw> {
    let mut out = Converter {
        shaper: Shaper {
            ts,
            family: style.font.clone(),
        },
        items: Vec::with_capacity(layout.prims.len()),
        origin: (f32::from(origin.x), f32::from(origin.y)),
        clip,
    };
    for prim in &layout.prims {
        out.push(prim);
    }
    out.items
}

/// The frames and the round handles of the selected element of a chart.
pub fn outline_draws(outline: &Outline, origin: Point<Pixels>, ts: &WindowTextSystem) -> Vec<Draw> {
    let frame = Color(0x8A8A8A);
    let mut out = Converter {
        shaper: Shaper {
            ts,
            family: SharedString::default(),
        },
        items: Vec::new(),
        origin: (f32::from(origin.x).round(), f32::from(origin.y).round()),
        clip: Bounds::default(),
    };
    for b in &outline.frames {
        out.items.push(Draw::Quad(quad(
            out.bounds(b[0].round(), b[1].round(), (b[2] - b[0]).round(), (b[3] - b[1]).round()),
            px(0.),
            gpui::transparent_black(),
            px(1.),
            c(frame.value()),
            BorderStyle::Solid,
        )));
    }
    for (x, y) in &outline.handles {
        out.disc(*x, *y, 3.5, frame);
        out.disc(*x, *y, 2.5, Color::WHITE);
    }
    out.items
}

/// Build the drawing commands of one chart of the sheet, in the box
/// (x0, y0, x1, y1) the sheet gives it (grid-element coordinates).
pub fn build_chart(
    chart: &Chart,
    data: &ChartData,
    grid: &GridLayout,
    rect: (f32, f32, f32, f32),
    ts: &WindowTextSystem,
    style: &ChartStyle,
) -> (ChartLayout, Vec<Draw>) {
    let (x0, y0, x1, y1) = rect;
    let (w, h) = (x1 - x0, y1 - y0);
    let origin = grid.to_window(x0, y0);
    let laid = layout(chart, data, (w, h), grid.zoom, ts, style);
    let items = draws(&laid, origin, Bounds::new(origin, size(px(w), px(h))), ts, style);
    (laid, items)
}

/// A chart drawn in a dialog, a pane or a gallery: a preview, a thumbnail.
/// `zoom` scales fonts and line widths (1 = as on the sheet at 100 %).
pub fn chart_canvas(
    chart: Rc<Chart>,
    data: Rc<ChartData>,
    width: f32,
    height: f32,
    zoom: f32,
    font: SharedString,
    locale: &'static Locale,
) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, cx| {
            let style = ChartStyle {
                font: font.clone(),
                locale,
            };
            let ts = window.text_system().clone();
            let size = (f32::from(bounds.size.width), f32::from(bounds.size.height));
            let laid = layout(&chart, &data, size, zoom, &ts, &style);
            let items = draws(&laid, bounds.origin, bounds, &ts, &style);
            crate::grid::paint_layers(vec![Layer { clip: bounds, items }], window, cx);
        },
    )
    .w(px(width))
    .h(px(height))
    .flex_none()
}

/// Paint a line of text turned around `pivot` (see `Draw::Turned`).
pub fn paint_turned(
    path: SharedString,
    bounds: Bounds<Pixels>,
    pivot: Point<Pixels>,
    turn: f32,
    color: Hsla,
    window: &mut gpui::Window,
    cx: &gpui::App,
) {
    let scale = window.scale_factor();
    let centre = point(pivot.x.scale(scale), pivot.y.scale(scale));
    let back = point(-pivot.x, -pivot.y);
    let matrix = gpui::TransformationMatrix::unit()
        .translate(centre)
        .rotate(gpui::radians(turn))
        .translate(point(back.x.scale(scale), back.y.scale(scale)));
    let _ = window.paint_svg(bounds, path, matrix, color, cx);
}

/// The asset path of a line of text to draw turned (`Draw::Turned`), and
/// the width and the height of its box: for turned text outside charts
/// (the text of a turned shape).
pub fn turned_text(ts: &WindowTextSystem, family: SharedString, text: &str, size: f32, bold: bool) -> (SharedString, f32, f32) {
    Shaper { ts, family }.turned(text, size, bold)
}
