//! WordArt, and the look of the text of shapes in general: text formatted
//! in runs, letters with an outline and a shadow, the WordArt gallery, the
//! Text Outline and Text Effects drop-downs, and the formatting commands
//! that act on the text selected in a shape (or on all of it).
//!
//! Plain runs (font, colour, bold…) are drawn by GPUI like any text.
//! Letters with an outline, a shadow or no fill are drawn as a vector
//! image of the text block (`svg.rs`): GPUI's glyphs have a colour and
//! nothing else.
//!
//! Not done: Transform (warped text), glow, reflection, bevels, gradient
//! fills of letters; one size per shape on screen (the size of its first
//! run), because GPUI lays a text out at one size.

use super::geometry::Placed;
use super::paint::{self, Brush};
use super::text::TextLook;
use super::{Cmd, DrawingUi, Drawings, SHAPE_TAB, TextStart, begin_text_edit, selected, svg, tabs};
use crate::app::{ExcelApp, MenuEntry};
use crate::features::{Command, Cx, Intent, object_id};
use crate::grid::{Draw, FontResolver, pt_to_px};
use crate::i18n::t;
use crate::theme::{self, c, engine_color};
use crate::widgets::ColorGrid;
use gpui::{
    App, Bounds, Context, Font, FontStyle, FontWeight, Pixels, SharedString, StrikethroughStyle, TextRun, TextSystem,
    UnderlineStyle, WeakEntity, Window, WindowTextSystem, WrappedLine, canvas, div, point, prelude::*, px, size,
};
use opensheet_engine::drawing::{Drawing, Shape, TEXT_MARGIN};
use opensheet_engine::style::{Color, Font as EngineFont, Style, Underline};
use opensheet_engine::wordart::{TextEffects, TextOutline, TextShadow, WordArtStyle};
use std::fmt::Write as _;
use std::ops::Range;

/// The font of a run as GPUI names it.
pub(super) fn gpui_font(font: &EngineFont, fonts: &FontResolver) -> Font {
    Font {
        family: fonts.resolve(&font.name),
        features: Default::default(),
        fallbacks: None,
        weight: if font.bold { FontWeight::BOLD } else { FontWeight::NORMAL },
        style: if font.italic { FontStyle::Italic } else { FontStyle::Normal },
    }
}

/// The paragraphs of the text of a shape laid out in a box `width` wide,
/// run by run, and the number of rows they take. A text that is not
/// wrapped only breaks where its paragraphs end.
pub(super) fn wrap(shape: &Shape, look: &TextLook, fonts: &FontResolver, width: f32, ts: &WindowTextSystem) -> (Vec<WrappedLine>, usize) {
    let decoration = (look.size / 14.0).max(1.0);
    let runs: Vec<TextRun> = shape
        .text_runs()
        .into_iter()
        .map(|(covered, font, _)| {
            let color = font.color.map_or(look.color, engine_color);
            TextRun {
                len: covered.len(),
                font: gpui_font(&font, fonts),
                color,
                background_color: None,
                underline: (font.underline != Underline::None).then_some(UnderlineStyle {
                    color: Some(color),
                    thickness: px(decoration),
                    wavy: false,
                }),
                strikethrough: font.strike.then_some(StrikethroughStyle {
                    color: Some(color),
                    thickness: px(decoration),
                }),
            }
        })
        .collect();
    let wrap_width = shape.wrap.then_some(px(width.max(8.0)));
    let lines: Vec<WrappedLine> = ts
        .shape_text(SharedString::from(shape.text.clone()), px(look.size), &runs, wrap_width, None)
        .map(|lines| lines.into_iter().collect())
        .unwrap_or_default();
    let rows = lines.iter().map(|line| line.wrap_boundaries().len() + 1).sum();
    (lines, rows)
}

/// The bytes of the text each row shows.
fn row_ranges(lines: &[WrappedLine]) -> Vec<Range<usize>> {
    let mut rows = Vec::new();
    let mut offset = 0;
    for line in lines {
        let mut start = 0;
        for boundary in line.wrap_boundaries() {
            let end = line.runs()[boundary.run_ix].glyphs[boundary.glyph_ix].index;
            rows.push(offset + start..offset + end);
            start = end;
        }
        rows.push(offset + start..offset + line.text.len());
        // The line feed between two paragraphs.
        offset += line.text.len() + 1;
    }
    rows
}

// ---- letters as a vector image -------------------------------------------

/// A run of the text as the image draws it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ArtRun {
    pub range: Range<usize>,
    pub family: String,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub color: Color,
    pub effects: TextEffects,
}

