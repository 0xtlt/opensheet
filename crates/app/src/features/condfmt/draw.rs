//! What conditional formats draw: data bars, the 52 icons of the icon
//! sets and the strip of a colour scale, as vector shapes.
//!
//! Everything here works in window coordinates and only produces `Draw`
//! items, so the same code paints cells of the grid, the galleries of the
//! ribbon menu and the previews of the dialogs. Icons are designed on a
//! 16 × 16 box (their size in a cell of 11 pt text at 100 %) and scaled
//! from there, so they stay sharp at any zoom.

use crate::grid::{Draw, Layer};
use crate::theme::c;
use gpui::{
    BorderStyle, Bounds, Hsla, IntoElement, PathBuilder, Pixels, Point, Styled, canvas, fill, linear_color_stop,
    linear_gradient, point, px, quad, size, transparent_black,
};
use opensheet_engine::condfmt::{Bar, Icon, IconSet};
use opensheet_engine::style::Color;

// ------------------------------------------------------------------ palette

// (fill, outline) of the coloured shapes.
const GREEN: (u32, u32) = (0x5EB774, 0x255E1B);
const YELLOW: (u32, u32) = (0xFBD86F, 0xD87103);
const RED: (u32, u32) = (0xEE7D7E, 0x90271B);
const GRAY: (u32, u32) = (0x9E9E9E, 0x404040);
const GOLD: (u32, u32) = (0xF1D752, 0x9A7427);
const SILVER: (u32, u32) = (0xE3E3E3, 0x8E8E8E);
/// Filled and empty parts of the ratings icons (signal bars, boxes).
const RATING_ON: (u32, u32) = (0x567CA3, 0x002E5F);
const RATING_OFF: (u32, u32) = (0xD5D5D5, 0xA1A1A1);
const BOX_ON: (u32, u32) = (0x6789AD, 0x012D5A);
const BOX_OFF: (u32, u32) = (0xE9E9E9, 0xCFCFCF);
// The lamps of the rimmed traffic lights.
const LAMP_GREEN: u32 = 0x46B43C;
const LAMP_YELLOW: u32 = 0xF5C82D;
const LAMP_RED: u32 = 0xEA4640;

// ------------------------------------------------------------------- shapes

/// A vertical arrow pointing up; the other straight arrows are this one
/// turned.
const ARROW_UP: [(f32, f32); 7] = [
    (8.0, 0.5),
    (15.5, 8.0),
    (11.0, 8.0),
    (11.0, 15.5),
    (5.0, 15.5),
    (5.0, 8.0),
    (0.5, 8.0),
];

/// An arrow pointing up and to the right; mirrored for the one pointing
/// down.
const ARROW_INCLINE: [(f32, f32); 7] = [
    (15.5, 0.5),
    (15.5, 11.5),
    (11.98, 7.98),
    (4.46, 15.5),
    (0.5, 11.54),
    (8.02, 4.02),
    (4.5, 0.5),
];

const CHECK: [(f32, f32); 7] = [
    (0.8, 8.6),
    (3.0, 6.6),
    (5.8, 9.8),
    (12.8, 1.2),
    (15.2, 2.9),
    (6.4, 14.6),
    (5.2, 14.6),
];

const CROSS: [(f32, f32); 12] = [
    (1.0, 3.4),
    (3.4, 1.0),
    (8.0, 5.6),
    (12.6, 1.0),
    (15.0, 3.4),
    (10.4, 8.0),
    (15.0, 12.6),
    (12.6, 15.0),
    (8.0, 10.4),
    (3.4, 15.0),
    (1.0, 12.6),
    (5.6, 8.0),
];

/// One step of an outline.
#[derive(Clone, Copy)]
enum Seg {
    Move(f32, f32),
    Line(f32, f32),
    /// A quadratic curve: control point, end point.
    Curve((f32, f32), (f32, f32)),
}

/// The cloth of a flag, flying to the right of its pole.
const FLAG: [Seg; 6] = [
    Seg::Move(2.5, 1.3),
    Seg::Curve((6.0, -0.4), (8.6, 2.1)),
    Seg::Curve((11.0, 4.2), (13.9, 3.3)),
    Seg::Line(12.3, 8.3),
    Seg::Curve((10.0, 9.4), (8.0, 7.3)),
    Seg::Curve((6.2, 5.6), (4.3, 7.7)),
];

