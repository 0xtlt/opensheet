//! The text of a shape, run by run: what `a:rPr` says about the letters
//! (font, fill, outline, shadow) and how the body holds its lines
//! (`wrap` of `a:bodyPr`). WordArt is nothing else: a text box whose runs
//! have an outline and a shadow, and whose text is not wrapped. A warp of
//! the whole text (Transform) is `a:prstTxWarp` of the body.

use super::*;
use opensheet_engine::textwarp::{TextWarp, WarpPreset};
use opensheet_engine::wordart::{
    GradientKind, GradientStop, TextEffects, TextGradient, TextOutline, TextReflection, TextRun, TextShadow,
};

// ---- writing -------------------------------------------------------------------

/// The properties of a run (`a:rPr`), or of the end of a paragraph that
/// has none (`a:endParaRPr`), in the order the schema wants them: line,
/// fill, effects, fonts.
fn push_run_properties(x: &mut String, element: &str, font: &Font, effects: &TextEffects) {
    w!(x, "<a:{element} sz=\"{}\"", font.size as u32 * 50);
    if font.bold {
        x.push_str(" b=\"1\"");
    }
    if font.italic {
        x.push_str(" i=\"1\"");
    }
    match font.underline {
        Underline::None => {}
        Underline::Single => x.push_str(" u=\"sng\""),
        Underline::Double => x.push_str(" u=\"dbl\""),
    }
    if font.strike {
        x.push_str(" strike=\"sngStrike\"");
    }
    x.push('>');
    if let Some(outline) = &effects.outline {
        let width = (outline.width.max(0.0) as f64 * EMU_PER_PT).round() as i64;
        w!(x, "<a:ln w=\"{width}\">");
        push_solid_fill(x, outline.color);
        x.push_str("<a:prstDash val=\"solid\"/></a:ln>");
    }
    if effects.hollow {
        x.push_str("<a:noFill/>");
    } else if let Some(gradient) = &effects.gradient {
        push_gradient(x, gradient);
    } else if let Some(color) = font.color {
        push_solid_fill(x, color);
    }
    let emu = |points: f32| (points.max(0.0) as f64 * EMU_PER_PT).round() as i64;
    if effects.shadow.is_some() || effects.reflection.is_some() {
        x.push_str("<a:effectLst>");
    }
    if let Some(shadow) = &effects.shadow {
        let angle = ((shadow.angle as f64).rem_euclid(360.0) * ANGLE_UNITS).round() as i64;
        w!(
            x,
            "<a:outerShdw blurRad=\"{}\" dist=\"{}\" dir=\"{angle}\" algn=\"tl\" rotWithShape=\"0\">",
            emu(shadow.blur),
            emu(shadow.distance)
        );
        let alpha = (shadow.opacity < 1.0).then(|| (shadow.opacity.clamp(0.0, 1.0) as f64 * 100_000.0).round() as u32);
        push_drawing_color(x, shadow.color, alpha);
        x.push_str("</a:outerShdw>");
    }
    if let Some(reflection) = &effects.reflection {
        // As Excel writes its Reflection Variations: the text mirrored
        // about its bottom edge, fading to nearly nothing.
        let percent = |part: f32| (part.clamp(0.0, 1.0) as f64 * 100_000.0).round() as u32;
        w!(
            x,
            "<a:reflection blurRad=\"{}\" stA=\"{}\" endA=\"300\" endPos=\"{}\"",
            emu(reflection.blur),
            percent(reflection.opacity),
            percent(reflection.reach)
        );
        if reflection.distance > 0.0 {
            w!(x, " dist=\"{}\"", emu(reflection.distance));
        }
        x.push_str(" dir=\"5400000\" sy=\"-100000\" algn=\"bl\" rotWithShape=\"0\"/>");
    }
    if effects.shadow.is_some() || effects.reflection.is_some() {
        x.push_str("</a:effectLst>");
    }
    for script in ["latin", "cs"] {
        w!(x, "<a:{script} typeface=\"");
        push_attr(x, &font.name);
        x.push_str("\"/>");
    }
    w!(x, "</a:{element}>");
}

