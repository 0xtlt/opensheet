//! Equations: how the structures of Office Math are laid out and drawn,
//! Insert ▸ Equation, and the Equation tab with its galleries.
//!
//! An equation is the text of a shape (`Shape::math`). It is drawn built
//! up ("Professional"): each structure is a box with a width, a height
//! above its baseline and a depth below, made of the boxes of its parts
//! (`layout`). Glyphs are text in a mathematical font; fraction bars,
//! radical signs and tall delimiters are drawn as lines.
//!
//! An equation is typed in the linear format, in the shape, like any
//! text of a shape; the built-up result is shown under the shape while it
//! is typed and takes its place when typing ends. The galleries insert
//! the linear form of a structure or a symbol where the caret is.
//!
//! Not done: a caret that moves inside the built-up equation (Excel edits
//! equations in place, structure by structure), equations inside other
//! text, turned equations (an equation in a turned shape is drawn level),
//! and printing (a printed equation is its linear text).

use super::geometry::{Placed, Polyline};
use super::paint::{self, Brush};
use super::{Cmd, DrawingUi, Drawings, EQUATION_TAB, begin_text_edit, selected, tabs, TextStart};
use crate::app::ExcelApp;
use crate::features::{Command, Cx, Intent, object_id};
use crate::grid::{Draw, FontResolver, Layer, pt_to_px};
use crate::i18n::t;
use crate::ribbon::{group, row, rows};
use crate::theme::{self, c, engine_color};
use crate::view::tooltip;
use gpui::{
    AnyElement, Bounds, Context, EntityInputHandler, Font, FontStyle, FontWeight, Hsla, Pixels, SharedString, TextRun, TextSystem,
    WeakEntity, Window, canvas, div, point, prelude::*, px,
};
use opensheet_engine::drawing::{Drawing, Shape, ShapeTool, TEXT_MARGIN, TextAlign, TextAnchor};
use opensheet_engine::math::{APPLY, BUILT_IN, Equation, Math, Row, SYMBOLS, Structure, tokens};

// ---- boxes ---------------------------------------------------------------

/// Something drawn in a box. Positions are from the left end of the
/// baseline of the box, y growing downwards.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Ink {
    Glyphs { text: String, size: f32, italic: bool, x: f32, y: f32 },
    /// A filled bar: its top-left corner, its width and its thickness.
    Rule { x: f32, y: f32, width: f32, height: f32 },
    /// A line through points.
    Stroke { points: Vec<(f32, f32)>, width: f32 },
}

/// A laid-out piece of an equation.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct MathBox {
    pub width: f32,
    /// How far it reaches above its baseline, and below.
    pub ascent: f32,
    pub descent: f32,
    pub ink: Vec<Ink>,
}

impl MathBox {
    fn space(width: f32) -> MathBox {
        MathBox {
            width,
            ..MathBox::default()
        }
    }

    /// The box moved right and down.
    fn moved(mut self, dx: f32, dy: f32) -> MathBox {
        for ink in &mut self.ink {
            match ink {
                Ink::Glyphs { x, y, .. } | Ink::Rule { x, y, .. } => {
                    *x += dx;
                    *y += dy;
                }
                Ink::Stroke { points, .. } => {
                    for point in points {
                        point.0 += dx;
                        point.1 += dy;
                    }
                }
            }
        }
        self.ascent -= dy;
        self.descent += dy;
        self
    }

    /// Put `other` in this box, `dx` right of its left end and `dy` under
    /// its baseline; the box grows to hold it.
    fn place(&mut self, other: MathBox, dx: f32, dy: f32) {
        let other = other.moved(dx, dy);
        self.width = self.width.max(dx + other.width);
        self.ascent = self.ascent.max(other.ascent);
        self.descent = self.descent.max(other.descent);
        self.ink.extend(other.ink);
    }

    /// Put `other` after this box, on its baseline.
    fn append(&mut self, other: MathBox) {
        let at = self.width;
        self.place(other, at, 0.0);
    }

    fn height(&self) -> f32 {
        self.ascent + self.descent
    }
}

/// What laying out needs to know of a font.
pub(super) trait Measure {
    fn width(&self, text: &str, size: f32, italic: bool) -> f32;
    /// How far letters reach above the baseline and below, at a size.
    fn extent(&self, size: f32) -> (f32, f32);
}

/// The fonts of the system, measured by the advances of their characters.
pub(super) struct Fonts<'a> {
    pub ts: &'a TextSystem,
    pub family: SharedString,
}

impl Fonts<'_> {
    fn font(&self, italic: bool) -> Font {
        Font {
            family: self.family.clone(),
            features: Default::default(),
            fallbacks: None,
            weight: FontWeight::NORMAL,
            style: if italic { FontStyle::Italic } else { FontStyle::Normal },
        }
    }
}

/// Slanted letters in a mathematical font: such fonts have no italic
/// face, they hold the italic alphabets as characters of their own
/// (U+1D434…). `None` for a font that slants its letters itself.
fn math_italic(family: &str, text: &str) -> Option<String> {
    if !family.contains("Math") {
        return None;
    }
    let mapped = |ch: char| -> char {
        let at = |base: u32, first: char| char::from_u32(base + (ch as u32 - first as u32)).unwrap_or(ch);
        match ch {
            // The italic h is Planck's constant, older than the block.
            'h' => 'ℎ',
            'a'..='z' => at(0x1D44E, 'a'),
            'A'..='Z' => at(0x1D434, 'A'),
            'α'..='ω' => at(0x1D6FC, 'α'),
            'ϵ' => '\u{1D716}',
            'ϑ' => '\u{1D717}',
            'ϕ' => '\u{1D719}',
            other => other,
        }
    };
    Some(text.chars().map(mapped).collect())
}

impl Measure for Fonts<'_> {
    fn width(&self, text: &str, size: f32, italic: bool) -> f32 {
        let slanted = if italic { math_italic(&self.family, text) } else { None };
        let (text, italic) = match &slanted {
            Some(slanted) => (slanted.as_str(), false),
            None => (text, italic),
        };
        let id = self.ts.resolve_font(&self.font(italic));
        text.chars()
            .map(|ch| self.ts.advance(id, px(size), ch).map_or(size * 0.5, |advance| f32::from(advance.width)))
            .sum()
    }

    fn extent(&self, size: f32) -> (f32, f32) {
        let id = self.ts.resolve_font(&self.font(false));
        (f32::from(self.ts.ascent(id, px(size))), f32::from(self.ts.descent(id, px(size))).abs())
    }
}

/// The font equations are set in: the first of these the system has.
pub(super) fn math_family(fonts: &FontResolver) -> SharedString {
    ["Cambria Math", "STIX Two Math", "STIXGeneral", "Times New Roman"]
        .into_iter()
        .find(|name| fonts.names.iter().any(|known| known == name))
        .unwrap_or("Times New Roman")
        .into()
}