/// The ten corners of the star, clockwise from its top point: points and
/// notches in turn.
fn star_points() -> [(f32, f32); 10] {
    let (u, v, outer, inner) = (8.0, 8.7, 7.6, 3.2);
    std::array::from_fn(|i| {
        let a = (i as f32 * 36.0).to_radians();
        let r = if i % 2 == 0 { outer } else { inner };
        (u + r * a.sin(), v - r * a.cos())
    })
}

/// Draws on the 16 × 16 design box of an icon placed at (`x`, `y`) of the
/// window and `scale` times larger.
struct Art<'a> {
    out: &'a mut Vec<Draw>,
    x: f32,
    y: f32,
    scale: f32,
}

impl Art<'_> {
    fn at(&self, (u, v): (f32, f32)) -> Point<Pixels> {
        point(px(self.x + u * self.scale), px(self.y + v * self.scale))
    }

    fn bounds(&self, u0: f32, v0: f32, u1: f32, v1: f32) -> Bounds<Pixels> {
        Bounds::new(self.at((u0, v0)), size(px((u1 - u0) * self.scale), px((v1 - v0) * self.scale)))
    }

    /// Width of the outlines: one design unit, never thinner than what
    /// still reads on screen.
    fn line(&self) -> f32 {
        self.scale.max(0.75)
    }

    /// An outline made of segments, filled and, with `line`, outlined.
    fn path(&mut self, segs: &[Seg], fill_color: u32, line: Option<u32>) {
        let (x, y, scale) = (self.x, self.y, self.scale);
        let at = move |(u, v): (f32, f32)| point(px(x + u * scale), px(y + v * scale));
        let trace = |builder: &mut PathBuilder| {
            for seg in segs {
                match *seg {
                    Seg::Move(u, v) => builder.move_to(at((u, v))),
                    Seg::Line(u, v) => builder.line_to(at((u, v))),
                    Seg::Curve(ctrl, to) => builder.curve_to(at(to), at(ctrl)),
                }
            }
            builder.close();
        };
        let mut builder = PathBuilder::fill();
        trace(&mut builder);
        if let Ok(path) = builder.build() {
            self.out.push(Draw::Path(path, c(fill_color)));
        }
        if let Some(line) = line {
            let mut builder = PathBuilder::stroke(px(self.line()));
            trace(&mut builder);
            if let Ok(path) = builder.build() {
                self.out.push(Draw::Path(path, c(line)));
            }
        }
    }

    fn polygon(&mut self, points: &[(f32, f32)], (fill_color, line): (u32, u32)) {
        let segs: Vec<Seg> = points
            .iter()
            .enumerate()
            .map(|(i, (u, v))| if i == 0 { Seg::Move(*u, *v) } else { Seg::Line(*u, *v) })
            .collect();
        self.path(&segs, fill_color, Some(line));
    }

    /// An open line of the given width (in design units).
    fn stroke(&mut self, points: &[(f32, f32)], width: f32, color: u32) {
        let mut builder = PathBuilder::stroke(px(width * self.scale));
        builder.move_to(self.at(points[0]));
        for p in &points[1..] {
            builder.line_to(self.at(*p));
        }
        if let Ok(path) = builder.build() {
            self.out.push(Draw::Path(path, c(color)));
        }
    }

    /// A rectangle given by its outer edges, with rounded corners and an
    /// outline drawn inside it.
    fn rect(&mut self, u0: f32, v0: f32, u1: f32, v1: f32, radius: f32, fill_color: Hsla, line: Option<u32>) {
        self.out.push(Draw::Quad(quad(
            self.bounds(u0, v0, u1, v1),
            px(radius * self.scale),
            fill_color,
            px(if line.is_some() { self.line() } else { 0.0 }),
            line.map(c).unwrap_or(transparent_black()),
            BorderStyle::Solid,
        )));
    }

    fn disc(&mut self, u: f32, v: f32, radius: f32, fill_color: u32, line: Option<u32>) {
        self.rect(u - radius, v - radius, u + radius, v + radius, radius, c(fill_color), line);
    }

    /// The part of a disc swept clockwise from twelve o'clock over
    /// `quarters` quarter turns.
    fn sector(&mut self, u: f32, v: f32, radius: f32, quarters: u32, color: u32) {
        let steps = quarters * 9;
        let mut segs = vec![Seg::Move(u, v)];
        for i in 0..=steps {
            let a = (i as f32 * 10.0).to_radians();
            segs.push(Seg::Line(u + radius * a.sin(), v - radius * a.cos()));
        }
        self.path(&segs, color, None);
    }

    /// A five-pointed star; `left_half` keeps only what lies left of its
    /// axis (the top point and the notch at the bottom are on it).
    fn star(&mut self, fill_color: u32, line: Option<u32>, left_half: bool) {
        let points = star_points();
        let points: Vec<(f32, f32)> = if left_half {
            std::iter::once(points[0]).chain(points[5..].iter().copied()).collect()
        } else {
            points.to_vec()
        };
        let segs: Vec<Seg> = points
            .iter()
            .enumerate()
            .map(|(i, (u, v))| if i == 0 { Seg::Move(*u, *v) } else { Seg::Line(*u, *v) })
            .collect();
        self.path(&segs, fill_color, line);
    }

    /// The outline of the half star: grey on its grey side, gold on the
    /// other.
    fn star_outline(&mut self) {
        let points = star_points();
        let left: Vec<(f32, f32)> = points[5..].iter().copied().chain(std::iter::once(points[0])).collect();
        self.stroke(&points[..=5], 1.0, SILVER.1);
        self.stroke(&left, 1.0, GOLD.1);
    }

    fn arrow(&mut self, turn: Turn, colors: (u32, u32)) {
        let points: Vec<(f32, f32)> = match turn {
            Turn::Up => ARROW_UP.to_vec(),
            Turn::Side => ARROW_UP.iter().map(|(u, v)| (16.0 - v, *u)).collect(),
            Turn::Down => ARROW_UP.iter().map(|(u, v)| (16.0 - u, 16.0 - v)).collect(),
            Turn::InclineUp => ARROW_INCLINE.to_vec(),
            Turn::InclineDown => ARROW_INCLINE.iter().map(|(u, v)| (*u, 16.0 - v)).collect(),
        };
        self.polygon(&points, colors);
    }

    fn flag(&mut self, cloth: u32, line: u32) {
        self.stroke(&[(2.2, 0.8), (6.3, 15.5)], 1.2, 0x1A1A1A);
        self.path(&FLAG, cloth, Some(line));
    }

    /// A lamp in its dark casing.
    fn traffic_light(&mut self, lamp: u32) {
        self.rect(0.5, 0.5, 15.5, 15.5, 3.0, c(0x161616), None);
        self.disc(8.0, 8.0, 5.3, lamp, None);
        // The reflection that makes it a lamp rather than a dot.
        self.rect(4.7, 4.4, 8.3, 7.4, 1.6, crate::theme::ca(0xFFFFFF, 0.55), None);
    }

    /// A disc with a white sign on it.
    fn symbol_disc(&mut self, (fill_color, line): (u32, u32)) {
        self.disc(8.0, 8.0, 7.5, fill_color, Some(line));
    }

    /// The signal-strength meter with its first `on` bars filled.
    fn bars(&mut self, on: usize) {
        for (i, top) in [10.0, 7.0, 3.0, 0.0].into_iter().enumerate() {
            let (fill_color, line) = if i < on { RATING_ON } else { RATING_OFF };
            let left = 0.5 + i as f32 * 4.0;
            self.rect(left, top, left + 3.0, 16.0, 0.0, c(fill_color), Some(line));
        }
    }

    /// Four boxes in a square, `on` of them filled: bottom left, bottom
    /// right, top left, top right.
    fn boxes(&mut self, on: usize) {
        for (i, (left, top)) in [(0.5, 8.5), (8.5, 8.5), (0.5, 0.5), (8.5, 0.5)].into_iter().enumerate() {
            let (fill_color, line) = if i < on { BOX_ON } else { BOX_OFF };
            self.rect(left, top, left + 7.0, top + 7.0, 0.0, c(fill_color), Some(line));
        }
    }

    /// A disc with `quarters` quarters black, clockwise from the top.
    fn quarters(&mut self, quarters: u32) {
        self.disc(8.0, 8.0, 7.5, 0xFFFFFF, None);
        if quarters > 0 {
            self.sector(8.0, 8.0, 7.2, quarters, 0x000000);
        }
        self.out.push(Draw::Quad(quad(
            self.bounds(0.5, 0.5, 15.5, 15.5),
            px(7.5 * self.scale),
            transparent_black(),
            px(self.line() * 1.2),
            c(0x000000),
            BorderStyle::Solid,
        )));
    }
}