/// `<a:gradFill>` as Excel writes the gradients of its galleries.
fn push_gradient(x: &mut String, gradient: &TextGradient) {
    x.push_str("<a:gradFill flip=\"none\" rotWithShape=\"1\"><a:gsLst>");
    for stop in gradient.stops() {
        w!(x, "<a:gs pos=\"{}\">", (stop.at.clamp(0.0, 1.0) as f64 * 100_000.0).round() as u32);
        push_drawing_color(x, stop.color, None);
        x.push_str("</a:gs>");
    }
    x.push_str("</a:gsLst>");
    match gradient.kind {
        GradientKind::Linear(angle) => {
            let angle = ((angle as f64).rem_euclid(360.0) * ANGLE_UNITS).round() as i64;
            w!(x, "<a:lin ang=\"{angle}\" scaled=\"1\"/>");
        }
        GradientKind::FromCentre => {
            x.push_str("<a:path path=\"circle\"><a:fillToRect l=\"50000\" t=\"50000\" r=\"50000\" b=\"50000\"/></a:path>");
        }
    }
    x.push_str("<a:tileRect/></a:gradFill>");
}

/// `<a:prstTxWarp>`: the warp and the adjustments that are not the
/// preset's own (Excel writes an empty list for those).
fn push_warp(x: &mut String, warp: &TextWarp) {
    w!(x, "<a:prstTxWarp prst=\"{}\">", warp.preset.ooxml_name());
    let own = warp.preset.adjustments();
    let changed: Vec<(&str, i32)> = warp.adjustments().into_iter().filter(|adjustment| !own.contains(adjustment)).collect();
    if changed.is_empty() {
        x.push_str("<a:avLst/>");
    } else {
        x.push_str("<a:avLst>");
        // A list that gives one of two adjustments gives both.
        for (name, value) in warp.adjustments() {
            w!(x, "<a:gd name=\"{name}\" fmla=\"val {value}\"/>");
        }
        x.push_str("</a:avLst>");
    }
    x.push_str("</a:prstTxWarp>");
}

/// `<xdr:txBody>`: a paragraph for each line of the text, a run for each
/// change of look inside it.
pub(super) fn push_text_body(x: &mut String, shape: &Shape, equation: bool) {
    if shape.wrap {
        w!(
            x,
            "<xdr:txBody><a:bodyPr vertOverflow=\"clip\" horzOverflow=\"clip\" wrap=\"square\" rtlCol=\"0\" anchor=\"{}\"",
            shape.anchor.ooxml_name()
        );
        match &shape.warp {
            Some(warp) => {
                x.push('>');
                push_warp(x, warp);
                x.push_str("</a:bodyPr><a:lstStyle/>");
            }
            None => x.push_str("/><a:lstStyle/>"),
        }
    } else {
        // As Excel writes WordArt: one line, and a box that fits it. A
        // warped text fills its box instead, whatever size it is given.
        w!(
            x,
            "<xdr:txBody><a:bodyPr wrap=\"none\" lIns=\"91440\" tIns=\"45720\" rIns=\"91440\" bIns=\"45720\" rtlCol=\"0\" anchor=\"{}\">",
            shape.anchor.ooxml_name()
        );
        match &shape.warp {
            Some(warp) => {
                push_warp(x, warp);
                x.push_str("<a:noAutofit/>");
            }
            None => x.push_str("<a:spAutoFit/>"),
        }
        x.push_str("</a:bodyPr><a:lstStyle/>");
    }
    if let (true, Some(math)) = (equation, &shape.math) {
        w!(x, "<a:p><a:pPr algn=\"{}\"/>", shape.align.ooxml_name());
        super::math::push_paragraph(x, math, &shape.font);
        x.push_str("</a:p></xdr:txBody>");
        return;
    }
    let runs = shape.text_runs();
    let mut start = 0;
    for paragraph in shape.text.split('\n') {
        let end = start + paragraph.len();
        w!(x, "<a:p><a:pPr algn=\"{}\"/>", shape.align.ooxml_name());
        let mut last = None;
        for (covered, font, effects) in &runs {
            let (from, to) = (covered.start.max(start), covered.end.min(end));
            // The look at the end of the paragraph: that of the run that
            // reaches it.
            if covered.start <= end && covered.end >= start {
                last = Some((font, effects));
            }
            if from >= to {
                continue;
            }
            x.push_str("<a:r>");
            push_run_properties(x, "rPr", font, effects);
            x.push_str("<a:t>");
            push_text(x, &shape.text[from..to]);
            x.push_str("</a:t></a:r>");
        }
        if paragraph.is_empty() {
            let (font, effects) = last.unwrap_or((&shape.font, &shape.effects));
            push_run_properties(x, "endParaRPr", font, effects);
        }
        x.push_str("</a:p>");
        start = end + 1;
    }
    x.push_str("</xdr:txBody>");
}