// ---- layout --------------------------------------------------------------

/// Height of the axis fraction bars and operators are centred on, above
/// the baseline; thickness of rules. In sizes of the text.
const AXIS: f32 = 0.26;
const RULE: f32 = 0.055;
/// Scripts and limits are this much smaller than what they hang on.
const SCRIPT: f32 = 0.7;

fn is_relation(ch: char) -> bool {
    "+−±∓×÷⋅=≠≤≥<>≈≡∼→←↔⇒⇐⇔∈∉⊂⊆⊃⊇∪∩∝≅≪≫∘∗↦≔≝≜≞".contains(ch)
}

#[derive(Clone, Copy)]
struct Style {
    size: f32,
    /// Never smaller than this (scripts of scripts stay readable).
    floor: f32,
    /// Letters stand upright: the name of a function.
    upright: bool,
    /// Inside a fraction: a fraction here is set smaller.
    nested: bool,
}

impl Style {
    fn script(self) -> Style {
        Style {
            size: (self.size * SCRIPT).max(self.floor),
            nested: true,
            ..self
        }
    }
}

struct Layout<'a> {
    fonts: &'a dyn Measure,
}

impl Layout<'_> {
    fn glyphs(&self, text: &str, style: Style, italic: bool) -> MathBox {
        let (ascent, descent) = self.fonts.extent(style.size);
        MathBox {
            width: self.fonts.width(text, style.size, italic),
            ascent,
            descent,
            ink: vec![Ink::Glyphs {
                text: text.to_string(),
                size: style.size,
                italic,
                x: 0.0,
                y: 0.0,
            }],
        }
    }

    fn text(&self, text: &str, style: Style, first: bool) -> MathBox {
        let em = style.size;
        if text.chars().eq([APPLY]) {
            return MathBox::space(em * 0.12);
        }
        // A hyphen typed for a minus is drawn as one.
        let shown = if text == "-" { "−" } else { text };
        let mut chars = shown.chars();
        let (one, single) = (chars.next().unwrap_or(' '), chars.next().is_none());
        // Variables slant; numbers, signs, capital Greek and names do not.
        let italic = !style.upright && shown.chars().all(|ch| ch.is_alphabetic() && !('Α'..='Ω').contains(&ch));
        let glyphs = self.glyphs(shown, style, italic);
        let pad = if single && is_relation(one) && !first && !style.nested {
            em * 0.22
        } else if single && is_relation(one) && !first {
            em * 0.08
        } else {
            0.0
        };
        let mut out = MathBox::space(pad);
        out.append(glyphs);
        out.width += pad + if single && matches!(one, ',' | ';') { em * 0.17 } else { 0.0 };
        out
    }

    fn row(&self, nodes: &Row, style: Style) -> MathBox {
        let mut out = MathBox::default();
        for (i, node) in nodes.iter().enumerate() {
            out.append(self.node(node, style, i == 0));
        }
        if nodes.is_empty() {
            // An empty place still has the height of a line.
            let (ascent, descent) = self.fonts.extent(style.size);
            out.ascent = ascent;
            out.descent = descent;
        }
        out
    }

    fn rule(x: f32, y: f32, width: f32, height: f32) -> Ink {
        Ink::Rule { x, y, width, height }
    }

    /// `over` centred above `base` (or `under` below it), a gap apart.
    fn stacked(base: MathBox, over: Option<MathBox>, under: Option<MathBox>, gap: f32) -> MathBox {
        let width = [Some(&base), over.as_ref(), under.as_ref()].into_iter().flatten().map(|b| b.width).fold(0.0, f32::max);
        let mut out = MathBox::default();
        let (ascent, descent) = (base.ascent, base.descent);
        out.place(base.clone(), (width - base.width) / 2.0, 0.0);
        if let Some(over) = over {
            let dy = -(ascent + gap + over.descent);
            out.place(over.clone(), (width - over.width) / 2.0, dy);
        }
        if let Some(under) = under {
            let dy = descent + gap + under.ascent;
            out.place(under.clone(), (width - under.width) / 2.0, dy);
        }
        out.width = width;
        out
    }

    /// Scripts beside a base: the superscript raised, the subscript
    /// lowered.
    fn scripted(&self, base: MathBox, sub: Option<MathBox>, sup: Option<MathBox>, em: f32) -> MathBox {
        let mut out = base.clone();
        let x = base.width + em * 0.04;
        if let Some(sup) = sup {
            let up = (em * 0.42).max(base.ascent - em * 0.3) + sup.descent * 0.3;
            out.place(sup, x, -up);
        }
        if let Some(sub) = sub {
            let down = (em * 0.2).max(base.descent + em * 0.08) + sub.ascent * 0.25;
            out.place(sub, x, down);
        }
        out.width += em * 0.03;
        out
    }

    /// A delimiter as tall as what it holds: lines from `top` to `bottom`.
    fn delimiter(ch: char, top: f32, bottom: f32, em: f32) -> MathBox {
        let width = em * 0.3;
        let line = em * RULE * 1.1;
        let middle = (top + bottom) / 2.0;
        let (near, far) = (width * 0.2, width * 0.85);
        // A curve from the far top through the near middle to the far
        // bottom: a round bracket opening to the right.
        let curve = |from: (f32, f32), through: (f32, f32), to: (f32, f32)| -> Vec<(f32, f32)> {
            (0..=12)
                .map(|i| {
                    let t = i as f32 / 12.0;
                    let (a, b, c) = ((1.0 - t) * (1.0 - t), 2.0 * t * (1.0 - t), t * t);
                    (a * from.0 + b * through.0 + c * to.0, a * from.1 + b * through.1 + c * to.1)
                })
                .collect()
        };
        let mirrored = matches!(ch, ')' | ']' | '}' | '⟩' | '⌋' | '⌉');
        let mut strokes: Vec<Vec<(f32, f32)>> = match ch {
            '(' | ')' => vec![curve((far, top), (near - width * 0.5, middle), (far, bottom))],
            '[' | ']' => vec![vec![(far, top), (near, top), (near, bottom), (far, bottom)]],
            '⌊' | '⌋' => vec![vec![(near, top), (near, bottom), (far, bottom)]],
            '⌈' | '⌉' => vec![vec![(far, top), (near, top), (near, bottom)]],
            '⟨' | '⟩' => vec![vec![(far, top), (near, middle), (far, bottom)]],
            '{' | '}' => {
                let nib = width * 0.5;
                let mut points = curve((far, top), (nib, top), (nib, top + em * 0.25));
                points.extend(curve((nib, middle - em * 0.2), (nib, middle), (near - width * 0.1, middle)));
                points.extend(curve((near - width * 0.1, middle), (nib, middle), (nib, middle + em * 0.2)));
                points.extend(curve((nib, bottom - em * 0.25), (nib, bottom), (far, bottom)));
                vec![points]
            }
            '‖' => vec![vec![(width * 0.3, top), (width * 0.3, bottom)], vec![(width * 0.7, top), (width * 0.7, bottom)]],
            _ => vec![vec![(width * 0.5, top), (width * 0.5, bottom)]],
        };
        if mirrored {
            for point in strokes.iter_mut().flatten() {
                point.0 = width - point.0;
            }
        }
        MathBox {
            width,
            ascent: -top,
            descent: bottom,
            ink: strokes.into_iter().map(|points| Ink::Stroke { points, width: line }).collect(),
        }
    }

    fn node(&self, node: &Math, style: Style, first: bool) -> MathBox {
        let em = style.size;
        let axis = -AXIS * em;
        let rule = (RULE * em).max(0.75);
        match node {
            Math::Text(text) => self.text(text, style, first),
            Math::Fraction { num, den, bar } => {
                let inner = if style.nested { style.script() } else { Style { nested: true, ..style } };
                let (num, den) = (self.row(num, inner), self.row(den, inner));
                let gap = em * 0.14;
                let width = num.width.max(den.width) + em * 0.24;
                let mut out = MathBox::space(em * 0.08);
                let left = out.width;
                out.place(num.clone(), left + (width - num.width) / 2.0, axis - rule / 2.0 - gap - num.descent);
                out.place(den.clone(), left + (width - den.width) / 2.0, axis + rule / 2.0 + gap + den.ascent);
                if *bar {
                    out.ink.push(Layout::rule(left, axis - rule / 2.0, width, rule));
                }
                out.width = left + width + em * 0.08;
                out
            }
            Math::Script { base, sub, sup } => {
                let base = self.row(base, style);
                let small = style.script();
                self.scripted(base, sub.as_ref().map(|sub| self.row(sub, small)), sup.as_ref().map(|sup| self.row(sup, small)), em)
            }
            Math::Radical { degree, body } => {
                let body = self.row(body, style);
                let gap = em * 0.12;
                let top = -(body.ascent + gap + rule / 2.0);
                let bottom = body.descent;
                let middle = bottom - 0.45 * (bottom - top);
                let sign = em * 0.55;
                // The degree sits in the crook of the sign.
                let degree = degree.as_ref().map(|degree| {
                    self.row(
                        degree,
                        Style {
                            size: (em * 0.55).max(style.floor),
                            ..style
                        },
                    )
                });
                let shift = degree.as_ref().map_or(0.0, |degree| (degree.width - em * 0.3).max(0.0));
                let mut out = MathBox::space(shift);
                if let Some(degree) = degree {
                    let x = shift + em * 0.3 - degree.width;
                    out.place(degree.clone(), x.max(0.0), middle - em * 0.12 - degree.descent);
                }
                out.ink.push(Ink::Stroke {
                    points: vec![
                        (shift, middle),
                        (shift + em * 0.12, middle - em * 0.06),
                        (shift + em * 0.3, bottom),
                        (shift + sign, top),
                        (shift + sign + body.width + em * 0.08, top),
                    ],
                    width: rule,
                });
                out.place(body.clone(), shift + sign + em * 0.03, 0.0);
                out.ascent = out.ascent.max(-top + rule);
                out.width = shift + sign + body.width + em * 0.14;
                out
            }
            Math::Nary { op, sub, sup, limits_beside, body } => {
                // The sign, large, centred on the axis.
                let large = Style {
                    size: em * if *limits_beside { 1.8 } else { 1.5 },
                    ..style
                };
                let sign = self.glyphs(&op.to_string(), large, false);
                let sign = {
                    let dy = (sign.ascent - sign.descent) / 2.0 + axis;
                    let mut centred = MathBox::default();
                    centred.place(sign, 0.0, dy);
                    centred
                };
                let small = style.script();
                let (sub, sup) = (sub.as_ref().map(|sub| self.row(sub, small)), sup.as_ref().map(|sup| self.row(sup, small)));
                let mut out = if *limits_beside {
                    let mut out = sign.clone();
                    let x = sign.width + em * 0.02;
                    if let Some(sup) = sup {
                        out.place(sup.clone(), x, -(sign.ascent - sup.ascent * 0.6));
                    }
                    if let Some(sub) = sub {
                        out.place(sub.clone(), x - em * 0.25, sign.descent - sub.descent * 0.2);
                    }
                    out
                } else {
                    Layout::stacked(sign, sup, sub, em * 0.08)
                };
                out.width += em * 0.16;
                out.append(self.row(body, style));
                out
            }
            Math::Delimited { open, close, items } => {
                let mut inner = MathBox::default();
                let boxes: Vec<MathBox> = items.iter().map(|item| self.row(item, style)).collect();
                let ascent = boxes.iter().map(|b| b.ascent).fold(0.0, f32::max);
                let descent = boxes.iter().map(|b| b.descent).fold(0.0, f32::max);
                // Tall content: delimiters drawn to its height, the same
                // above and below the axis. Else the glyphs of the font.
                let (line_ascent, line_descent) = self.fonts.extent(em);
                let tall = ascent > line_ascent * 1.15 || descent > line_descent * 1.6;
                let reach = (ascent + axis).max(descent - axis) + em * 0.08;
                let (top, bottom) = (axis - reach, axis + reach);
                let fence = |ch: char| {
                    if tall {
                        Layout::delimiter(ch, top, bottom, em)
                    } else {
                        self.glyphs(&ch.to_string(), style, false)
                    }
                };
                if let Some(open) = open {
                    inner.append(fence(*open));
                }
                for (i, item) in boxes.into_iter().enumerate() {
                    if i > 0 {
                        inner.width += em * 0.1;
                        inner.append(fence('|'));
                        inner.width += em * 0.1;
                    }
                    inner.append(item);
                }
                if let Some(close) = close {
                    inner.append(fence(*close));
                }
                inner
            }
            Math::Function { name, arg } => {
                // A little room before the name, unless it leads.
                let mut out = MathBox::space(if first { 0.0 } else { em * 0.17 });
                out.append(self.row(name, Style { upright: true, ..style }));
                out.width += em * 0.17;
                out.append(self.row(arg, style));
                out
            }
            Math::Accent { mark, body } => {
                let body = self.row(body, style);
                let (line_ascent, _) = self.fonts.extent(em);
                // The body of a letter is lower than the line: the mark
                // sits on the letter.
                let top = body.ascent.min(line_ascent * 0.78).max(body.ascent - line_ascent * 0.22);
                let mut out = body.clone();
                if *mark == '\u{305}' {
                    out.ink.push(Layout::rule(0.0, -(top + em * 0.1), body.width, rule));
                    out.ascent = out.ascent.max(top + em * 0.1 + rule);
                    return out;
                }
                let sign = match mark {
                    '\u{303}' => '˜',
                    '\u{307}' => '˙',
                    '\u{308}' => '¨',
                    '\u{30c}' => 'ˇ',
                    '\u{20d7}' => '→',
                    _ => 'ˆ',
                };
                let small = Style {
                    size: if sign == '→' { em * 0.6 } else { em },
                    ..style
                };
                let sign_box = self.glyphs(&sign.to_string(), small, false);
                // Spacing accents are drawn where they sit over a letter
                // of their own height: lifted by what the body adds.
                let lift = if sign == '→' { top - em * 0.05 } else { top - line_ascent * 0.68 };
                out.place(sign_box.clone(), (body.width - sign_box.width) / 2.0 + em * 0.04, -lift.max(0.0));
                out.width = body.width;
                out
            }
            Math::Bar { over, body } => {
                let mut out = self.row(body, style);
                if *over {
                    out.ink.push(Layout::rule(0.0, -(out.ascent + em * 0.08), out.width, rule));
                    out.ascent += em * 0.08 + rule;
                } else {
                    out.ink.push(Layout::rule(0.0, out.descent + em * 0.06, out.width, rule));
                    out.descent += em * 0.06 + rule;
                }
                out
            }
            Math::Limit { base, limit, upper } => {
                let base = self.row(base, style);
                let limit = self.row(limit, style.script());
                if *upper {
                    Layout::stacked(base, Some(limit), None, em * 0.06)
                } else {
                    Layout::stacked(base, None, Some(limit), em * 0.06)
                }
            }
            Math::Matrix { rows } => {
                let cells: Vec<Vec<MathBox>> = rows.iter().map(|row| row.iter().map(|cell| self.row(cell, style)).collect()).collect();
                let columns = cells.iter().map(Vec::len).max().unwrap_or(0);
                let widths: Vec<f32> = (0..columns)
                    .map(|column| cells.iter().filter_map(|row| row.get(column)).map(|cell| cell.width).fold(0.0, f32::max))
                    .collect();
                let (column_gap, row_gap) = (em * 0.8, em * 0.3);
                let heights: Vec<(f32, f32)> = cells
                    .iter()
                    .map(|row| (row.iter().map(|cell| cell.ascent).fold(0.0, f32::max), row.iter().map(|cell| cell.descent).fold(0.0, f32::max)))
                    .collect();
                let total: f32 = heights.iter().map(|(a, d)| a + d).sum::<f32>() + row_gap * heights.len().saturating_sub(1) as f32;
                let mut out = MathBox::default();
                let mut y = axis - total / 2.0;
                for (row, (ascent, descent)) in cells.into_iter().zip(heights) {
                    let mut x = 0.0;
                    for (column, cell) in row.into_iter().enumerate() {
                        let dx = x + (widths[column] - cell.width) / 2.0;
                        out.place(cell, dx, y + ascent);
                        x += widths[column] + column_gap;
                    }
                    y += ascent + descent + row_gap;
                }
                out.width = widths.iter().sum::<f32>() + column_gap * columns.saturating_sub(1) as f32;
                out
            }
        }
    }
}