#[derive(Clone, Copy)]
enum Turn {
    Up,
    Side,
    Down,
    InclineUp,
    InclineDown,
}

/// Draw an icon in the square of side `size` whose top-left corner is at
/// (`x`, `y`) of the window.
pub fn icon(icon: Icon, x: f32, y: f32, size: f32, out: &mut Vec<Draw>) {
    let mut art = Art {
        out,
        x,
        y,
        scale: size / 16.0,
    };
    match icon {
        Icon::GreenUpArrow => art.arrow(Turn::Up, GREEN),
        Icon::YellowSideArrow => art.arrow(Turn::Side, YELLOW),
        Icon::RedDownArrow => art.arrow(Turn::Down, RED),
        Icon::GrayUpArrow => art.arrow(Turn::Up, GRAY),
        Icon::GraySideArrow => art.arrow(Turn::Side, GRAY),
        Icon::GrayDownArrow => art.arrow(Turn::Down, GRAY),
        Icon::YellowUpInclineArrow => art.arrow(Turn::InclineUp, YELLOW),
        Icon::YellowDownInclineArrow => art.arrow(Turn::InclineDown, YELLOW),
        Icon::GrayUpInclineArrow => art.arrow(Turn::InclineUp, GRAY),
        Icon::GrayDownInclineArrow => art.arrow(Turn::InclineDown, GRAY),
        Icon::GreenFlag => art.flag(0x63AE4C, 0x2B5A21),
        Icon::YellowFlag => art.flag(0xEDC13A, 0x8A6A12),
        Icon::RedFlag => art.flag(0xEA5855, 0x7A1F1C),
        Icon::GreenCircle => art.disc(8.0, 8.0, 7.5, GREEN.0, Some(GREEN.1)),
        Icon::YellowCircle => art.disc(8.0, 8.0, 7.5, 0xFCDA72, Some(YELLOW.1)),
        Icon::RedCircleWithBorder => art.disc(8.0, 8.0, 7.5, 0xF07F81, Some(RED.1)),
        Icon::BlackCircleWithBorder => art.disc(8.0, 8.0, 7.5, 0x3A3A3A, Some(0x000000)),
        Icon::GreenTrafficLight => art.traffic_light(LAMP_GREEN),
        Icon::YellowTrafficLight => art.traffic_light(LAMP_YELLOW),
        Icon::RedTrafficLight => art.traffic_light(LAMP_RED),
        Icon::YellowTriangle => art.polygon(&[(8.0, 1.2), (15.3, 14.2), (0.7, 14.2)], (0xFAD25C, YELLOW.1)),
        Icon::RedDiamond => art.polygon(&[(8.0, 0.8), (15.2, 8.0), (8.0, 15.2), (0.8, 8.0)], (0xE77777, RED.1)),
        Icon::GreenCheckSymbol => {
            art.symbol_disc((0x71BB82, GREEN.1));
            art.stroke(&[(4.0, 8.4), (6.9, 11.2), (12.0, 5.0)], 2.2, 0xFFFFFF);
        }
        Icon::YellowExclamationSymbol => {
            art.symbol_disc((0xEFC581, YELLOW.1));
            art.stroke(&[(8.0, 3.4), (8.0, 9.4)], 2.2, 0xFFFFFF);
            art.disc(8.0, 12.1, 1.25, 0xFFFFFF, None);
        }
        Icon::RedCrossSymbol => {
            art.symbol_disc((0xEA8E8D, RED.1));
            art.stroke(&[(5.0, 5.0), (11.0, 11.0)], 2.4, 0xFFFFFF);
            art.stroke(&[(11.0, 5.0), (5.0, 11.0)], 2.4, 0xFFFFFF);
        }
        Icon::GreenCheck => art.polygon(&CHECK, (0x55AC68, 0x286320)),
        Icon::YellowExclamation => {
            art.polygon(&[(6.2, 0.5), (9.8, 0.5), (9.0, 10.6), (7.0, 10.6)], (0xF6CE77, YELLOW.1));
            art.disc(8.0, 13.6, 1.9, 0xF6CE77, Some(YELLOW.1));
        }
        Icon::RedCross => art.polygon(&CROSS, (0xE17271, 0x91281C)),
        Icon::RedCircle => art.disc(8.0, 8.0, 7.0, 0xFF0000, None),
        Icon::PinkCircle => art.disc(8.0, 8.0, 7.0, 0xFF8D89, None),
        Icon::GrayCircle => art.disc(8.0, 8.0, 7.0, 0xBFBFBF, None),
        Icon::BlackCircle => art.disc(8.0, 8.0, 7.0, 0x000000, None),
        Icon::CircleWithOneWhiteQuarter => art.quarters(3),
        Icon::CircleWithTwoWhiteQuarters => art.quarters(2),
        Icon::CircleWithThreeWhiteQuarters => art.quarters(1),
        Icon::WhiteCircle => art.quarters(0),
        Icon::ZeroBars => art.bars(0),
        Icon::OneBar => art.bars(1),
        Icon::TwoBars => art.bars(2),
        Icon::ThreeBars => art.bars(3),
        Icon::FourBars => art.bars(4),
        Icon::GoldStar => art.star(GOLD.0, Some(GOLD.1), false),
        Icon::HalfGoldStar => {
            art.star(SILVER.0, None, false);
            art.star(GOLD.0, None, true);
            art.star_outline();
        }
        Icon::SilverStar => art.star(SILVER.0, Some(SILVER.1), false),
        Icon::GreenUpTriangle => art.polygon(&[(8.0, 3.2), (15.2, 9.8), (0.8, 9.8)], (0x60B071, 0x255D1A)),
        Icon::YellowDash => art.rect(0.5, 6.0, 15.5, 9.5, 0.0, c(0xFCD348), Some(YELLOW.1)),
        Icon::RedDownTriangle => art.polygon(&[(0.8, 5.6), (15.2, 5.6), (8.0, 12.2)], (0xE26767, RED.1)),
        Icon::FourFilledBoxes => art.boxes(4),
        Icon::ThreeFilledBoxes => art.boxes(3),
        Icon::TwoFilledBoxes => art.boxes(2),
        Icon::OneFilledBox => art.boxes(1),
        Icon::ZeroFilledBoxes => art.boxes(0),
    }
}