// ---- reading -------------------------------------------------------------------

/// The properties of a run as a file has them.
pub(super) struct RawRun {
    font: Font,
    effects: TextEffects,
    /// The run gives its colour itself.
    colored: bool,
}

/// `<a:effectLst>`: the outer shadow and the reflection, the effects that
/// are kept.
fn parse_effects<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, palette: &Palette) -> (Option<TextShadow>, Option<TextReflection>) {
    let mut shadow = None;
    let mut reflection = None;
    each_child(r, tag, |r, t| {
        if t.name == "reflection" {
            let points = |name: &str| t.attr_f64(name).filter(|v| v.is_finite() && *v >= 0.0).map_or(0.0, |v| (v / EMU_PER_PT).min(400.0) as f32);
            let part = |name: &str, default: f32| {
                t.attr_f64(name).filter(|v| v.is_finite()).map_or(default, |v| (v / 100_000.0).clamp(0.0, 1.0) as f32)
            };
            reflection = Some(TextReflection {
                blur: points("blurRad"),
                opacity: part("stA", 1.0),
                reach: part("endPos", 1.0),
                distance: points("dist"),
            });
            r.skip(t);
            return;
        }
        if t.name != "outerShdw" {
            r.skip(t);
            return;
        }
        let points = |name: &str| t.attr_f64(name).filter(|v| v.is_finite() && *v >= 0.0).map_or(0.0, |v| (v / EMU_PER_PT).min(400.0) as f32);
        let (blur, distance) = (points("blurRad"), points("dist"));
        let angle = t.attr_f64("dir").filter(|v| v.is_finite()).map_or(0.0, |v| (v / ANGLE_UNITS).rem_euclid(360.0) as f32);
        // The colour (which may be one of the theme), then how much of
        // it shows: the element is read once for each.
        let color = parse_color(&mut r.fork(), t, palette);
        let mut opacity = 1.0f32;
        each_child(r, t, |r, c| {
            each_child(r, c, |r, adjustment| {
                if adjustment.name == "alpha" {
                    if let Some(alpha) = adjustment.attr_f64("val").filter(|v| v.is_finite()) {
                        opacity = (alpha / 100_000.0).clamp(0.0, 1.0) as f32;
                    }
                }
                r.skip(adjustment);
            });
        });
        shadow = Some(TextShadow {
            color: color.unwrap_or(Color::BLACK),
            opacity,
            blur,
            distance,
            angle,
        });
    });
    (shadow, reflection)
}

/// `<a:gradFill>`: its colours and the way it runs. A gradient along a
/// shape (`rect` or `shape` paths) is read as one from the middle.
fn parse_gradient<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, palette: &Palette) -> Option<TextGradient> {
    let mut stops = Vec::new();
    let mut kind = GradientKind::Linear(0.0);
    each_child(r, tag, |r, t| match t.name {
        "gsLst" => each_child(r, t, |r, stop| {
            let at = stop.attr_f64("pos").filter(|v| v.is_finite()).map_or(0.0, |v| (v / 100_000.0).clamp(0.0, 1.0) as f32);
            if let Some(color) = parse_color(r, stop, palette) {
                stops.push(GradientStop { at, color });
            }
        }),
        "lin" => {
            let angle = t.attr_f64("ang").filter(|v| v.is_finite()).map_or(0.0, |v| (v / ANGLE_UNITS).rem_euclid(360.0) as f32);
            kind = GradientKind::Linear(angle);
            r.skip(t);
        }
        "path" => {
            kind = GradientKind::FromCentre;
            r.skip(t);
        }
        _ => r.skip(t),
    });
    TextGradient::new(kind, &stops)
}