/// Lay an equation out at a size (pixels): built up, or as the characters
/// of its linear format.
pub(super) fn layout(equation: &Equation, size: f32, fonts: &dyn Measure) -> MathBox {
    let style = Style {
        size,
        floor: (size * 0.5).max(6.0).min(size),
        upright: false,
        nested: false,
    };
    let layout = Layout { fonts };
    if equation.linear {
        let shown: String = equation.to_linear().chars().filter(|ch| *ch != APPLY).collect();
        layout.row(&tokens(&shown), style)
    } else {
        layout.row(&equation.body, style)
    }
}

// ---- drawing -------------------------------------------------------------

/// The items that draw a box with the left end of its baseline at
/// (x, y), in pixels from the origin of the brush.
fn draw_box(laid: &MathBox, at: (f32, f32), color: Hsla, family: &SharedString, brush: &Brush, out: &mut Vec<Draw>) {
    for ink in &laid.ink {
        match ink {
            Ink::Glyphs { text, size, italic, x, y } => {
                let slanted = if *italic { math_italic(family, text) } else { None };
                let (text, italic) = match &slanted {
                    Some(slanted) => (slanted, &false),
                    None => (text, italic),
                };
                let font = Font {
                    family: family.clone(),
                    features: Default::default(),
                    fallbacks: None,
                    weight: FontWeight::NORMAL,
                    style: if *italic { FontStyle::Italic } else { FontStyle::Normal },
                };
                let id = brush.ts.resolve_font(&font);
                let (ascent, descent) = (f32::from(brush.ts.ascent(id, px(*size))), f32::from(brush.ts.descent(id, px(*size))).abs());
                let shared: SharedString = text.clone().into();
                let run = TextRun {
                    len: shared.len(),
                    font,
                    color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                };
                let line = brush.ts.shape_line(shared, px(*size), &[run], None);
                // A line as high as its letters: its baseline is where
                // the box says.
                out.push(Draw::Text {
                    line,
                    origin: point(brush.origin.x + px(at.0 + x), brush.origin.y + px(at.1 + y - ascent)),
                    line_height: px(ascent + descent),
                    clip: brush.clip,
                });
            }
            Ink::Rule { x, y, width, height } => {
                let (x0, y0) = (at.0 + x, at.1 + y);
                brush.fill(&[(x0, y0), (x0 + width, y0), (x0 + width, y0 + height), (x0, y0 + height)], color, out);
            }
            Ink::Stroke { points, width } => {
                let line = Polyline {
                    points: points.iter().map(|(x, y)| (at.0 + x, at.1 + y)).collect(),
                    closed: false,
                };
                brush.stroke(&line, width.max(0.75), &[], color, out);
            }
        }
    }
}