// ---------------------------------------------------------------- data bars

/// How far a gradient bar fades towards white at its tip.
const GRADIENT_FADE: f32 = 0.9;

fn towards_white(color: Color, amount: f32) -> Hsla {
    let mix = |v: u8| (v as f32 + (255.0 - v as f32) * amount).round() as u32;
    c((mix(color.r()) << 16) | (mix(color.g()) << 8) | mix(color.b()))
}

/// Draw a data bar in a cell whose box is `cell` (x0, y0, x1, y1 in window
/// coordinates, gridlines included at its right and bottom edges). The
/// bar leaves a pixel free along each edge; its axis runs over the whole
/// height of the cell.
pub fn bar(bar: &Bar, cell: (f32, f32, f32, f32), zoom: f32, out: &mut Vec<Draw>) {
    let gap = zoom.round().max(1.0);
    let (x0, y0, x1, y1) = (cell.0 + gap, cell.1 + gap, cell.2 - 1.0 - gap, cell.3 - 1.0 - gap);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let width = x1 - x0;
    let at = |fraction: f32| (x0 + fraction.clamp(0.0, 1.0) * width).round();
    let (base, tip) = (at(bar.start), at(bar.end));
    let (left, right) = (base.min(tip), base.max(tip));
    if right - left >= 1.0 {
        let bounds = Bounds::new(point(px(left), px(y0)), size(px(right - left), px(y1 - y0)));
        let background = if bar.gradient {
            // From the full colour to almost white in the direction the
            // bars of positive numbers grow: the bars of negative numbers,
            // which grow the other way, fade towards the axis.
            let rightwards = (tip >= base) != bar.negative;
            let angle = if rightwards { 90.0 } else { 270.0 };
            linear_gradient(
                angle,
                linear_color_stop(c(bar.fill.0), 0.0),
                linear_color_stop(towards_white(bar.fill, GRADIENT_FADE), 1.0),
            )
        } else {
            c(bar.fill.0).into()
        };
        out.push(Draw::Quad(quad(
            bounds,
            px(0.),
            background,
            px(if bar.border.is_some() { gap.min(right - left) } else { 0.0 }),
            bar.border.map(|color| c(color.0)).unwrap_or(transparent_black()),
            BorderStyle::Solid,
        )));
    }
    if let Some(axis) = bar.axis {
        let x = at(axis);
        let (dash, space) = (3.0 * gap, gap);
        let mut y = cell.1;
        let bottom = cell.3 - 1.0;
        while y < bottom {
            let end = (y + dash).min(bottom);
            out.push(Draw::Quad(fill(
                Bounds::new(point(px(x), px(y)), size(px(gap), px(end - y))),
                c(bar.axis_color.0),
            )));
            y += dash + space;
        }
    }
}