fn parse_run_properties<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, palette: &Palette) -> RawRun {
    let mut font = Font::default();
    if let Some(size) = tag.attr_f64("sz").filter(|size| size.is_finite()) {
        font.size = (size / 50.0).round().clamp(2.0, 818.0) as u16;
    }
    font.bold = tag.attr_bool("b").unwrap_or(false);
    font.italic = tag.attr_bool("i").unwrap_or(false);
    font.underline = match tag.attr("u").as_deref() {
        None | Some("none") => Underline::None,
        Some("dbl" | "wavyDbl") => Underline::Double,
        Some(_) => Underline::Single,
    };
    font.strike = matches!(tag.attr("strike").as_deref(), Some("sngStrike" | "dblStrike"));
    let mut paint = Paint::Unset;
    let mut effects = TextEffects::default();
    each_child(r, tag, |r, t| match t.name {
        "latin" => {
            // "+mn-lt" and the like name a font of the theme.
            if let Some(name) = t.attr("typeface").filter(|name| !name.is_empty() && !name.starts_with('+')) {
                font.name = Arc::from(name.as_ref());
            }
            r.skip(t);
        }
        // A line without a colour of its own is no outline (`<a:ln w="0"/>`
        // is how Excel says "none" in its WordArt styles).
        "ln" => {
            let line = parse_line(r, t, palette);
            if let Paint::Color(color) = line.paint {
                effects.outline = Some(TextOutline {
                    color,
                    width: line.width.unwrap_or(DEFAULT_LINE_PT),
                });
            }
        }
        "effectLst" => (effects.shadow, effects.reflection) = parse_effects(r, t, palette),
        // A gradient is read twice: for itself, and for the one colour
        // of who draws no gradient (its first).
        "gradFill" => {
            effects.gradient = parse_gradient(&mut r.fork(), t, palette);
            parse_paint(r, t, palette, &mut paint);
        }
        _ => {
            if matches!(t.name, "solidFill" | "noFill" | "pattFill" | "blipFill") {
                effects.gradient = None;
            }
            parse_paint(r, t, palette, &mut paint)
        }
    });
    match paint {
        Paint::Color(color) => font.color = Some(color),
        Paint::Nothing => effects.hollow = true,
        Paint::Unset => {}
    }
    RawRun {
        font,
        effects,
        colored: !matches!(paint, Paint::Unset),
    }
}

/// `<xdr:txBody>` as a file has it.
#[derive(Default)]
pub(super) struct RawText {
    text: String,
    /// The runs that have text, with their length in `text`.
    runs: Vec<(usize, RawRun)>,
    /// The properties of the end of the first paragraph: the look of a
    /// text that has no run.
    end: Option<RawRun>,
    pub align: Option<TextAlign>,
    pub anchor: Option<TextAnchor>,
    /// `wrap="none"`.
    unwrapped: bool,
    /// `a:prstTxWarp` of the body.
    warp: Option<TextWarp>,
    /// The equation of a paragraph that holds one, with the size and the
    /// colour of its letters.
    math: Option<(opensheet_engine::math::Equation, Option<RawRun>)>,
}

impl RawText {
    /// Text that belongs to the run before it (a line break).
    fn extend(&mut self, text: &str) {
        self.text.push_str(text);
        if let Some((len, _)) = self.runs.last_mut() {
            *len += text.len();
        }
    }

    /// Put the text into a shape. `automatic` is the colour the style of
    /// the shape gives runs that name none.
    pub fn into_shape(self, shape: &mut Shape, automatic: Option<Color>) {
        shape.wrap = !self.unwrapped;
        shape.warp = self.warp;
        shape.text = self.text;
        let resolve = |run: RawRun| {
            let mut font = run.font;
            if !run.colored {
                font.color = automatic;
            }
            (font, run.effects)
        };
        let mut runs: Vec<TextRun> = Vec::new();
        for (len, run) in self.runs {
            let (font, effects) = resolve(run);
            runs.push(TextRun { len, font, effects });
        }
        match runs.first() {
            Some(first) => {
                shape.font = first.font.clone();
                shape.effects = first.effects;
            }
            // Without text, a shape has the look its paragraph ends in,
            // or that of a new shape.
            None => {
                if let Some(end) = self.end {
                    (shape.font, shape.effects) = resolve(end);
                }
            }
        }
        shape.runs = runs;
        // Neighbours of one look are one run, and a single look is none.
        shape.format_text(0..0, &|_, _| {});
        // An equation: its text is the equation in the linear format.
        if let Some((equation, look)) = self.math {
            if let Some(look) = look {
                let name = shape.font.name.clone();
                (shape.font, _) = resolve(look);
                // The font equations are set in is not the shape's.
                shape.font.name = name;
                shape.font.italic = false;
            }
            shape.runs.clear();
            shape.text = equation.to_linear();
            shape.math = Some(equation);
        }
    }
}