fn ink_color(shape: &Shape) -> Hsla {
    engine_color(shape.font.color.unwrap_or(opensheet_engine::Color::BLACK))
}

/// Draw the equation of a shape in its text box, level.
pub(super) fn draw(shape: &Shape, equation: &Equation, placed: &Placed, natural: (f32, f32), brush: &Brush, fonts: &FontResolver, out: &mut Vec<Draw>) {
    let family = math_family(fonts);
    let measure = Fonts { ts: brush.ts, family: family.clone() };
    let laid = layout(equation, pt_to_px(shape.font.size_pt()) * brush.zoom, &measure);
    let level = Placed { rotation: 0.0, ..*placed };
    let (x0, y0, x1, y1) = paint::text_box(shape, &level, natural, brush.zoom);
    let x = match shape.align {
        TextAlign::Left | TextAlign::Justify => x0,
        TextAlign::Center => (x0 + x1 - laid.width) / 2.0,
        TextAlign::Right => x1 - laid.width,
    };
    let top = match shape.anchor {
        TextAnchor::Top => y0,
        TextAnchor::Middle => (y0 + y1 - laid.height()) / 2.0,
        TextAnchor::Bottom => y1 - laid.height(),
    };
    draw_box(&laid, (x.round(), (top + laid.ascent).round()), ink_color(shape), &family, brush, out);
}