/// The colours of a scale side by side, blending from one to the next:
/// the preview of a colour scale.
pub fn scale_strip(colors: &[Color], bounds: Bounds<Pixels>, out: &mut Vec<Draw>) {
    match colors {
        [] => {}
        [only] => out.push(Draw::Quad(fill(bounds, c(only.0)))),
        _ => {
            let parts = colors.len() - 1;
            let width = f32::from(bounds.size.width) / parts as f32;
            for (i, pair) in colors.windows(2).enumerate() {
                let left = bounds.origin.x + px((i as f32 * width).floor());
                let right = if i + 1 == parts { bounds.right() } else { bounds.origin.x + px(((i + 1) as f32 * width).floor()) };
                out.push(Draw::Quad(fill(
                    Bounds::new(point(left, bounds.origin.y), size(right - left, bounds.size.height)),
                    linear_gradient(90.0, linear_color_stop(c(pair[0].0), 0.0), linear_color_stop(c(pair[1].0), 1.0)),
                )));
            }
        }
    }
}

// ----------------------------------------------------------------- elements

/// An element of the given size painted by `draw`, which receives the
/// bounds it got: how icons, bars and thumbnails are put in menus and
/// dialogs.
pub fn picture(width: f32, height: f32, draw: impl Fn(Bounds<Pixels>, &mut Vec<Draw>) + 'static) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, cx| {
            let mut items = Vec::new();
            draw(bounds, &mut items);
            crate::grid::paint_layers(vec![Layer { clip: bounds, items }], window, cx);
        },
    )
    .w(px(width))
    .h(px(height))
    .flex_none()
}