/// A block of text to draw as an image. Lengths are in pixels.
pub(super) struct ArtBlock<'a> {
    pub text: &'a str,
    pub rows: &'a [Range<usize>],
    pub runs: &'a [ArtRun],
    /// Size of the letters.
    pub size: f32,
    /// Width of the box the rows are aligned in.
    pub width: f32,
    pub line_height: f32,
    /// How far under the top of its row the baseline of a row is.
    pub baseline: f32,
    pub align: gpui::TextAlign,
    /// Pixels per point: outlines and shadows are given in points.
    pub per_point: f32,
    /// Degrees the block is turned by, clockwise, about `pivot` (measured
    /// from the top-left corner of the block).
    pub turn: f32,
    pub pivot: (f32, f32),
    /// Device pixels per pixel.
    pub scale: f32,
}

/// Where the image of a block goes: its box (left, top, width, height)
/// measured from the top-left corner of the block.
pub(super) type ArtBox = (f32, f32, f32, f32);

fn hex(color: Color) -> String {
    format!("#{:06X}", color.value() & 0xFF_FFFF)
}

impl ArtBlock<'_> {
    /// Room kept around the block for what reaches out of it: shadows,
    /// outlines, slanted letters.
    fn pad(&self) -> f32 {
        let reach = self
            .runs
            .iter()
            .map(|run| {
                let outline = run.effects.outline.map_or(0.0, |line| line.width);
                let shadow = run.effects.shadow.map_or(0.0, |shadow| shadow.distance + 2.0 * shadow.blur);
                (outline + shadow) * self.per_point
            })
            .fold(0.0, f32::max);
        (reach + self.size * 0.25).ceil()
    }

    /// The spans of one row. `shadow`: the copy drawn behind the letters
    /// for that shadow, in which runs with another shadow do not show.
    fn spans(&self, row: &Range<usize>, shadow: Option<&TextShadow>, out: &mut String) {
        let shown = self.text[row.clone()].trim_end();
        let end = row.start + shown.len();
        for run in self.runs {
            let (from, to) = (run.range.start.max(row.start), run.range.end.min(end));
            if from >= to {
                continue;
            }
            let _ = write!(
                out,
                r#"<tspan font-family="'{}', Helvetica, Arial, sans-serif" font-weight="{}" font-style="{}""#,
                svg::escape(&run.family),
                if run.bold { "bold" } else { "normal" },
                if run.italic { "italic" } else { "normal" },
            );
            let decoration = match (run.underline, run.strike) {
                (true, true) => "underline line-through",
                (true, false) => "underline",
                (false, true) => "line-through",
                (false, false) => "",
            };
            if !decoration.is_empty() {
                let _ = write!(out, r#" text-decoration="{decoration}""#);
            }
            let stroke_width = run.effects.outline.map(|line| (line.width * self.per_point).max(0.5));
            match shadow {
                Some(shadow) if run.effects.shadow.as_ref() == Some(shadow) => {
                    let ink = hex(shadow.color);
                    let opacity = shadow.opacity.clamp(0.0, 1.0);
                    if !run.effects.hollow {
                        let _ = write!(out, r#" fill="{ink}" fill-opacity="{opacity}""#);
                    } else {
                        out.push_str(r#" fill="none""#);
                    }
                    if let Some(width) = stroke_width {
                        let _ = write!(out, r#" stroke="{ink}" stroke-opacity="{opacity}" stroke-width="{width:.2}" stroke-linejoin="round""#);
                    }
                }
                Some(_) => out.push_str(r#" fill="none""#),
                None => {
                    if run.effects.hollow {
                        out.push_str(r#" fill="none""#);
                    } else {
                        let _ = write!(out, r#" fill="{}""#, hex(run.color));
                    }
                    if let (Some(width), Some(line)) = (stroke_width, run.effects.outline) {
                        let _ = write!(out, r#" stroke="{}" stroke-width="{width:.2}" stroke-linejoin="round""#, hex(line.color));
                    }
                }
            }
            let _ = write!(out, ">{}</tspan>", svg::escape(&self.text[from..to]));
        }
    }

    /// The document that draws the block, and the box it is painted in.
    pub fn svg(&self) -> (String, ArtBox) {
        let pad = self.pad();
        let height = self.rows.len().max(1) as f32 * self.line_height;
        let turned = self.turn.rem_euclid(360.0) > 0.05 && self.turn.rem_euclid(360.0) < 359.95;
        // The image: the block with room around it or, turned, the
        // square that holds it whichever way it is turned about its pivot.
        let (left, top, w, h) = if turned {
            let far_x = (self.pivot.0 + pad).max(self.width + pad - self.pivot.0);
            let far_y = (self.pivot.1 + pad).max(height + pad - self.pivot.1);
            let reach = far_x.hypot(far_y).ceil();
            (self.pivot.0 - reach, self.pivot.1 - reach, 2.0 * reach, 2.0 * reach)
        } else {
            (-pad, -pad, self.width + 2.0 * pad, height + 2.0 * pad)
        };
        let fit = (svg::MAX_SIDE / (w.max(h) * self.scale).max(1.0)).min(1.0);
        let mut out = String::new();
        let _ = write!(
            out,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{:.0}" height="{:.0}" viewBox="0 0 {w} {h}">"#,
            (w * self.scale * fit).ceil(),
            (h * self.scale * fit).ceil(),
        );
        // One blur per kind of shadow.
        let mut shadows: Vec<TextShadow> = Vec::new();
        for shadow in self.runs.iter().filter_map(|run| run.effects.shadow) {
            if !shadows.contains(&shadow) {
                shadows.push(shadow);
            }
        }
        out.push_str("<defs>");
        for (i, shadow) in shadows.iter().enumerate() {
            let spread = shadow.blur * self.per_point / 2.0;
            if spread > 0.0 {
                let _ = write!(
                    out,
                    r#"<filter id="s{i}" x="-20%" y="-60%" width="140%" height="220%"><feGaussianBlur stdDeviation="{spread:.2}"/></filter>"#
                );
            }
        }
        out.push_str("</defs>");
        let _ = write!(out, r#"<g transform="translate({:.2} {:.2})""#, -left, -top);
        let _ = write!(out, r#" font-size="{:.2}">"#, self.size);
        if turned {
            let _ = write!(out, r#"<g transform="rotate({:.2} {:.2} {:.2})">"#, self.turn, self.pivot.0, self.pivot.1);
        }
        let (x, anchor) = match self.align {
            gpui::TextAlign::Left => (0.0, "start"),
            gpui::TextAlign::Center => (self.width / 2.0, "middle"),
            gpui::TextAlign::Right => (self.width, "end"),
        };
        for layer in shadows.iter().map(Some).chain([None]) {
            for (i, row) in self.rows.iter().enumerate() {
                let y = i as f32 * self.line_height + self.baseline;
                let _ = write!(out, r#"<text x="{x:.2}" y="{y:.2}" text-anchor="{anchor}" xml:space="preserve""#);
                if let Some(shadow) = layer {
                    let (dx, dy) = shadow.shift();
                    let _ = write!(out, r#" transform="translate({:.2} {:.2})""#, dx * self.per_point, dy * self.per_point);
                    let index = shadows.iter().position(|other| other == shadow).unwrap_or(0);
                    if shadow.blur > 0.0 {
                        let _ = write!(out, r#" filter="url(#s{index})""#);
                    }
                }
                out.push('>');
                self.spans(row, layer, &mut out);
                out.push_str("</text>");
            }
        }
        if turned {
            out.push_str("</g>");
        }
        out.push_str("</g></svg>");
        (out, (left, top, w, h))
    }
}

/// The runs of a shape as the image draws them.
fn art_runs(shape: &Shape, fonts: &FontResolver) -> Vec<ArtRun> {
    shape
        .text_runs()
        .into_iter()
        .map(|(range, font, effects)| ArtRun {
            range,
            family: fonts.resolve(&font.name).to_string(),
            bold: font.bold,
            italic: font.italic,
            underline: font.underline != Underline::None,
            strike: font.strike,
            color: font.color.unwrap_or(Color::BLACK),
            effects,
        })
        .collect()
}

/// Draw the text of a shape whose letters have an outline, a shadow or no
/// fill: an image of the whole block, turned with the shape.
pub(super) fn draw_art(shape: &Shape, placed: &Placed, natural: (f32, f32), brush: &Brush, fonts: &FontResolver, out: &mut Vec<Draw>) {
    let look = paint::text_look(shape, brush.zoom, fonts);
    // Laid out as in the level shape, then turned about its middle.
    let level = Placed { rotation: 0.0, ..*placed };
    let (x0, y0, x1, y1) = paint::text_box(shape, &level, natural, brush.zoom);
    let (lines, count) = wrap(shape, &look, fonts, x1 - x0, brush.ts);
    let rows = row_ranges(&lines);
    let line_height = f32::from(look.line_height());
    let top = y0 + look.top(count, y1 - y0);
    let font_id = brush.ts.resolve_font(&look.font);
    let (ascent, descent) = (
        f32::from(brush.ts.ascent(font_id, px(look.size))),
        f32::from(brush.ts.descent(font_id, px(look.size))),
    );
    let runs = art_runs(shape, fonts);
    let centre = placed.centre();
    let block = ArtBlock {
        text: &shape.text,
        rows: &rows,
        runs: &runs,
        size: look.size,
        width: x1 - x0,
        line_height,
        // Where GPUI puts the baseline in a line of this height.
        baseline: (line_height - ascent - descent) / 2.0 + ascent,
        align: look.align(),
        per_point: pt_to_px(1.0) * brush.zoom,
        turn: placed.rotation,
        pivot: (centre.0 - x0, centre.1 - top),
        scale: brush.scale,
    };
    let (document, (left, up, width, height)) = block.svg();
    let Some(image) = svg::image(&document, (false, false), 0) else {
        return;
    };
    let origin = point(brush.origin.x + px(x0 + left), brush.origin.y + px(top + up));
    out.push(Draw::Image {
        image,
        bounds: Bounds::new(origin, size(px(width), px(height))),
    });
}

// ---- measuring -----------------------------------------------------------

/// The width of a line of text in a font, in pixels: the advances of its
/// characters (kerning apart), which is what sizing a box needs.
fn line_width(ts: &TextSystem, font: &Font, size: f32, text: &str) -> f32 {
    let id = ts.resolve_font(font);
    text.chars()
        .map(|ch| ts.advance(id, px(size), ch).map_or(size * 0.5, |advance| f32::from(advance.width)))
        .sum()
}

/// The size (at 100 %) of the box that fits the text of a shape whose text
/// is not wrapped: what Excel's "resize shape to fit text" gives WordArt.
pub(super) fn fitted_size(shape: &Shape, fonts: &FontResolver, ts: &TextSystem) -> (f32, f32) {
    let look_size = pt_to_px(shape.font.size_pt());
    let line_height = (look_size * 1.2).round().max(1.0);
    let runs = shape.text_runs();
    let mut widest = 0.0f32;
    let mut start = 0;
    let mut rows = 0;
    for paragraph in shape.text.split('\n') {
        let end = start + paragraph.len();
        let mut width = 0.0;
        for (covered, font, _) in &runs {
            let (from, to) = (covered.start.max(start), covered.end.min(end));
            if from < to {
                width += line_width(ts, &gpui_font(font, fonts), look_size, &shape.text[from..to]);
            }
        }
        widest = widest.max(width);
        rows += 1;
        start = end + 1;
    }
    (
        (widest + 2.0 * TEXT_MARGIN.0 + look_size * 0.1).ceil().max(2.0 * TEXT_MARGIN.0 + look_size),
        (rows as f32 * line_height + 2.0 * TEXT_MARGIN.1).ceil(),
    )
}

/// Bring the box of a shape whose text is not wrapped to the size of its
/// text, keeping its top-left corner. Part of the undo step being made.
pub(super) fn fit_to_text(app: &mut ExcelApp, id: u32, cx: &mut Cx) {
    let sheet = app.sheet;
    let Some(shape) = app.wb.drawing(sheet, id).and_then(Drawing::as_shape).filter(|shape| !shape.wrap) else {
        return;
    };
    let fitted = fitted_size(shape, &app.fonts, cx.text_system());
    app.wb.update_drawing(sheet, id, |drawing| drawing.size = fitted);
}

// ---- formatting ----------------------------------------------------------

/// The bytes of the text selected in the shape being typed in, with the
/// text as it stands in the editor. `None` when no text is being typed or
/// nothing is selected in it: a command then formats the whole text.
fn typed_selection(app: &ExcelApp, id: u32, cx: &App) -> Option<(String, Range<usize>)> {
    let edit = app.ext::<DrawingUi>()?.text.as_ref().filter(|edit| edit.id == id)?;
    let area = edit.area.read(cx);
    Some((area.text().to_string(), area.selection()))
}

/// Change the look of the text of the selected shape as one undo step:
/// of the characters selected in it while it is being typed in, else of
/// all of it.
pub(super) fn format(app: &mut ExcelApp, label: &str, cx: &mut Cx, change: &dyn Fn(&mut EngineFont, &mut TextEffects)) -> bool {
    let Some(id) = selected(app).filter(|drawing| drawing.as_shape().is_some()).map(|drawing| drawing.id) else {
        return false;
    };
    format_shape(app, id, label, cx, &|shape, range| shape.format_text(range, change))
}

/// Run a change on a shape with the range of its text a formatting
/// command is meant for; what is being typed is put into the shape first,
/// so that the range means what the user sees.
fn format_shape(app: &mut ExcelApp, id: u32, label: &str, cx: &mut Cx, change: &dyn Fn(&mut Shape, Range<usize>)) -> bool {
    if !app.may(Intent::EditObject(Some(object_id(&Drawings, id))), cx) {
        return false;
    }
    let typed = typed_selection(app, id, cx);
    let sheet = app.sheet;
    // A style of the gallery names colours of the theme: they take their
    // values in the theme of this workbook.
    let theme = app.wb.theme_arc();
    app.wb.update_drawing(sheet, id, |drawing| {
        let Some(shape) = drawing.as_shape_mut() else {
            return;
        };
        let range = match typed {
            Some((text, range)) => {
                if shape.text != text {
                    shape.set_text(text);
                }
                range
            }
            None => 0..0,
        };
        change(shape, range);
        theme.resolve_shape(shape);
    });
    fit_to_text(app, id, cx);
    super::equation::fit(app, id, cx);
    app.commit(label, cx);
    true
}

/// A formatting command of the Home tab on a shape: `change` is what it
/// does to a cell style. The font goes to the selected characters (or to
/// all), the fill and the alignment to the shape.
pub(super) fn restyle(app: &mut ExcelApp, id: u32, label: &str, change: &dyn Fn(&mut Style), cx: &mut Cx) -> bool {
    format_shape(app, id, label, cx, &|shape, range| restyle_shape(shape, range, change))
}

pub(super) fn restyle_shape(shape: &mut Shape, range: Range<usize>, change: &dyn Fn(&mut Style)) {
    let font = shape.font.clone();
    super::restyle(shape, change);
    // The font is a matter of runs: each takes the change by itself.
    shape.font = font;
    shape.format_text(range, &|font, _| {
        let mut style = Style {
            font: font.clone(),
            ..Style::default()
        };
        change(&mut style);
        *font = style.font;
    });
}

/// Text Fill: a colour, or `None` for letters without fill.
pub(super) fn set_fill(app: &mut ExcelApp, color: Option<Color>, cx: &mut Cx) {
    format(app, Cmd::TextFill.label(), cx, &|font, effects| {
        font.color = color;
        effects.hollow = color.is_none();
    });
}

fn set_outline(app: &mut ExcelApp, cx: &mut Cx, change: &dyn Fn(&mut Option<TextOutline>)) {
    format(app, Cmd::TextOutline.label(), cx, &|_, effects| change(&mut effects.outline));
}

fn set_shadow(app: &mut ExcelApp, shadow: Option<TextShadow>, cx: &mut Cx) {
    format(app, t("Text Shadow", "Ombre du texte"), cx, &|_, effects| effects.shadow = shadow);
}

fn apply_style(app: &mut ExcelApp, style: WordArtStyle, cx: &mut Cx) {
    format(app, t("WordArt Style", "Style WordArt"), cx, &|font, effects| style.apply(font, effects));
}

// ---- inserting -----------------------------------------------------------

/// Where the top-left corner of a new object of the given size goes for
/// it to sit in the middle of what the window shows of the sheet (the
/// scrolling pane); on the active cell when the grid has not been laid
/// out yet.
pub(super) fn middle_of_view(app: &ExcelApp, size: (f32, f32)) -> (f64, f64) {
    let sheet = &app.wb.sheets[app.sheet];
    let middle = app.current_layout().and_then(|layout| {
        let pane = *paint::panes(&layout).last()?;
        let (x, y) = ((pane.clip.0 + pane.clip.2) / 2.0, (pane.clip.1 + pane.clip.3) / 2.0);
        Some(paint::sheet_point(&layout, sheet, &pane, x, y))
    });
    match middle {
        Some((x, y)) => ((x - size.0 as f64 / 2.0).max(0.0), (y - size.1 as f64 / 2.0).max(0.0)),
        None => sheet.corner_point((app.active(), (0.0, 0.0))),
    }
}

/// Insert WordArt in the middle of what the window shows of the sheet,
/// with its text selected, ready to be typed over.
pub(super) fn insert(app: &mut ExcelApp, style: WordArtStyle, window: &mut Window, cx: &mut Cx) {
    if !app.may(Intent::EditObject(None), cx) {
        return;
    }
    let sheet = app.sheet;
    let shape = style.shape(t("Your text here", "Votre texte ici"));
    let (width, height) = fitted_size(&shape, &app.fonts, cx.text_system());
    let (x, y) = middle_of_view(app, (width, height));
    let mut drawing = Drawing::shape(shape, (width, height));
    drawing.offset = (x as f32, y as f32);
    let Some(id) = app.wb.add_drawing(sheet, drawing) else {
        return;
    };
    app.commit(t("Insert WordArt", "Insérer un objet WordArt"), cx);
    app.select_object(Some(object_id(&Drawings, id)), cx);
    app.context_tab = Some(SHAPE_TAB);
    begin_text_edit(app, id, TextStart::All, window, cx);
}

// ---- the gallery ---------------------------------------------------------

/// The letter the gallery shows in a style.
fn sample_svg(style: WordArtStyle, side: f32, scale: f32) -> String {
    let (fill, effects) = style.look();
    let run = ArtRun {
        range: 0..1,
        family: "Helvetica".to_string(),
        bold: false,
        italic: false,
        underline: false,
        strike: false,
        color: fill,
        effects,
    };
    let size = side * 0.82;
    let block = ArtBlock {
        text: "A",
        rows: &[0..1],
        runs: std::slice::from_ref(&run),
        size,
        width: side,
        line_height: side,
        baseline: side * 0.8,
        align: gpui::TextAlign::Center,
        // Thin lines and close shadows, as on letters of 54 points.
        per_point: size / 40.0,
        turn: 0.0,
        pivot: (0.0, 0.0),
        scale,
    };
    block.svg().0
}

fn thumbnail(style: WordArtStyle, side: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, _| {
            let document = sample_svg(style, side, window.scale_factor());
            if let Some(image) = svg::image(&document, (false, false), 0) {
                // The image has room around the letter: centred on the box.
                let extra = px((size_of_image(&document) - side) / 2.0);
                let area = Bounds::new(
                    point(bounds.origin.x - extra, bounds.origin.y - extra),
                    size(bounds.size.width + extra * 2.0, bounds.size.height + extra * 2.0),
                );
                let _ = window.paint_image(area, Default::default(), image, 0, false);
            }
        },
    )
    .size(px(side))
    .flex_none()
}

/// The side of the square image a sample document draws, in pixels (its
/// view box).
fn size_of_image(document: &str) -> f32 {
    document
        .split_once("viewBox=\"0 0 ")
        .and_then(|(_, rest)| rest.split(' ').next())
        .and_then(|side| side.parse().ok())
        .unwrap_or(0.0)
}

/// The drop-down of Insert ▸ WordArt and of the WordArt Styles of the
/// Shape Format tab: the first inserts, the second restyles the text of
/// the selected shape.
struct Gallery {
    app: WeakEntity<ExcelApp>,
    restyle: bool,
}

impl Render for Gallery {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let buttons = WordArtStyle::all().map(|style| {
            let app = self.app.clone();
            let restyle = self.restyle;
            div()
                .id(("wordart-style", style.0 as usize))
                .size(px(58.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .border_1()
                .border_color(c(theme::BORDER))
                .bg(c(0xFFFFFF))
                .overflow_hidden()
                .hover(|s| s.border_color(c(theme::ACCENT)))
                .child(thumbnail(style, 46.0))
                .on_click(move |_, window, cx| {
                    app.update(cx, |app, cx| {
                        app.popup = None;
                        if restyle {
                            apply_style(app, style, cx);
                        } else {
                            insert(app, style, window, cx);
                        }
                        cx.notify();
                    })
                    .ok();
                })
        });
        div().w(px(5.0 * 58.0 + 4.0 * 4.0 + 16.0)).p(px(8.)).flex().flex_wrap().gap(px(4.)).children(buttons)
    }
}

pub(super) fn open_gallery(app: &mut ExcelApp, restyle: bool, window: &mut Window, cx: &mut Cx) {
    let weak = cx.weak_entity();
    tabs::open_dropdown(app, window, cx, |_| Gallery { app: weak, restyle });
}

// ---- outline and effects -------------------------------------------------

/// The drop-down of Text Outline: no outline, a colour, a weight.
struct OutlineMenu {
    app: WeakEntity<ExcelApp>,
}

impl Render for OutlineMenu {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self
            .app
            .upgrade()
            .and_then(|app| selected(app.read(cx)).and_then(Drawing::as_shape).map(|shape| shape.effects.outline));
        let apply = |app: &WeakEntity<ExcelApp>, cx: &mut App, change: Box<dyn Fn(&mut Option<TextOutline>)>| {
            app.update(cx, |app, cx| {
                app.popup = None;
                set_outline(app, cx, &*change);
                cx.notify();
            })
            .ok();
        };
        let app = self.app.clone();
        let grid = ColorGrid::new("wordart-outline", current.map(|outline| outline.map(|line| line.color)))
            .none_label(t("No Outline", "Sans contour"))
            .more(&self.app)
            .on_pick(move |color: &Option<Color>, _, cx| {
                let color = *color;
                apply(
                    &app,
                    cx,
                    Box::new(move |outline| match color {
                        Some(color) => outline.get_or_insert(TextOutline { color, width: 0.75 }).color = color,
                        None => *outline = None,
                    }),
                );
            });
        let weights = tabs::WEIGHTS.iter().enumerate().map(|(i, (points, label))| {
            let (points, app) = (*points, self.app.clone());
            let on = current.flatten().is_some_and(|line| line.width == points);
            div()
                .id(("wordart-weight", i))
                .h(px(22.))
                .px(px(6.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(3.))
                .when(on, |d| d.bg(c(theme::TOGGLED)))
                .hover(|s| s.bg(c(theme::HOVER)))
                .child(div().w(px(40.)).child(*label))
                .child(div().w(px(70.)).h(px(pt_to_px(points).max(1.0))).bg(c(0x404040)))
                .on_click(move |_, _, cx| {
                    apply(
                        &app,
                        cx,
                        Box::new(move |outline| outline.get_or_insert(TextOutline { color: Color::BLACK, width: points }).width = points),
                    );
                })
        });
        div().p(px(8.)).flex().text_size(px(12.)).child(grid).child(
            div()
                .ml(px(6.))
                .pl(px(6.))
                .border_l_1()
                .border_color(c(theme::BORDER))
                .flex()
                .flex_col()
                .child(div().h(px(20.)).px(px(6.)).text_color(c(theme::TEXT_MUTED)).child(t("Weight", "Épaisseur")))
                .children(weights),
        )
    }
}

pub(super) fn open_outline(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let weak = cx.weak_entity();
    tabs::open_dropdown(app, window, cx, |_| OutlineMenu { app: weak });
}

/// The shadows of the Shadow menu: Excel's outer ones, by the side they
/// fall on.
fn shadows() -> [(&'static str, f32, f32); 9] {
    [
        (t("Offset: Bottom Right", "Décalage : bas à droite"), 45.0, 3.0),
        (t("Offset: Bottom", "Décalage : bas"), 90.0, 3.0),
        (t("Offset: Bottom Left", "Décalage : bas à gauche"), 135.0, 3.0),
        (t("Offset: Right", "Décalage : droite"), 0.0, 3.0),
        (t("Offset: Center", "Décalage : centre"), 0.0, 0.0),
        (t("Offset: Left", "Décalage : gauche"), 180.0, 3.0),
        (t("Offset: Top Right", "Décalage : haut à droite"), 315.0, 3.0),
        (t("Offset: Top", "Décalage : haut"), 270.0, 3.0),
        (t("Offset: Top Left", "Décalage : haut à gauche"), 225.0, 3.0),
    ]
}

/// The Text Effects menu: the shadows. Transform (warped text) is shown
/// greyed out: it is not available.
pub(super) fn open_effects(app: &mut ExcelApp, window: &mut Window, cx: &mut Cx) {
    let current = selected(app).and_then(Drawing::as_shape).and_then(|shape| shape.effects.shadow);
    let mut entries = vec![
        MenuEntry::new(t("No Shadow", "Aucune ombre"), |app, _, cx| set_shadow(app, None, cx)).checked(current.is_none()),
        MenuEntry::separator(),
    ];
    for (name, angle, distance) in shadows() {
        let shadow = TextShadow {
            distance,
            ..TextShadow::offset(angle)
        };
        entries.push(MenuEntry::new(name, move |app, _, cx| set_shadow(app, Some(shadow), cx)).checked(current == Some(shadow)));
    }
    let entries = vec![
        MenuEntry::submenu(t("Shadow", "Ombre"), entries),
        MenuEntry::new(t("Transform", "Transformer"), |_, _, _| {}).disabled(true),
    ];
    app.open_command_menu(entries, 200.0, window, cx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::drawing::ShapeTool;

    fn run(range: Range<usize>, effects: TextEffects) -> ArtRun {
        ArtRun {
            range,
            family: "Aptos <Narrow>".to_string(),
            bold: true,
            italic: false,
            underline: true,
            strike: false,
            color: Color(0x156082),
            effects,
        }
    }

    fn block<'a>(text: &'a str, rows: &'a [Range<usize>], runs: &'a [ArtRun], turn: f32) -> ArtBlock<'a> {
        ArtBlock {
            text,
            rows,
            runs,
            size: 40.0,
            width: 300.0,
            line_height: 48.0,
            baseline: 38.0,
            align: gpui::TextAlign::Center,
            per_point: 2.0,
            turn,
            pivot: (150.0, 48.0),
            scale: 2.0,
        }
    }

    #[test]
    fn letters_are_drawn_with_their_outline_and_shadow() {
        let outlined = TextEffects {
            hollow: false,
            outline: Some(TextOutline {
                color: Color(0xFFFFFF),
                width: 1.0,
            }),
            shadow: Some(TextShadow::offset(45.0)),
            ..TextEffects::default()
        };
        let hollow = TextEffects {
            hollow: true,
            outline: Some(TextOutline {
                color: Color(0xC00000),
                width: 0.5,
            }),
            shadow: None,
            ..TextEffects::default()
        };
        let text = "Total & more \nsales";
        let rows = [0..13, 14..19];
        let runs = [run(0..5, outlined), run(5..19, hollow)];
        let (document, (left, top, width, height)) = block(text, &rows, &runs, 0.0).svg();
        // Room around the block for the shadow: 3 + 2 × 4 points at 2
        // pixels each, and a quarter of the size of the letters.
        assert_eq!((left, top, width, height), (-34.0, -34.0, 368.0, 164.0));
        assert!(document.contains(r#"width="736" height="328" viewBox="0 0 368 164""#), "{document}");
        // The shadow is drawn first, blurred and moved, and only under
        // the run that has one.
        let shadow = document.find(r#"filter="url(#s0)""#).expect("a shadow layer");
        let letters = document.find(r##"fill="#156082""##).expect("the letters");
        assert!(shadow < letters);
        assert!(document.contains(r#"<feGaussianBlur stdDeviation="4.00"/>"#));
        assert!(document.contains(r##"fill="#000000" fill-opacity="0.4""##));
        assert!(document.contains(r##"stroke="#FFFFFF" stroke-width="2.00""##));
        assert!(document.contains(r##"fill="none" stroke="#C00000" stroke-width="1.00""##));
        // Rows end without their blanks, text is escaped, fonts are named.
        assert!(document.contains("> &amp; more</tspan>"), "{document}");
        assert!(document.contains("font-family=\"'Aptos &lt;Narrow&gt;', Helvetica"));
        assert!(document.contains(r#"text-decoration="underline""#) && document.contains(r#"text-anchor="middle""#));
        assert_eq!(document.matches("<text ").count(), 4);
        // Turned, the image is the square that holds the block however it
        // is turned about the middle of its shape.
        let (turned, (left, top, width, height)) = block(text, &rows, &runs, 30.0).svg();
        assert!(turned.contains(r#"<g transform="rotate(30.00 150.00 48.00)">"#));
        assert_eq!(width, height);
        assert!((left + width / 2.0 - 150.0).abs() < 0.01 && (top + height / 2.0 - 48.0).abs() < 0.01);
        assert!(width >= 2.0 * (184.0f32).hypot(82.0));
    }

    #[test]
    fn home_tab_commands_reach_the_selected_characters() {
        let mut shape = ShapeTool::Rectangle.shape();
        shape.text = "Total sales".to_string();
        restyle_shape(&mut shape, 0..5, &|style| style.font.bold = true);
        assert_eq!(shape.runs.len(), 2);
        assert!(shape.runs[0].font.bold && !shape.runs[1].font.bold);
        // Alignment and fill are the shape's, whatever is selected.
        restyle_shape(&mut shape, 6..11, &|style| {
            style.h_align = opensheet_engine::style::HAlign::Right;
            style.fill = Some(Color(0xFFFF00));
        });
        assert_eq!((shape.align, shape.fill), (opensheet_engine::drawing::TextAlign::Right, Some(Color(0xFFFF00))));
        assert_eq!(shape.runs.len(), 2);
        // Without a selection the whole text takes the change and the
        // runs keep what tells them apart.
        restyle_shape(&mut shape, 0..0, &|style| style.font.set_size_pt(20.0));
        assert!(shape.runs.iter().all(|run| run.font.size_pt() == 20.0));
        assert!(shape.runs[0].font.bold && !shape.runs[1].font.bold);
        // The samples of the gallery are documents of their own.
        let sample = sample_svg(WordArtStyle(3), 46.0, 2.0);
        assert!(sample.contains(">A</tspan>") && sample.contains("stroke="));
        assert!(size_of_image(&sample) > 46.0);
    }
}