/// The size (at 100 %) of the box that fits the equation of a shape.
fn fitted_size(shape: &Shape, fonts: &FontResolver, ts: &TextSystem) -> Option<(f32, f32)> {
    let equation = shape.math.as_ref()?;
    let measure = Fonts { ts, family: math_family(fonts) };
    let size = pt_to_px(shape.font.size_pt());
    let laid = layout(equation, size, &measure);
    Some(((laid.width + 2.0 * TEXT_MARGIN.0 + size * 0.5).ceil().max(size * 4.0), (laid.height() + 2.0 * TEXT_MARGIN.1 + size * 0.3).ceil()))
}

/// Bring the box of an equation to the size of the equation, keeping its
/// middle where it is. Part of the undo step being made.
pub(super) fn fit(app: &mut ExcelApp, id: u32, cx: &mut Cx) {
    let sheet = app.sheet;
    let Some(fitted) = app.wb.drawing(sheet, id).and_then(Drawing::as_shape).and_then(|shape| fitted_size(shape, &app.fonts, cx.text_system())) else {
        return;
    };
    app.wb.update_drawing(sheet, id, |drawing| drawing.size = fitted);
}

/// The text of a shape that holds an equation changed: the equation is
/// built up from it again.
pub(super) fn retyped(shape: &mut Shape) {
    if let Some(equation) = &mut shape.math {
        equation.body = opensheet_engine::math::parse(&shape.text);
        // What was typed with names (`\alpha`) is kept with the
        // characters they stand for.
        shape.text = equation.to_linear();
        shape.runs.clear();
    }
}

// ---- while typing --------------------------------------------------------

/// What is being typed in an equation, built up: shown under its shape.
#[derive(Default)]
pub(super) struct Typing {
    preview: Option<(u32, Equation)>,
}

impl Typing {
    #[cfg(test)]
    pub(super) fn preview(&self) -> Option<Equation> {
        self.preview.as_ref().map(|(_, equation)| equation.clone())
    }
}

/// The text being typed in a shape changed.
pub(super) fn typed(app: &mut ExcelApp, cx: &mut Cx) {
    let current = app.ext::<DrawingUi>().and_then(|ui| ui.text.as_ref()).map(|edit| (edit.id, edit.area.read(cx).text().to_string()));
    let preview = current.and_then(|(id, text)| {
        let shape = app.wb.drawing(app.sheet, id)?.as_shape()?;
        shape.math.as_ref()?;
        Some((id, Equation::parse(&text)))
    });
    app.ext_mut::<Typing>().preview = preview;
    cx.notify();
}

/// Under the shape whose equation is being typed: the equation as it
/// builds up, on a white card.
pub(super) fn draw_preview(app: &ExcelApp, drawing: &Drawing, placed: &Placed, brush: &Brush, out: &mut Vec<Draw>) {
    let Some(shape) = drawing.as_shape() else {
        return;
    };
    let typed = app.ext::<Typing>().and_then(|typing| typing.preview.as_ref()).filter(|(id, _)| *id == drawing.id).map(|(_, equation)| equation);
    let Some(equation) = typed.or(shape.math.as_ref()) else {
        return;
    };
    let family = math_family(&app.fonts);
    let measure = Fonts { ts: brush.ts, family: family.clone() };
    let laid = layout(equation, pt_to_px(shape.font.size_pt()) * brush.zoom, &measure);
    if laid.width <= 0.0 {
        return;
    }
    let (_, _, _, bottom) = placed.extent();
    let (x, y) = (placed.x0.min(placed.x1), bottom + 8.0);
    let (w, h) = (laid.width + 16.0, laid.height() + 12.0);
    brush.fill(&[(x, y), (x + w, y), (x + w, y + h), (x, y + h)], c(0xFFFFFF), out);
    let frame = Polyline {
        points: vec![(x, y), (x + w, y), (x + w, y + h), (x, y + h)],
        closed: true,
    };
    brush.stroke(&frame, 1.0, &[], c(theme::BORDER), out);
    draw_box(&laid, (x + 8.0, y + 6.0 + laid.ascent), c(0x000000), &family, brush, out);
}

// ---- inserting -----------------------------------------------------------

/// True when the selected shape holds an equation (it gets the Equation
/// tab beside Shape Format).
pub(super) fn is_equation(drawing: &Drawing) -> bool {
    drawing.as_shape().is_some_and(|shape| shape.math.is_some())
}

/// Insert ▸ Equation: a text box holding an equation, in the middle of
/// what the window shows. Without text, it is ready to be typed in.
pub(super) fn insert(app: &mut ExcelApp, linear: &str, window: &mut Window, cx: &mut Cx) -> Option<u32> {
    if !app.may(Intent::EditObject(None), cx) {
        return None;
    }
    let mut shape = ShapeTool::TextBox.shape();
    shape.fill = None;
    shape.outline = None;
    shape.align = TextAlign::Center;
    shape.anchor = TextAnchor::Middle;
    let equation = Equation::parse(linear);
    shape.text = equation.to_linear();
    shape.math = Some(equation);
    let fitted = fitted_size(&shape, &app.fonts, cx.text_system()).unwrap_or((180.0, 40.0));
    let size = (fitted.0.max(180.0), fitted.1.max(36.0));
    let (x, y) = super::wordart::middle_of_view(app, size);
    let mut drawing = Drawing::shape(shape, size);
    drawing.offset = (x as f32, y as f32);
    let sheet = app.sheet;
    let id = app.wb.add_drawing(sheet, drawing)?;
    app.commit(t("Insert Equation", "Insérer une équation"), cx);
    app.select_object(Some(object_id(&Drawings, id)), cx);
    app.context_tab = Some(EQUATION_TAB);
    if linear.is_empty() {
        begin_text_edit(app, id, TextStart::End, window, cx);
    }
    Some(id)
}