/// One icon as an element.
pub fn icon_picture(which: Icon, side: f32) -> impl IntoElement {
    picture(side, side, move |bounds, out| {
        icon(which, f32::from(bounds.origin.x), f32::from(bounds.origin.y), side, out)
    })
}

/// The icons of a set in a row, the one of the highest values first (as
/// the galleries show them).
pub fn set_picture(set: IconSet, side: f32, gap: f32) -> impl IntoElement {
    let icons: Vec<Icon> = set.icons().iter().rev().copied().collect();
    let width = icons.len() as f32 * (side + gap) - gap;
    picture(width, side, move |bounds, out| {
        for (i, which) in icons.iter().enumerate() {
            icon(
                *which,
                f32::from(bounds.origin.x) + i as f32 * (side + gap),
                f32::from(bounds.origin.y),
                side,
                out,
            );
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_icon() -> Vec<Icon> {
        let mut icons: Vec<Icon> = IconSet::ALL.iter().flat_map(|set| set.icons().iter().copied()).collect();
        icons.sort_by_key(|icon| *icon as u32);
        icons.dedup();
        icons
    }

    #[test]
    fn the_icon_sets_use_fifty_two_pictures() {
        assert_eq!(every_icon().len(), 52);
    }

    #[test]
    fn every_icon_draws_something_inside_its_box() {
        for which in every_icon() {
            for side in [8.0, 16.0, 40.0] {
                let mut out = Vec::new();
                icon(which, 100.0, 50.0, side, &mut out);
                assert!(!out.is_empty(), "{which:?} draws nothing");
                for item in &out {
                    if let Draw::Quad(q) = item {
                        let b = q.bounds;
                        assert!(
                            f32::from(b.left()) >= 99.9
                                && f32::from(b.top()) >= 49.9
                                && f32::from(b.right()) <= 100.1 + side
                                && f32::from(b.bottom()) <= 50.1 + side,
                            "{which:?} leaves its box at {side}: {b:?}"
                        );
                    }
                }
            }
        }
    }

    fn sample_bar(start: f32, end: f32) -> Bar {
        Bar {
            start,
            end,
            fill: Color(0x638EC6),
            gradient: false,
            border: None,
            negative: end < start,
            axis: None,
            axis_color: Color::BLACK,
        }
    }

    fn quads(out: &[Draw]) -> Vec<Bounds<Pixels>> {
        out.iter()
            .filter_map(|item| match item {
                Draw::Quad(q) => Some(q.bounds),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_bar_keeps_a_pixel_free_inside_the_cell() {
        let mut out = Vec::new();
        bar(&sample_bar(0.0, 1.0), (10.0, 20.0, 110.0, 40.0), 1.0, &mut out);
        let b = quads(&out)[0];
        assert_eq!(
            (f32::from(b.left()), f32::from(b.top()), f32::from(b.right()), f32::from(b.bottom())),
            (11.0, 21.0, 108.0, 38.0)
        );
    }

    #[test]
    fn a_negative_bar_grows_leftwards_from_the_axis() {
        let mut out = Vec::new();
        let mut negative = sample_bar(0.5, 0.25);
        negative.axis = Some(0.5);
        bar(&negative, (0.0, 0.0, 103.0, 20.0), 1.0, &mut out);
        let all = quads(&out);
        // The bar, then the dashes of the axis over the whole height.
        assert_eq!((f32::from(all[0].left()), f32::from(all[0].right())), (26.0, 51.0));
        assert!(all.len() > 3);
        assert!(all[1..].iter().all(|dash| f32::from(dash.left()) == 51.0 && f32::from(dash.size.width) == 1.0));
        assert_eq!(f32::from(all[1].top()), 0.0);
        assert_eq!(f32::from(all.last().unwrap().bottom()), 19.0);
    }

    #[test]
    fn gradients_fade_the_way_positive_bars_grow() {
        let angle = |bar: &Bar| {
            let mut out = Vec::new();
            super::bar(bar, (0.0, 0.0, 103.0, 20.0), 1.0, &mut out);
            match &out[0] {
                Draw::Quad(q) => format!("{:?}", q.background),
                _ => String::new(),
            }
        };
        let gradient = |start: f32, end: f32, negative: bool| Bar {
            gradient: true,
            negative,
            ..sample_bar(start, end)
        };
        // Left to right: a positive bar and a negative one fade alike.
        let positive = angle(&gradient(0.5, 1.0, false));
        let negative = angle(&gradient(0.5, 0.0, true));
        let mirrored = angle(&gradient(0.5, 0.0, false));
        assert_eq!(positive, negative);
        // Right to left, everything is mirrored.
        assert_ne!(positive, mirrored);
        assert_eq!(mirrored, angle(&gradient(0.5, 1.0, true)));
    }

    #[test]
    fn a_bar_of_no_length_draws_nothing() {
        let mut out = Vec::new();
        bar(&sample_bar(0.0, 0.0), (0.0, 0.0, 100.0, 20.0), 1.0, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn a_scale_strip_covers_its_box() {
        let mut out = Vec::new();
        let bounds = Bounds::new(point(px(5.), px(5.)), size(px(91.), px(10.)));
        scale_strip(&[Color(0xF8696B), Color(0xFFEB84), Color(0x63BE7B)], bounds, &mut out);
        let all = quads(&out);
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].left(), bounds.left());
        assert_eq!(all[0].right(), all[1].left());
        assert_eq!(all[1].right(), bounds.right());
    }
}
