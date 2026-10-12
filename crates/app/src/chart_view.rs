//! Drawing charts over the grid. The engine lays a chart out as vector
//! primitives (`opensheet_engine::chart_layout`); this module measures text
//! for it, converts the primitives to the grid's display list and adds what
//! belongs to the application: the selection frame and its handles.

use crate::grid::{Draw, GridLayout};
use crate::i18n::t;
use crate::theme::{c, ca};
use gpui::{
    BorderStyle, Bounds, Font, FontStyle, FontWeight, Hsla, PathBuilder, Pixels, Point, ShapedLine, SharedString,
    TextRun, WindowTextSystem, fill, point, px, quad, size,
};
use opensheet_engine::chart::{Chart, ChartData};
use opensheet_engine::chart_layout::{
    Ink, Labels, LayoutOptions, Prim, Seg, Stroke, TextAnchor, TextMeasure, layout_chart, line_height,
};
use opensheet_engine::{Color, Locale};

pub struct ChartStyle<'a> {
    /// Font family for every label.
    pub font: SharedString,
    /// Shown when the chart has an automatic title and several series.
    pub placeholder_title: &'a str,
    /// Prefix of unnamed series ("Series" → "Series1").
    pub series_prefix: &'a str,
    pub locale: &'static Locale,
}

/// Pixel box (x0, y0, x1, y1) of a chart in grid-element coordinates.
pub fn chart_rect(layout: &GridLayout, chart: &Chart) -> (f32, f32, f32, f32) {
    let z = layout.zoom;
    let x0 = layout.col_x(chart.anchor.1) + chart.offset.0 * z;
    let y0 = layout.row_y(chart.anchor.0) + chart.offset.1 * z;
    (x0, y0, x0 + chart.size.0 * z, y0 + chart.size.1 * z)
}

/// The eight resize handles: corners and edge midpoints, clockwise from the
/// top-left corner.
pub fn handle_points(rect: (f32, f32, f32, f32)) -> [(f32, f32); 8] {
    let (x0, y0, x1, y1) = rect;
    let (mx, my) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    [
        (x0, y0),
        (mx, y0),
        (x1, y0),
        (x1, my),
        (x1, y1),
        (mx, y1),
        (x0, y1),
        (x0, my),
    ]
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

/// The SVG image behind an asset path made by `Shaper::turned`: one line of
/// text in a box of its own size. `None` for any other path.
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
        let path = format!(
            "{TEXT_ASSET}{w:.0}/{h:.0}/{baseline:.2}/{size:.2}/{}/{}/{}.svg",
            if bold { "b" } else { "r" },
            hex(&self.family),
            hex(text)
        );
        (path.into(), w, h)
    }
}

impl TextMeasure for Shaper<'_> {
    fn width(&self, text: &str, size: f32, bold: bool) -> f32 {
        f32::from(self.shape(text, size, gpui::black(), bold, false).width)
    }
}

fn ink(ink: Ink) -> Hsla {
    if ink.opacity >= 1.0 { c(ink.color.0) } else { ca(ink.color.0, ink.opacity.max(0.0)) }
}

/// Converts the engine's primitives, whose origin is the corner of the
/// chart, into display-list items of the window.
struct Converter<'a> {
    layout: &'a GridLayout,
    shaper: Shaper<'a>,
    items: Vec<Draw>,
    /// Corner of the chart in grid-element coordinates.
    origin: (f32, f32),
    clip: Bounds<Pixels>,
}

impl Converter<'_> {
    fn pt(&self, x: f32, y: f32) -> Point<Pixels> {
        self.layout.to_window(self.origin.0 + x, self.origin.1 + y)
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
                let color = c(run.color.0);
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
                    origin: self.layout.to_window(x, y),
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
        self.fill_path(&segs, c(color.0));
    }
}

/// Build the drawing commands of one chart.
pub fn build_chart(
    chart: &Chart,
    data: &ChartData,
    layout: &GridLayout,
    ts: &WindowTextSystem,
    style: &ChartStyle,
    selected: bool,
) -> Vec<Draw> {
    let (x0, y0, x1, y1) = chart_rect(layout, chart);
    let (w, h) = (x1 - x0, y1 - y0);
    let mut out = Converter {
        layout,
        shaper: Shaper {
            ts,
            family: style.font.clone(),
        },
        items: Vec::new(),
        origin: (x0, y0),
        clip: Bounds::new(layout.to_window(x0, y0), size(px(w), px(h))),
    };
    let options = LayoutOptions {
        scale: layout.zoom,
        locale: style.locale,
        labels: Labels {
            placeholder_title: style.placeholder_title,
            series_prefix: style.series_prefix,
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
        },
    };
    for prim in layout_chart(chart, data, (w, h), &options, &out.shaper) {
        out.push(&prim);
    }

    if selected {
        let frame = Color(0x8A8A8A);
        out.items.push(Draw::Quad(quad(
            out.bounds(-1.0, -1.0, w + 2.0, h + 2.0),
            px(0.),
            gpui::transparent_black(),
            px(1.),
            c(frame.0),
            BorderStyle::Solid,
        )));
        // Handles sit on the frame and may poke out of the chart box.
        for (hx, hy) in handle_points((0.0, 0.0, w, h)) {
            out.disc(hx, hy, 4.5, frame);
            out.disc(hx, hy, 3.5, Color::WHITE);
        }
    }
    out.items
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