/// Put the linear form of a structure or a symbol into the selected
/// equation: where the caret is while it is typed in, else at its end.
pub(super) fn insert_text(app: &mut ExcelApp, text: &str, window: &mut Window, cx: &mut Cx) -> bool {
    let Some(id) = selected(app).filter(|drawing| is_equation(drawing)).map(|drawing| drawing.id) else {
        return false;
    };
    if !app.may(Intent::EditObject(Some(object_id(&Drawings, id))), cx) {
        return false;
    }
    let editing = app.ext::<DrawingUi>().and_then(|ui| ui.text.as_ref()).filter(|edit| edit.id == id).map(|edit| edit.area.clone());
    if let Some(area) = editing {
        area.update(cx, |area, cx| area.replace_text_in_range(None, text, window, cx));
        typed(app, cx);
        return true;
    }
    let sheet = app.sheet;
    app.wb.update_drawing(sheet, id, |drawing| {
        if let Some(shape) = drawing.as_shape_mut() {
            // A structure after a letter would be read with it: a blank
            // between them keeps them apart.
            let glue = shape.text.chars().last().is_some_and(|ch| ch.is_alphanumeric()) && text.chars().next().is_some_and(|ch| ch.is_alphanumeric());
            if glue {
                shape.text.push(' ');
            }
            shape.text.push_str(text);
            retyped(shape);
        }
    });
    fit(app, id, cx);
    app.commit(t("Equation", "Équation"), cx);
    true
}

/// Professional / Linear: how the selected equation is shown.
pub(super) fn set_linear(app: &mut ExcelApp, linear: bool, cx: &mut Cx) -> bool {
    let Some(id) = selected(app).filter(|drawing| is_equation(drawing)).map(|drawing| drawing.id) else {
        return false;
    };
    if !app.may(Intent::EditObject(Some(object_id(&Drawings, id))), cx) {
        return false;
    }
    let sheet = app.sheet;
    app.wb.update_drawing(sheet, id, |drawing| {
        if let Some(equation) = drawing.as_shape_mut().and_then(|shape| shape.math.as_mut()) {
            equation.linear = linear;
        }
    });
    fit(app, id, cx);
    app.commit(if linear { Cmd::EqLinear.label() } else { Cmd::EqProfessional.label() }, cx);
    true
}

pub(super) fn shown_linear(app: &ExcelApp) -> Option<bool> {
    selected(app)?.as_shape()?.math.as_ref().map(|equation| equation.linear)
}

// ---- names ---------------------------------------------------------------

fn built_in_name(index: usize) -> &'static str {
    match index {
        0 => t("Area of Circle", "Surface du cercle"),
        1 => t("Binomial Theorem", "Formule du binôme"),
        2 => t("Expansion of a Sum", "Élévation d'une somme"),
        3 => t("Fourier Series", "Série de Fourier"),
        4 => t("Pythagorean Theorem", "Théorème de Pythagore"),
        5 => t("Quadratic Formula", "Formule quadratique"),
        6 => t("Taylor Expansion", "Développement de Taylor"),
        7 => t("Trig Identity 1", "Identité trigonométrique 1"),
        _ => t("Trig Identity 2", "Identité trigonométrique 2"),
    }
}

/// The command that opens the gallery of a kind of structure.
pub(super) fn structure_of(cmd: Cmd) -> Option<Structure> {
    Some(match cmd {
        Cmd::EqFraction => Structure::Fraction,
        Cmd::EqScript => Structure::Script,
        Cmd::EqRadical => Structure::Radical,
        Cmd::EqIntegral => Structure::Integral,
        Cmd::EqLargeOperator => Structure::LargeOperator,
        Cmd::EqBracket => Structure::Bracket,
        Cmd::EqFunction => Structure::Function,
        Cmd::EqAccent => Structure::Accent,
        Cmd::EqLimit => Structure::LimitAndLog,
        Cmd::EqOperator => Structure::Operator,
        Cmd::EqMatrix => Structure::Matrix,
        _ => return None,
    })
}

// ---- galleries -----------------------------------------------------------

/// An equation drawn in a box of the galleries: at `size`, smaller when
/// it is wider or higher than the box.
fn preview(linear: &'static str, family: SharedString, width: f32, height: f32, size: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, cx| {
            let ts = window.text_system().clone();
            let brush = Brush {
                origin: bounds.origin,
                zoom: 1.0,
                scale: window.scale_factor(),
                ts: &ts,
                clip: bounds,
            };
            let equation = Equation::parse(linear);
            let measure = Fonts { ts: &ts, family: family.clone() };
            let mut laid = layout(&equation, size, &measure);
            let fit = ((width - 6.0) / laid.width.max(1.0)).min((height - 4.0) / laid.height().max(1.0));
            if fit < 1.0 {
                laid = layout(&equation, (size * fit).max(5.0), &measure);
            }
            let mut items = Vec::new();
            let at = (((width - laid.width) / 2.0).round(), ((height - laid.height()) / 2.0 + laid.ascent).round());
            draw_box(&laid, at, c(0x262626), &family, &brush, &mut items);
            crate::grid::paint_layers(vec![Layer { clip: bounds, items }], window, cx);
        },
    )
    .w(px(width))
    .h(px(height))
    .flex_none()
}

/// What a drop-down of the Equation tab offers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Offer {
    /// The built-in equations, and a new empty one.
    BuiltIn,
    Structures(Structure),
    Symbols,
}

struct Gallery {
    app: WeakEntity<ExcelApp>,
    offer: Offer,
    family: SharedString,
}

impl Gallery {
    fn cell(&self, index: usize, content: impl IntoElement, act: impl Fn(&mut ExcelApp, &mut Window, &mut Cx) + 'static) -> gpui::Stateful<gpui::Div> {
        let app = self.app.clone();
        div()
            .id(("equation-choice", index))
            .rounded(px(3.))
            .border_1()
            .border_color(c(theme::BORDER))
            .bg(c(0xFFFFFF))
            .hover(|s| s.border_color(c(theme::ACCENT)).bg(c(theme::HOVER)))
            .child(content)
            .on_click(move |_, window, cx| {
                app.update(cx, |app, cx| {
                    app.popup = None;
                    act(app, window, cx);
                    cx.notify();
                })
                .ok();
            })
    }
}