fn parse_paragraph<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, palette: &Palette, out: &mut RawText) {
    each_child(r, tag, |r, t| match t.name {
        "pPr" => {
            if out.align.is_none() {
                out.align = Some(t.attr("algn").and_then(|name| TextAlign::from_ooxml_name(&name)).unwrap_or_default());
            }
            r.skip(t);
        }
        // A run, or a field shown as the text it had when saved.
        "r" | "fld" => {
            let mut run = None;
            let mut text = String::new();
            each_child(r, t, |r, c| match c.name {
                "rPr" => run = Some(parse_run_properties(r, c, palette)),
                "t" => text.push_str(&printable(&r.text(c), true)),
                _ => r.skip(c),
            });
            if text.is_empty() {
                return;
            }
            out.text.push_str(&text);
            let run = run.unwrap_or(RawRun {
                font: Font::default(),
                effects: TextEffects::default(),
                colored: false,
            });
            out.runs.push((text.len(), run));
        }
        "br" => {
            out.extend("\n");
            r.skip(t);
        }
        // `a14:m`: an equation. The look of its first run is that of its
        // letters.
        "m" if out.math.is_none() => {
            let mut look = None;
            let mut scan = r.fork();
            find_run_properties(&mut scan, t, palette, &mut look, 0);
            out.math = Some((super::math::parse_equation(r, t), look));
        }
        "endParaRPr" => {
            let run = parse_run_properties(r, t, palette);
            out.end.get_or_insert(run);
        }
        _ => r.skip(t),
    });
}

/// The first `a:rPr` under an element, however deep.
fn find_run_properties<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, palette: &Palette, found: &mut Option<RawRun>, depth: usize) {
    each_child(r, tag, |r, t| {
        if found.is_none() && t.name == "rPr" && t.attr("sz").is_some() {
            *found = Some(parse_run_properties(r, t, palette));
        } else if found.is_none() && depth < 24 {
            find_run_properties(r, t, palette, found, depth + 1);
        } else {
            r.skip(t);
        }
    });
}

/// `<a:prstTxWarp>`; `textNoShape` and presets that do not exist are no
/// warp.
fn parse_warp<'a>(r: &mut Reader<'a>, tag: &Tag<'a>) -> Option<TextWarp> {
    let mut warp = tag.attr("prst").and_then(|name| WarpPreset::from_ooxml_name(&name)).map(TextWarp::from);
    each_child(r, tag, |r, list| {
        each_child(r, list, |r, guide| {
            // `fmla="val 12345"`.
            let value = guide.attr("fmla").and_then(|formula| formula.trim().strip_prefix("val")?.trim().parse::<f64>().ok());
            if let (Some(warp), Some(name), Some(value)) = (warp.as_mut(), guide.attr("name"), value.filter(|v| v.is_finite())) {
                warp.set_adjustment(&name, value.clamp(i32::MIN as f64, i32::MAX as f64) as i32);
            }
            r.skip(guide);
        });
    });
    warp
}

pub(super) fn parse_text<'a>(r: &mut Reader<'a>, tag: &Tag<'a>, palette: &Palette) -> RawText {
    let mut out = RawText::default();
    let mut paragraphs = 0;
    each_child(r, tag, |r, t| match t.name {
        "bodyPr" => {
            out.anchor = Some(t.attr("anchor").and_then(|name| TextAnchor::from_ooxml_name(&name)).unwrap_or_default());
            out.unwrapped = t.attr("wrap").as_deref() == Some("none");
            each_child(r, t, |r, c| {
                if c.name == "prstTxWarp" {
                    out.warp = parse_warp(r, c);
                } else {
                    r.skip(c);
                }
            });
        }
        "p" => {
            if paragraphs > 0 {
                out.extend("\n");
            }
            paragraphs += 1;
            parse_paragraph(r, t, palette, &mut out);
        }
        _ => r.skip(t),
    });
    out
}