impl Render for Gallery {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        match self.offer {
            Offer::BuiltIn => {
                let mut cells: Vec<AnyElement> = BUILT_IN
                    .iter()
                    .enumerate()
                    .map(|(index, (_, linear))| {
                        let content = div()
                            .p(px(4.))
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(div().text_size(px(11.5)).text_color(c(theme::TEXT_MUTED)).child(built_in_name(index)))
                            .child(preview(linear, self.family.clone(), 380.0, 46.0, 17.0));
                        self.cell(index, content, move |app, window, cx| {
                            insert(app, linear, window, cx);
                        })
                        .into_any_element()
                    })
                    .collect();
                cells.push(
                    self.cell(BUILT_IN.len(), div().h(px(26.)).px(px(8.)).flex().items_center().child(t("Insert New Equation", "Insérer une nouvelle équation")), |app, window, cx| {
                        insert(app, "", window, cx);
                    })
                    .into_any_element(),
                );
                div().w(px(404.)).p(px(6.)).flex().flex_col().gap(px(4.)).text_size(px(12.)).children(cells)
            }
            Offer::Structures(structure) => {
                let cells = structure.templates().iter().enumerate().map(|(index, linear)| {
                    let wide = linear.chars().count() > 12;
                    self.cell(index, preview(linear, self.family.clone(), if wide { 152.0 } else { 74.0 }, 56.0, 17.0), move |app, window, cx| {
                        insert_text(app, linear, window, cx);
                    })
                });
                div().w(px(4.0 * 74.0 + 3.0 * 4.0 + 20.0)).p(px(6.)).flex().flex_wrap().gap(px(4.)).children(cells)
            }
            Offer::Symbols => {
                let family = self.family.clone();
                let cells = SYMBOLS.chars().enumerate().map(|(index, symbol)| {
                    let content = div().size(px(26.)).flex().items_center().justify_center().font_family(family.clone()).text_size(px(15.)).child(symbol.to_string());
                    self.cell(index, content, move |app, window, cx| {
                        insert_text(app, &symbol.to_string(), window, cx);
                    })
                });
                div().w(px(12.0 * 28.0 + 11.0 * 2.0 + 12.0)).p(px(6.)).flex().flex_wrap().gap(px(2.)).children(cells)
            }
        }
    }
}

pub(super) fn open_gallery(app: &mut ExcelApp, offer: Offer, window: &mut Window, cx: &mut Cx) {
    let weak = cx.weak_entity();
    let family = math_family(&app.fonts);
    tabs::open_dropdown(app, window, cx, |_| Gallery { app: weak, offer, family });
}

// ---- the tab -------------------------------------------------------------

/// The groups of the "Equation" tab.
pub(super) fn equation_tab(app: &mut ExcelApp, cx: &mut Cx) -> Vec<AnyElement> {
    let family = math_family(&app.fonts);
    // The first symbols at hand, the others in the drop-down.
    let quick: Vec<AnyElement> = SYMBOLS
        .chars()
        .take(16)
        .enumerate()
        .map(|(index, symbol)| {
            div()
                .id(("eq-symbol", index))
                .size(px(22.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(3.))
                .font_family(family.clone())
                .text_size(px(14.))
                .hover(|s| s.bg(c(theme::HOVER)))
                .active(|s| s.bg(c(theme::PRESSED)))
                .child(symbol.to_string())
                .tooltip(tooltip(t("Symbols", "Symboles")))
                .on_click(cx.listener(move |app, _, window, cx| {
                    insert_text(app, &symbol.to_string(), window, cx);
                }))
                .into_any_element()
        })
        .collect();
    let (first, second) = quick.into_iter().enumerate().partition::<Vec<_>, _>(|(index, _)| *index < 8);
    let line = |cells: Vec<(usize, AnyElement)>| div().flex().children(cells.into_iter().map(|(_, cell)| cell));
    let structures = [
        ("eq-fraction", Cmd::EqFraction),
        ("eq-script", Cmd::EqScript),
        ("eq-radical", Cmd::EqRadical),
        ("eq-integral", Cmd::EqIntegral),
        ("eq-large", Cmd::EqLargeOperator),
        ("eq-bracket", Cmd::EqBracket),
        ("eq-function", Cmd::EqFunction),
        ("eq-accent", Cmd::EqAccent),
        ("eq-limit", Cmd::EqLimit),
        ("eq-operator", Cmd::EqOperator),
        ("eq-matrix", Cmd::EqMatrix),
    ];
    vec![
        group(
            t("Tools", "Outils"),
            div().flex().items_center().gap(px(2.)).child(app.cmd_large("eq-equation", Cmd::Equation, true, cx)).child(rows(vec![
                app.cmd_labeled("eq-professional", Cmd::EqProfessional, false, cx),
                app.cmd_labeled("eq-linear", Cmd::EqLinear, false, cx),
            ])),
        )
        .into_any_element(),
        group(
            t("Symbols", "Symboles"),
            div()
                .flex()
                .items_center()
                .gap(px(4.))
                .child(div().flex().flex_col().child(line(first)).child(line(second)))
                .child(app.cmd_large("eq-symbols", Cmd::EqSymbols, true, cx)),
        )
        .into_any_element(),
        group(
            t("Structures", "Structures"),
            row(structures.into_iter().map(|(id, cmd)| app.cmd_large(id, cmd, true, cx)).collect()),
        )
        .border_r_0()
        .into_any_element(),
    ]
}

/// Run a command of equations.
pub(super) fn run(app: &mut ExcelApp, cmd: Cmd, window: &mut Window, cx: &mut Cx) {
    match cmd {
        Cmd::Equation => open_gallery(app, Offer::BuiltIn, window, cx),
        Cmd::EqProfessional => {
            set_linear(app, false, cx);
        }
        Cmd::EqLinear => {
            set_linear(app, true, cx);
        }
        Cmd::EqSymbols => open_gallery(app, Offer::Symbols, window, cx),
        other => {
            if let Some(structure) = structure_of(other) {
                open_gallery(app, Offer::Structures(structure), window, cx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character half its size wide; letters 0.8 above the baseline
    /// and 0.2 below.
    struct Fixed;

    impl Measure for Fixed {
        fn width(&self, text: &str, size: f32, _: bool) -> f32 {
            text.chars().count() as f32 * size * 0.5
        }

        fn extent(&self, size: f32) -> (f32, f32) {
            (size * 0.8, size * 0.2)
        }
    }

    fn laid(linear: &str) -> MathBox {
        layout(&Equation::parse(linear), 20.0, &Fixed)
    }

    fn glyphs(laid: &MathBox) -> Vec<(String, f32, f32, f32)> {
        laid.ink
            .iter()
            .filter_map(|ink| match ink {
                Ink::Glyphs { text, size, x, y, .. } => Some((text.clone(), *size, *x, *y)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn letters_slant_and_signs_have_room() {
        let sum = laid("a+12");
        let found = glyphs(&sum);
        assert_eq!(found.iter().map(|g| g.0.as_str()).collect::<Vec<_>>(), vec!["a", "+", "12"]);
        // Room on both sides of the sign, none before a sign that leads.
        assert!((found[1].2 - (10.0 + 4.4)).abs() < 0.01 && (found[2].2 - (10.0 + 4.4 + 10.0 + 4.4)).abs() < 0.01);
        assert_eq!(glyphs(&laid("-x"))[0], ("−".to_string(), 20.0, 0.0, 0.0));
        let italics: Vec<bool> = laid("xΔ2").ink.iter().map(|ink| matches!(ink, Ink::Glyphs { italic: true, .. })).collect();
        assert_eq!(italics, vec![true, false, false]);
        assert_eq!((sum.ascent, sum.descent), (16.0, 4.0));
        // The name of a function stands upright, a little before its
        // argument.
        let sine = laid("sin\u{2061}x");
        assert!(matches!(&sine.ink[0], Ink::Glyphs { text, italic: false, .. } if text == "sin"));
        assert!(matches!(&sine.ink[1], Ink::Glyphs { text, italic: true, x, .. } if text == "x" && *x > 30.0));
        // A mathematical font has its slanted letters as characters.
        assert_eq!(math_italic("STIX Two Math", "ahZβ2").as_deref(), Some("𝑎ℎ𝑍𝛽2"));
        assert_eq!(math_italic("Times New Roman", "a"), None);
    }

    #[test]
    fn fractions_stack_on_the_axis() {
        let half = laid("a/bc");
        let found = glyphs(&half);
        // The numerator above the bar, the denominator under it, both
        // centred; the bar on the axis.
        let Some(Ink::Rule { x, y, width, height }) = half.ink.iter().find(|ink| matches!(ink, Ink::Rule { .. })) else {
            panic!("a bar");
        };
        assert!((y + height / 2.0 + AXIS * 20.0).abs() < 0.01);
        assert!(found[0].3 < *y && found[1].3 > y + height);
        let middle = x + width / 2.0;
        assert!((found[0].2 + 5.0 - middle).abs() < 0.01 && (found[1].2 + 10.0 - middle).abs() < 0.01);
        assert!(half.ascent > 16.0 && half.descent > 4.0);
        // A fraction in a fraction is smaller; one without a bar has none.
        let nested = glyphs(&laid("(a/b)/c"));
        assert!(nested[0].1 < 20.0 && nested[2].1 == 20.0);
        assert!(!laid("n¦k").ink.iter().any(|ink| matches!(ink, Ink::Rule { .. })));
    }

    #[test]
    fn scripts_radicals_and_operators() {
        let power = glyphs(&laid("x_i^2"));
        assert_eq!(power[0].1, 20.0);
        assert!((power[1].1 - 14.0).abs() < 0.01 && (power[2].1 - 14.0).abs() < 0.01);
        // The superscript is raised, the subscript lowered, both after
        // the base.
        assert!(power[1].3 < -6.0 && power[2].3 > 3.0 && power[1].2 > 10.0 && power[2].2 > 10.0);
        // Scripts of scripts stop shrinking.
        let deep = glyphs(&laid("x^(y^(z^w))"));
        assert_eq!(deep[2].1, deep[3].1);
        // A radical: its sign runs over the body, and takes the degree.
        let root = laid("√(3&x+1)");
        let Some(Ink::Stroke { points, .. }) = root.ink.iter().find(|ink| matches!(ink, Ink::Stroke { .. })) else {
            panic!("a radical sign");
        };
        assert_eq!(points.len(), 5);
        assert!(points[4].0 > points[3].0 + 30.0 && points[3].1 < -16.0);
        assert!(glyphs(&root)[0].1 < 14.0);
        // A sum: limits over and under the sign, the body after it.
        let sum = glyphs(&laid("∑_(k=0)^n▒a_k"));
        let sign = sum.iter().find(|g| g.0 == "∑").unwrap();
        let (upper, lower) = (sum.iter().find(|g| g.0 == "n").unwrap(), sum.iter().find(|g| g.0 == "k").unwrap());
        assert_eq!(sign.1, 30.0);
        assert!(upper.3 < sign.3 - 20.0 && lower.3 > sign.3 + 4.0);
        assert!(sum.iter().find(|g| g.0 == "a").unwrap().2 > sign.2 + 15.0);
        // An integral has them beside it.
        let integral = glyphs(&laid("∫_0^1▒x"));
        let sign = integral.iter().find(|g| g.0 == "∫").unwrap();
        assert!(integral.iter().find(|g| g.0 == "1").unwrap().2 >= sign.2 + 18.0);
    }

    #[test]
    fn delimiters_grow_and_matrices_align() {
        // Around a letter: the brackets of the font.
        let small = laid("(x)");
        assert_eq!(glyphs(&small).len(), 3);
        // Around a fraction: drawn, as tall as the fraction, the same
        // above the axis as below.
        let tall = laid("(a/b)");
        let strokes: Vec<&Vec<(f32, f32)>> = tall.ink.iter().filter_map(|ink| match ink { Ink::Stroke { points, .. } => Some(points), _ => None }).collect();
        assert_eq!(strokes.len(), 2);
        let (top, bottom) = (strokes[0][0].1, strokes[0][strokes[0].len() - 1].1);
        assert!(((top + bottom) / 2.0 + AXIS * 20.0).abs() < 0.01 && bottom - top > 40.0);
        assert!((tall.ascent + top).abs() < 0.01 && strokes[1][0].0 > strokes[0][0].0 + 10.0);
        for brackets in ["[a/b]", "{a/b}", "⟨a/b⟩", "|a/b|", "‖a/b‖", "⌊a/b⌋", "⌈a/b⌉"] {
            assert!(laid(brackets).ink.iter().any(|ink| matches!(ink, Ink::Stroke { .. })), "{brackets}");
        }
        // A matrix: columns line up, rows are centred on the axis.
        let matrix = glyphs(&laid("■(a&bcd@ef&g)"));
        // (Glyphs: a, b, c, d, then e, f, g.)
        assert!((matrix[0].2 + 5.0 - (matrix[4].2 + 10.0)).abs() < 0.01);
        assert!((matrix[1].2 + 15.0 - (matrix[6].2 + 5.0)).abs() < 0.01);
        assert!(matrix[0].3 < 0.0 && matrix[4].3 > 0.0);
        // An accent sits over its letter, a limit under its word.
        let hat = glyphs(&laid("x\u{302}"));
        assert_eq!(hat.len(), 2);
        let limit = glyphs(&laid("lim┬(n→∞)\u{2061}a"));
        assert!(limit[1].3 > 10.0 && limit[1].1 < 20.0);
        // Linear: characters on one line.
        let mut equation = Equation::parse("a/b");
        equation.linear = true;
        let line = layout(&equation, 20.0, &Fixed);
        assert_eq!(glyphs(&line).iter().map(|g| g.0.as_str()).collect::<Vec<_>>(), vec!["a", "/", "b"]);
        assert_eq!(line.ascent, 16.0);
        // Every built-in equation and every structure is laid out.
        let mut all: Vec<&str> = BUILT_IN.iter().map(|(_, linear)| *linear).collect();
        for structure in Structure::ALL {
            all.extend(structure.templates());
        }
        for linear in all {
            let laid = laid(linear);
            assert!(laid.width > 0.0 && laid.width.is_finite() && laid.height() >= 20.0 && laid.height() < 200.0, "{linear}");
        }
    }
}
