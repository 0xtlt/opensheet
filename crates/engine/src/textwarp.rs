//! Warped text: the Transform gallery of Text Effects (`a:prstTxWarp` of
//! `a:bodyPr`).
//!
//! A preset is a handful of guide curves computed from the size of the
//! shape and one or two adjustments. `textwarp_presets.txt` holds the 40
//! presets as the OOXML standard defines them (its
//! `presetTextWarpDefinitions`): adjustments (`a`), guide formulas (`g`),
//! adjust handles (`x` for one that slides, `o` for one that turns about
//! the middle) and paths (`p`). The formulas are evaluated here as
//! written, so that a warp bends exactly where Excel's does.
//!
//! Two kinds of presets:
//! - most have their paths in pairs, a top and a bottom one: the text is
//!   stretched between the two, every point of a letter being moved, so
//!   that letters bend ([`LinePlace::Between`]);
//! - "Follow Path" (one path, or three for Button) sets each line of text
//!   on a path, letters keeping their shape and turning with the path
//!   ([`LinePlace::Along`]).
//!
//! Everything here is geometry: who draws the text gives the size of its
//! lines and gets back where each point of each letter goes.

use std::collections::HashMap;
use std::sync::OnceLock;

/// The presets of `a:prstTxWarp`, in the order of Excel's gallery: the
/// four of "Follow Path", then "Warp" row by row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WarpPreset {
    ArchUp,
    ArchDown,
    Circle,
    Button,
    Plain,
    Stop,
    Triangle,
    TriangleInverted,
    Chevron,
    ChevronInverted,
    RingInside,
    RingOutside,
    ArchUpPour,
    ArchDownPour,
    CirclePour,
    ButtonPour,
    CurveUp,
    CurveDown,
    CanUp,
    CanDown,
    Wave1,
    Wave2,
    DoubleWave1,
    Wave4,
    Inflate,
    Deflate,
    InflateBottom,
    DeflateBottom,
    InflateTop,
    DeflateTop,
    DeflateInflate,
    DeflateInflateDeflate,
    FadeRight,
    FadeLeft,
    FadeUp,
    FadeDown,
    SlantUp,
    SlantDown,
    CascadeUp,
    CascadeDown,
}

impl WarpPreset {
    pub const ALL: [WarpPreset; 40] = [
        WarpPreset::ArchUp,
        WarpPreset::ArchDown,
        WarpPreset::Circle,
        WarpPreset::Button,
        WarpPreset::Plain,
        WarpPreset::Stop,
        WarpPreset::Triangle,
        WarpPreset::TriangleInverted,
        WarpPreset::Chevron,
        WarpPreset::ChevronInverted,
        WarpPreset::RingInside,
        WarpPreset::RingOutside,
        WarpPreset::ArchUpPour,
        WarpPreset::ArchDownPour,
        WarpPreset::CirclePour,
        WarpPreset::ButtonPour,
        WarpPreset::CurveUp,
        WarpPreset::CurveDown,
        WarpPreset::CanUp,
        WarpPreset::CanDown,
        WarpPreset::Wave1,
        WarpPreset::Wave2,
        WarpPreset::DoubleWave1,
        WarpPreset::Wave4,
        WarpPreset::Inflate,
        WarpPreset::Deflate,
        WarpPreset::InflateBottom,
        WarpPreset::DeflateBottom,
        WarpPreset::InflateTop,
        WarpPreset::DeflateTop,
        WarpPreset::DeflateInflate,
        WarpPreset::DeflateInflateDeflate,
        WarpPreset::FadeRight,
        WarpPreset::FadeLeft,
        WarpPreset::FadeUp,
        WarpPreset::FadeDown,
        WarpPreset::SlantUp,
        WarpPreset::SlantDown,
        WarpPreset::CascadeUp,
        WarpPreset::CascadeDown,
    ];

    /// How many presets start the gallery under "Follow Path".
    pub const FOLLOW_PATH: usize = 4;

    /// The value of `prst` in a file.
    pub fn ooxml_name(self) -> &'static str {
        match self {
            WarpPreset::ArchUp => "textArchUp",
            WarpPreset::ArchDown => "textArchDown",
            WarpPreset::Circle => "textCircle",
            WarpPreset::Button => "textButton",
            WarpPreset::Plain => "textPlain",
            WarpPreset::Stop => "textStop",
            WarpPreset::Triangle => "textTriangle",
            WarpPreset::TriangleInverted => "textTriangleInverted",
            WarpPreset::Chevron => "textChevron",
            WarpPreset::ChevronInverted => "textChevronInverted",
            WarpPreset::RingInside => "textRingInside",
            WarpPreset::RingOutside => "textRingOutside",
            WarpPreset::ArchUpPour => "textArchUpPour",
            WarpPreset::ArchDownPour => "textArchDownPour",
            WarpPreset::CirclePour => "textCirclePour",
            WarpPreset::ButtonPour => "textButtonPour",
            WarpPreset::CurveUp => "textCurveUp",
            WarpPreset::CurveDown => "textCurveDown",
            WarpPreset::CanUp => "textCanUp",
            WarpPreset::CanDown => "textCanDown",
            WarpPreset::Wave1 => "textWave1",
            WarpPreset::Wave2 => "textWave2",
            WarpPreset::DoubleWave1 => "textDoubleWave1",
            WarpPreset::Wave4 => "textWave4",
            WarpPreset::Inflate => "textInflate",
            WarpPreset::Deflate => "textDeflate",
            WarpPreset::InflateBottom => "textInflateBottom",
            WarpPreset::DeflateBottom => "textDeflateBottom",
            WarpPreset::InflateTop => "textInflateTop",
            WarpPreset::DeflateTop => "textDeflateTop",
            WarpPreset::DeflateInflate => "textDeflateInflate",
            WarpPreset::DeflateInflateDeflate => "textDeflateInflateDeflate",
            WarpPreset::FadeRight => "textFadeRight",
            WarpPreset::FadeLeft => "textFadeLeft",
            WarpPreset::FadeUp => "textFadeUp",
            WarpPreset::FadeDown => "textFadeDown",
            WarpPreset::SlantUp => "textSlantUp",
            WarpPreset::SlantDown => "textSlantDown",
            WarpPreset::CascadeUp => "textCascadeUp",
            WarpPreset::CascadeDown => "textCascadeDown",
        }
    }

    /// The preset a file names; `textNoShape` and unknown names are none.
    pub fn from_ooxml_name(name: &str) -> Option<WarpPreset> {
        WarpPreset::ALL.into_iter().find(|preset| preset.ooxml_name() == name)
    }

    /// The name Excel shows for the preset, in English.
    pub fn name(self) -> &'static str {
        match self {
            WarpPreset::ArchUp => "Arch Up (Curve)",
            WarpPreset::ArchDown => "Arch Down (Curve)",
            WarpPreset::Circle => "Circle (Curve)",
            WarpPreset::Button => "Button (Curve)",
            WarpPreset::Plain => "Plain Text",
            WarpPreset::Stop => "Stop",
            WarpPreset::Triangle => "Triangle Up",
            WarpPreset::TriangleInverted => "Triangle Down",
            WarpPreset::Chevron => "Chevron Up",
            WarpPreset::ChevronInverted => "Chevron Down",
            WarpPreset::RingInside => "Ring Inside",
            WarpPreset::RingOutside => "Ring Outside",
            WarpPreset::ArchUpPour => "Arch Up (Pour)",
            WarpPreset::ArchDownPour => "Arch Down (Pour)",
            WarpPreset::CirclePour => "Circle (Pour)",
            WarpPreset::ButtonPour => "Button (Pour)",
            WarpPreset::CurveUp => "Curve Up",
            WarpPreset::CurveDown => "Curve Down",
            WarpPreset::CanUp => "Can Up",
            WarpPreset::CanDown => "Can Down",
            WarpPreset::Wave1 => "Wave 1",
            WarpPreset::Wave2 => "Wave 2",
            WarpPreset::DoubleWave1 => "Double Wave 1",
            WarpPreset::Wave4 => "Double Wave 2",
            WarpPreset::Inflate => "Inflate",
            WarpPreset::Deflate => "Deflate",
            WarpPreset::InflateBottom => "Inflate Bottom",
            WarpPreset::DeflateBottom => "Deflate Bottom",
            WarpPreset::InflateTop => "Inflate Top",
            WarpPreset::DeflateTop => "Deflate Top",
            WarpPreset::DeflateInflate => "Deflate Inflate",
            WarpPreset::DeflateInflateDeflate => "Deflate Inflate Deflate",
            WarpPreset::FadeRight => "Fade Right",
            WarpPreset::FadeLeft => "Fade Left",
            WarpPreset::FadeUp => "Fade Up",
            WarpPreset::FadeDown => "Fade Down",
            WarpPreset::SlantUp => "Slant Up",
            WarpPreset::SlantDown => "Slant Down",
            WarpPreset::CascadeUp => "Cascade Up",
            WarpPreset::CascadeDown => "Cascade Down",
        }
    }

    fn definition(self) -> &'static Definition {
        let name = self.ooxml_name();
        definitions().iter().find(|definition| definition.name == name).expect("every preset is defined")
    }

    /// True for the presets that set letters on a path without bending
    /// them: their paths do not come in pairs.
    pub fn follows_path(self) -> bool {
        self.definition().paths.len() % 2 == 1
    }

    /// The names of the adjustments of the preset (`adj`, or `adj1` and
    /// `adj2`) with their values in a new warp.
    pub fn adjustments(self) -> Vec<(&'static str, i32)> {
        self.definition().adjust.iter().map(|(name, value)| (*name, *value)).collect()
    }
}

/// The warp of the text of a shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextWarp {
    pub preset: WarpPreset,
    /// The adjustments (`a:gd` of `a:avLst`), in the order of
    /// [`WarpPreset::adjustments`]; `None` is the preset's own value.
    pub adjust: [Option<i32>; 2],
}

impl From<WarpPreset> for TextWarp {
    fn from(preset: WarpPreset) -> TextWarp {
        TextWarp {
            preset,
            adjust: [None, None],
        }
    }
}

// ---- definitions ---------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum Step {
    Move(&'static str, &'static str),
    Line(&'static str, &'static str),
    Quad([&'static str; 4]),
    Cubic([&'static str; 6]),
    /// Radii, start angle and swing.
    Arc([&'static str; 4]),
}

/// An adjustment a handle changes, with the least and the most it may
/// become (names of guides or numbers).
#[derive(Clone, Copy, Debug)]
struct Reference {
    adjust: usize,
    min: &'static str,
    max: &'static str,
}

#[derive(Clone, Debug)]
struct HandleDefinition {
    /// The handle turns about the middle of the shape: its first
    /// reference is an angle, its second a distance from the middle.
    polar: bool,
    first: Option<Reference>,
    second: Option<Reference>,
    at: (&'static str, &'static str),
}

struct Definition {
    name: &'static str,
    adjust: Vec<(&'static str, i32)>,
    guides: Vec<(&'static str, Vec<&'static str>)>,
    handles: Vec<HandleDefinition>,
    paths: Vec<Vec<Step>>,
}

/// A turn in the units of the definitions: 60 000ths of a degree.
const TURN: f64 = 21_600_000.0;

fn definitions() -> &'static [Definition] {
    static DEFINITIONS: OnceLock<Vec<Definition>> = OnceLock::new();
    DEFINITIONS.get_or_init(|| parse_definitions(include_str!("textwarp_presets.txt")))
}

fn parse_definitions(source: &'static str) -> Vec<Definition> {
    let mut out: Vec<Definition> = Vec::new();
    for line in source.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let words: Vec<&'static str> = line.split_whitespace().collect();
        if words.len() == 1 {
            out.push(Definition {
                name: words[0],
                adjust: Vec::new(),
                guides: Vec::new(),
                handles: Vec::new(),
                paths: Vec::new(),
            });
            continue;
        }
        let Some(definition) = out.last_mut() else {
            continue;
        };
        match words[0] {
            // `a adj val 50000`; the one angle is given by name.
            "a" => {
                let value = match words[3] {
                    "cd2" => 10_800_000,
                    "cd4" => 5_400_000,
                    number => number.parse().unwrap_or(0),
                };
                definition.adjust.push((words[1], value));
            }
            "g" => definition.guides.push((words[1], words[2..].to_vec())),
            "x" | "o" => {
                let reference = |at: usize| {
                    let adjust = definition.adjust.iter().position(|(name, _)| *name == words[at])?;
                    Some(Reference {
                        adjust,
                        min: words[at + 1],
                        max: words[at + 2],
                    })
                };
                definition.handles.push(HandleDefinition {
                    polar: words[0] == "o",
                    first: reference(1),
                    second: reference(4),
                    at: (words[7], words[8]),
                });
            }
            "p" => {
                let mut steps = Vec::new();
                let mut at = 1;
                while at < words.len() {
                    let (step, taken) = match words[at] {
                        "M" => (Step::Move(words[at + 1], words[at + 2]), 2),
                        "L" => (Step::Line(words[at + 1], words[at + 2]), 2),
                        "Q" => (Step::Quad([words[at + 1], words[at + 2], words[at + 3], words[at + 4]]), 4),
                        "C" => (
                            Step::Cubic([
                                words[at + 1],
                                words[at + 2],
                                words[at + 3],
                                words[at + 4],
                                words[at + 5],
                                words[at + 6],
                            ]),
                            6,
                        ),
                        _ => (Step::Arc([words[at + 1], words[at + 2], words[at + 3], words[at + 4]]), 4),
                    };
                    steps.push(step);
                    at += 1 + taken;
                }
                definition.paths.push(steps);
            }
            _ => {}
        }
    }
    out
}

/// The values of the guides of a preset in a shape of a given size.
struct Guides {
    values: HashMap<&'static str, f64>,
}

impl Guides {
    fn new(definition: &Definition, adjust: &[f64], width: f64, height: f64) -> Guides {
        let (short, long) = (width.min(height), width.max(height));
        let mut values: HashMap<&'static str, f64> = HashMap::from([
            ("w", width),
            ("h", height),
            ("l", 0.0),
            ("t", 0.0),
            ("r", width),
            ("b", height),
            ("hc", width / 2.0),
            ("vc", height / 2.0),
            ("wd2", width / 2.0),
            ("wd3", width / 3.0),
            ("wd4", width / 4.0),
            ("wd5", width / 5.0),
            ("wd6", width / 6.0),
            ("wd8", width / 8.0),
            ("wd10", width / 10.0),
            ("wd32", width / 32.0),
            ("hd2", height / 2.0),
            ("hd3", height / 3.0),
            ("hd4", height / 4.0),
            ("hd5", height / 5.0),
            ("hd6", height / 6.0),
            ("hd8", height / 8.0),
            ("ss", short),
            ("ls", long),
            ("ssd2", short / 2.0),
            ("ssd4", short / 4.0),
            ("ssd6", short / 6.0),
            ("ssd8", short / 8.0),
            ("ssd16", short / 16.0),
            ("ssd32", short / 32.0),
            ("cd2", TURN / 2.0),
            ("cd4", TURN / 4.0),
            ("cd8", TURN / 8.0),
            ("3cd4", TURN * 3.0 / 4.0),
            ("3cd8", TURN * 3.0 / 8.0),
            ("5cd8", TURN * 5.0 / 8.0),
            ("7cd8", TURN * 7.0 / 8.0),
        ]);
        for (i, (name, _)) in definition.adjust.iter().enumerate() {
            values.insert(name, adjust.get(i).copied().unwrap_or(0.0));
        }
        let mut guides = Guides { values };
        for (name, formula) in &definition.guides {
            let value = guides.formula(formula);
            guides.values.insert(name, value);
        }
        guides
    }

    /// A guide by its name, or a number written out.
    fn get(&self, name: &str) -> f64 {
        match self.values.get(name) {
            Some(value) => *value,
            None => name.parse().unwrap_or(0.0),
        }
    }

    /// One formula of the standard: an operator and up to three operands.
    fn formula(&self, words: &[&str]) -> f64 {
        let arg = |i: usize| words.get(i).map_or(0.0, |name| self.get(name));
        let (x, y, z) = (arg(1), arg(2), arg(3));
        let radians = |angle: f64| (angle / TURN * 360.0).to_radians();
        match words.first().copied().unwrap_or("") {
            "val" => x,
            "*/" => {
                if z == 0.0 {
                    0.0
                } else {
                    x * y / z
                }
            }
            "+-" => x + y - z,
            "+/" => {
                if z == 0.0 {
                    0.0
                } else {
                    (x + y) / z
                }
            }
            "?:" => {
                if x > 0.0 {
                    y
                } else {
                    z
                }
            }
            "abs" => x.abs(),
            "pin" => {
                if y < x {
                    x
                } else if y > z {
                    z
                } else {
                    y
                }
            }
            "sin" => x * radians(y).sin(),
            "cos" => x * radians(y).cos(),
            "cat2" => x * z.atan2(y).cos(),
            "sat2" => x * z.atan2(y).sin(),
            "sqrt" => x.max(0.0).sqrt(),
            "max" => x.max(y),
            "min" => x.min(y),
            _ => 0.0,
        }
    }
}

// ---- curves ----------------------------------------------------------------

/// A guide curve as a chain of short straight pieces, with the length
/// travelled at each of its points.
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    pub points: Vec<(f64, f64)>,
    lengths: Vec<f64>,
}

impl Curve {
    fn new(points: Vec<(f64, f64)>) -> Curve {
        let mut lengths = Vec::with_capacity(points.len());
        let mut total = 0.0;
        for (i, point) in points.iter().enumerate() {
            if i > 0 {
                total += (point.0 - points[i - 1].0).hypot(point.1 - points[i - 1].1);
            }
            lengths.push(total);
        }
        Curve { points, lengths }
    }

    pub fn length(&self) -> f64 {
        self.lengths.last().copied().unwrap_or(0.0)
    }

    /// The point `distance` along the curve and the direction of travel
    /// there (a unit vector). Past its ends, the curve goes on straight.
    /// The direction is that of the chord about the point, so that it
    /// turns evenly along a curve cut into straight pieces.
    pub fn at(&self, distance: f64) -> ((f64, f64), (f64, f64)) {
        let (point, piece) = self.on_piece(distance);
        let reach = self.length() / 500.0;
        let (before, after) = (self.on_piece(distance - reach).0, self.on_piece(distance + reach).0);
        let chord = (after.0 - before.0).hypot(after.1 - before.1);
        if chord > 0.0 {
            (point, ((after.0 - before.0) / chord, (after.1 - before.1) / chord))
        } else {
            (point, piece)
        }
    }

    /// The point `distance` along the curve and the direction of the
    /// straight piece it is on.
    fn on_piece(&self, distance: f64) -> ((f64, f64), (f64, f64)) {
        let count = self.points.len();
        if count == 0 {
            return ((0.0, 0.0), (1.0, 0.0));
        }
        if count == 1 || self.length() <= 0.0 {
            return (self.points[0], (1.0, 0.0));
        }
        // The piece the distance falls in; the first or the last one for
        // a distance before the start or after the end.
        let after = self.lengths.partition_point(|length| *length < distance).clamp(1, count - 1);
        let mut from = after - 1;
        // Pieces of no length (a doubled point) have no direction.
        while from > 0 && self.lengths[after] - self.lengths[from] <= 0.0 {
            from -= 1;
        }
        let (a, b) = (self.points[from], self.points[after]);
        let piece = (self.lengths[after] - self.lengths[from]).max(f64::MIN_POSITIVE);
        let direction = ((b.0 - a.0) / piece, (b.1 - a.1) / piece);
        let along = distance - self.lengths[from];
        ((a.0 + direction.0 * along, a.1 + direction.1 * along), direction)
    }

    /// The point a fraction of the way along the curve.
    pub fn at_fraction(&self, fraction: f64) -> (f64, f64) {
        self.at(fraction * self.length()).0
    }
}

/// How many straight pieces a curved step is cut into.
const PIECES: usize = 64;

fn curve(steps: &[Step], guides: &Guides) -> Curve {
    let mut points: Vec<(f64, f64)> = Vec::new();
    let mut pen = (0.0, 0.0);
    let pt = |x: &str, y: &str| (guides.get(x), guides.get(y));
    for step in steps {
        match *step {
            Step::Move(x, y) | Step::Line(x, y) => {
                pen = pt(x, y);
                points.push(pen);
            }
            Step::Quad([cx, cy, x, y]) => {
                let (control, end) = (pt(cx, cy), pt(x, y));
                for i in 1..=PIECES {
                    let t = i as f64 / PIECES as f64;
                    let s = 1.0 - t;
                    points.push((
                        s * s * pen.0 + 2.0 * s * t * control.0 + t * t * end.0,
                        s * s * pen.1 + 2.0 * s * t * control.1 + t * t * end.1,
                    ));
                }
                pen = end;
            }
            Step::Cubic([ax, ay, bx, by, x, y]) => {
                let (a, b, end) = (pt(ax, ay), pt(bx, by), pt(x, y));
                for i in 1..=PIECES {
                    let t = i as f64 / PIECES as f64;
                    let s = 1.0 - t;
                    let (k0, k1, k2, k3) = (s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t);
                    points.push((
                        k0 * pen.0 + k1 * a.0 + k2 * b.0 + k3 * end.0,
                        k0 * pen.1 + k1 * a.1 + k2 * b.1 + k3 * end.1,
                    ));
                }
                pen = end;
            }
            Step::Arc([rx, ry, start, swing]) => {
                let (rx, ry) = (guides.get(rx), guides.get(ry));
                let (start, swing) = (guides.get(start), guides.get(swing));
                // The angles of a file are those seen on the page; on an
                // ellipse the angle that runs evenly round it differs,
                // except at each quarter turn.
                let even = |angle: f64| {
                    let radians = (angle / TURN * 360.0).to_radians();
                    let turns = ((radians + std::f64::consts::PI) / std::f64::consts::TAU).floor();
                    let within = radians - turns * std::f64::consts::TAU;
                    (rx * within.sin()).atan2(ry * within.cos()) + turns * std::f64::consts::TAU
                };
                let (from, to) = if rx > 0.0 && ry > 0.0 {
                    (even(start), even(start + swing))
                } else {
                    (0.0, 0.0)
                };
                let centre = (pen.0 - rx * from.cos(), pen.1 - ry * from.sin());
                let pieces = ((to - from).abs() / std::f64::consts::TAU * 180.0).ceil().max(1.0) as usize;
                for i in 1..=pieces {
                    let angle = from + (to - from) * i as f64 / pieces as f64;
                    points.push((centre.0 + rx * angle.cos(), centre.1 + ry * angle.sin()));
                }
                pen = *points.last().unwrap_or(&pen);
            }
        }
    }
    Curve::new(points)
}

// ---- the warp of one shape -------------------------------------------------

/// A handle that changes an adjustment of the warp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WarpHandle {
    /// Where it is drawn, from the top-left corner of the shape.
    pub at: (f64, f64),
}

/// The guide curves of a warp in a shape of a given size.
#[derive(Clone, Debug, PartialEq)]
pub struct WarpGeometry {
    pub curves: Vec<Curve>,
    pub handles: Vec<WarpHandle>,
    follows_path: bool,
}

/// A line of text as it is before being warped: `width` long from its
/// left end, `ascent` above its baseline and `descent` below it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WarpLine {
    pub width: f64,
    pub ascent: f64,
    pub descent: f64,
}

/// How a line of text sits against its neighbours.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WarpAlign {
    Left,
    Centre,
    Right,
}

/// Where a line of text goes in a warp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LinePlace {
    /// Stretched between two curves: across, the line runs from the
    /// fraction `u0` of the curves to `u1`; down, from `v0` (0 on the top
    /// curve, 1 on the bottom one) at its ascent to `v1` at its descent.
    Between {
        top: usize,
        bottom: usize,
        u: (f64, f64),
        v: (f64, f64),
    },
    /// Set on a curve: the left end of the line is `start` along it,
    /// lengths are multiplied by `scale`, and the baseline runs `drop`
    /// below the curve (a second line under a first one).
    Along {
        curve: usize,
        start: f64,
        scale: f64,
        drop: f64,
    },
}

impl TextWarp {
    /// The values of the adjustments, each within what its handle allows.
    fn values(&self, definition: &Definition) -> Vec<f64> {
        definition
            .adjust
            .iter()
            .enumerate()
            .map(|(i, (_, own))| self.adjust.get(i).copied().flatten().unwrap_or(*own) as f64)
            .collect()
    }

    /// The adjustments as a file lists them: name and value, the preset's
    /// own values included.
    pub fn adjustments(&self) -> Vec<(&'static str, i32)> {
        let definition = self.preset.definition();
        definition
            .adjust
            .iter()
            .enumerate()
            .map(|(i, (name, own))| (*name, self.adjust.get(i).copied().flatten().unwrap_or(*own)))
            .collect()
    }

    /// Set an adjustment by its name in a file; the preset's own value
    /// is no adjustment.
    pub fn set_adjustment(&mut self, name: &str, value: i32) {
        let adjust = &self.preset.definition().adjust;
        if let Some(i) = adjust.iter().position(|(known, _)| *known == name) {
            if let Some(slot) = self.adjust.get_mut(i) {
                *slot = (value != adjust[i].1).then_some(value);
            }
        }
    }

    /// The guide curves and the handles of the warp in a shape `width`
    /// by `height`.
    pub fn geometry(&self, width: f64, height: f64) -> WarpGeometry {
        let definition = self.preset.definition();
        let guides = Guides::new(definition, &self.values(definition), width.max(0.0), height.max(0.0));
        WarpGeometry {
            curves: definition.paths.iter().map(|steps| curve(steps, &guides)).collect(),
            handles: definition
                .handles
                .iter()
                .map(|handle| WarpHandle {
                    at: (guides.get(handle.at.0), guides.get(handle.at.1)),
                })
                .collect(),
            follows_path: definition.paths.len() % 2 == 1,
        }
    }

    /// The warp after one of its handles was dragged to a point of the
    /// shape: the adjustments the handle stands for take the values, in
    /// their ranges, that bring it nearest to the point.
    pub fn with_handle_at(&self, handle: usize, to: (f64, f64), width: f64, height: f64) -> TextWarp {
        let definition = self.preset.definition();
        let Some(dragged) = definition.handles.get(handle) else {
            return *self;
        };
        let mut warp = *self;
        // The range of an adjustment may depend on the size of the shape.
        let bounds = |reference: &Reference, warp: &TextWarp| {
            let guides = Guides::new(definition, &warp.values(definition), width, height);
            (guides.get(reference.min), guides.get(reference.max))
        };
        let distance = |warp: &TextWarp| {
            let guides = Guides::new(definition, &warp.values(definition), width, height);
            (guides.get(dragged.at.0) - to.0).hypot(guides.get(dragged.at.1) - to.1)
        };
        if let (true, Some(angle)) = (dragged.polar, dragged.first) {
            // An angle is the one the point is seen at from the middle.
            let seen = (to.1 - height / 2.0).atan2(to.0 - width / 2.0).to_degrees().rem_euclid(360.0);
            let (min, max) = bounds(&angle, &warp);
            warp.adjust[angle.adjust] = Some((seen / 360.0 * TURN).clamp(min, max).round() as i32);
        }
        // What slides is searched for: the place of a handle is given by
        // the formulas, not the other way round.
        let slides = if dragged.polar { [dragged.second, None] } else { [dragged.first, dragged.second] };
        for reference in slides.into_iter().flatten() {
            let (min, max) = bounds(&reference, &warp);
            let mut best = (f64::INFINITY, min);
            let (mut low, mut high) = (min, max);
            for _ in 0..4 {
                const STEPS: usize = 50;
                for i in 0..=STEPS {
                    let value = low + (high - low) * i as f64 / STEPS as f64;
                    warp.adjust[reference.adjust] = Some(value.round() as i32);
                    let far = distance(&warp);
                    if far < best.0 {
                        best = (far, value);
                    }
                }
                let reach = (high - low) / STEPS as f64;
                (low, high) = ((best.1 - reach).max(min), (best.1 + reach).min(max));
            }
            warp.adjust[reference.adjust] = Some(best.1.round() as i32);
        }
        warp
    }
}

impl WarpGeometry {
    /// True when letters are set on the curves instead of being bent
    /// between them.
    pub fn follows_path(&self) -> bool {
        self.follows_path
    }

    /// How many lines of text the warp has a place for: one per curve to
    /// follow, or one per pair of curves.
    pub fn places(&self) -> usize {
        if self.follows_path { self.curves.len() } else { self.curves.len() / 2 }
    }

    /// Where the lines of a text go. Lines are dealt out to the places of
    /// the warp in order; with more lines than places, neighbours share
    /// one, stacked. Between two curves the widest line of the text
    /// spans the curves from end to end, shorter ones are aligned
    /// against it; on a curve to follow, a line keeps its size unless it
    /// is longer than the curve.
    pub fn place(&self, lines: &[WarpLine], align: WarpAlign) -> Vec<LinePlace> {
        let places = self.places();
        if places == 0 || lines.is_empty() {
            return Vec::new();
        }
        let place_of = |line: usize| if lines.len() <= places { line } else { line * places / lines.len() };
        let widest = lines.iter().map(|line| line.width).fold(0.0, f64::max).max(f64::MIN_POSITIVE);
        let aligned = |room: f64| match align {
            WarpAlign::Left => 0.0,
            WarpAlign::Centre => room / 2.0,
            WarpAlign::Right => room,
        };
        let mut out = Vec::with_capacity(lines.len());
        for (i, line) in lines.iter().enumerate() {
            let place = place_of(i);
            // The lines that share the place, and the heights above this one.
            let sharing: Vec<&WarpLine> = (0..lines.len()).filter(|other| place_of(*other) == place).map(|other| &lines[other]).collect();
            let first = (0..lines.len()).find(|other| place_of(*other) == place).unwrap_or(i);
            let total: f64 = sharing.iter().map(|line| line.ascent + line.descent).sum::<f64>().max(f64::MIN_POSITIVE);
            let above: f64 = lines[first..i].iter().map(|line| line.ascent + line.descent).sum();
            if self.follows_path {
                let length = self.curves[place].length();
                let scale = if line.width > length && line.width > 0.0 { length / line.width } else { 1.0 };
                out.push(LinePlace::Along {
                    curve: place,
                    start: aligned(length - line.width * scale),
                    scale,
                    drop: (above + line.ascent - lines[first].ascent) * scale,
                });
            } else {
                let share = line.width / widest;
                let from = aligned(1.0 - share);
                out.push(LinePlace::Between {
                    top: 2 * place,
                    bottom: 2 * place + 1,
                    u: (from, from + share),
                    v: (above / total, (above + line.ascent + line.descent) / total),
                });
            }
        }
        out
    }

    /// Where a point of a letter goes: `x` from the left end of its line,
    /// `y` below its baseline (negative above), `centre` the middle of
    /// the letter along the line (a letter set on a path turns about the
    /// point of the path under its middle and keeps its shape).
    pub fn map(&self, place: &LinePlace, line: &WarpLine, x: f64, y: f64, centre: f64) -> (f64, f64) {
        match *place {
            LinePlace::Between { top, bottom, u, v } => {
                let across = if line.width > 0.0 { u.0 + (u.1 - u.0) * x / line.width } else { u.0 };
                let height = (line.ascent + line.descent).max(f64::MIN_POSITIVE);
                let down = v.0 + (v.1 - v.0) * (y + line.ascent) / height;
                let (a, b) = (self.curves[top].at_fraction(across), self.curves[bottom].at_fraction(across));
                (a.0 + (b.0 - a.0) * down, a.1 + (b.1 - a.1) * down)
            }
            LinePlace::Along { curve, start, scale, drop } => {
                let (on, direction) = self.curves[curve].at(start + centre * scale);
                // Down from the baseline is to the right of the way the
                // curve runs.
                let normal = (-direction.1, direction.0);
                let (along, down) = ((x - centre) * scale, y * scale + drop);
                (on.0 + direction.0 * along + normal.0 * down, on.1 + direction.1 * along + normal.1 * down)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: (f64, f64), b: (f64, f64)) -> bool {
        (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01
    }

    #[test]
    fn every_preset_is_defined_and_its_formulas_resolve() {
        assert_eq!(definitions().len(), 40);
        for preset in WarpPreset::ALL {
            assert_eq!(WarpPreset::from_ooxml_name(preset.ooxml_name()), Some(preset));
            let definition = preset.definition();
            // Every word of every formula, path and handle is a number, a
            // built-in name or a guide defined before it is used.
            let guides = Guides::new(definition, &TextWarp::from(preset).values(definition), 400.0, 200.0);
            let known = |name: &str| guides.values.contains_key(name) || name.parse::<f64>().is_ok();
            for (name, formula) in &definition.guides {
                assert!(formula[1..].iter().all(|word| known(word)), "{} {name}: {formula:?}", definition.name);
            }
            for steps in &definition.paths {
                for step in steps {
                    let words: Vec<&str> = match *step {
                        Step::Move(x, y) | Step::Line(x, y) => vec![x, y],
                        Step::Quad(words) | Step::Arc(words) => words.to_vec(),
                        Step::Cubic(words) => words.to_vec(),
                    };
                    assert!(words.iter().all(|word| known(word)), "{}: {words:?}", definition.name);
                }
            }
            let geometry = TextWarp::from(preset).geometry(400.0, 200.0);
            assert!(!geometry.curves.is_empty() && geometry.curves.iter().all(|curve| curve.length() > 0.0), "{preset:?}");
            assert_eq!(geometry.handles.len(), definition.handles.len());
            assert!(!definition.adjust.is_empty() && definition.adjust.len() <= 2);
            // Everything stays finite in a shape without size.
            let flat = TextWarp::from(preset).geometry(0.0, 0.0);
            assert!(flat.curves.iter().flat_map(|curve| &curve.points).all(|p| p.0.is_finite() && p.1.is_finite()));
        }
        assert_eq!(WarpPreset::from_ooxml_name("textNoShape"), None);
        let followers: Vec<WarpPreset> = WarpPreset::ALL.into_iter().filter(|preset| preset.follows_path()).collect();
        assert_eq!(followers, WarpPreset::ALL[..WarpPreset::FOLLOW_PATH]);
    }

    #[test]
    fn guide_curves_are_where_the_standard_puts_them() {
        // Plain: the two long sides of the shape.
        let plain = TextWarp::from(WarpPreset::Plain).geometry(400.0, 200.0);
        assert_eq!(plain.curves[0].points, vec![(0.0, 0.0), (400.0, 0.0)]);
        assert_eq!(plain.curves[1].points, vec![(0.0, 200.0), (400.0, 200.0)]);
        // Triangle: the top rises to the middle from half the height.
        let triangle = TextWarp::from(WarpPreset::Triangle).geometry(400.0, 200.0);
        assert_eq!(triangle.curves[0].points, vec![(0.0, 100.0), (200.0, 0.0), (400.0, 100.0)]);
        assert_eq!(triangle.handles[0].at, (0.0, 100.0));
        // Arch Up: the upper half of the ellipse, from left to right.
        let arch = TextWarp::from(WarpPreset::ArchUp).geometry(400.0, 200.0);
        let curve = &arch.curves[0];
        assert!(close(curve.points[0], (0.0, 100.0)));
        assert!(close(curve.at_fraction(0.5), (200.0, 0.0)));
        assert!(close(*curve.points.last().unwrap(), (400.0, 100.0)));
        // Arch Down: the lower half, also from left to right.
        let arch = TextWarp::from(WarpPreset::ArchDown).geometry(400.0, 200.0);
        let curve = &arch.curves[0];
        assert!(close(curve.points[0], (0.0, 100.0)), "{:?}", curve.points[0]);
        assert!(close(curve.at_fraction(0.5), (200.0, 200.0)));
        // Circle: the whole ellipse, clockwise from the left.
        let circle = TextWarp::from(WarpPreset::Circle).geometry(200.0, 200.0);
        let curve = &circle.curves[0];
        assert!((curve.length() - std::f64::consts::TAU * 100.0).abs() < 0.5);
        assert!(close(curve.at_fraction(0.25), (100.0, 0.0)));
        // Slant Up: both sides rise to the right.
        let slant = TextWarp::from(WarpPreset::SlantUp).geometry(400.0, 200.0);
        assert!(slant.curves[0].points[0].1 > slant.curves[0].points[1].1);
        // Inflate: the top bulges up in the middle, the bottom down.
        let inflate = TextWarp::from(WarpPreset::Inflate).geometry(400.0, 200.0);
        assert!(inflate.curves[0].at_fraction(0.5).1 < inflate.curves[0].points[0].1);
        assert!(inflate.curves[1].at_fraction(0.5).1 > inflate.curves[1].points[0].1);
        // Deflate Inflate Deflate has three bands, Button three paths.
        assert_eq!(TextWarp::from(WarpPreset::DeflateInflateDeflate).geometry(400.0, 200.0).places(), 3);
        assert_eq!(TextWarp::from(WarpPreset::Button).geometry(400.0, 200.0).places(), 3);
        // On an ellipse an angle is the one seen on the page: an arch
        // that starts at 135° starts on the diagonal of a square only.
        let warp = TextWarp {
            preset: WarpPreset::ArchUp,
            adjust: [Some(8_100_000), None],
        };
        let start = warp.geometry(400.0, 200.0).curves[0].points[0];
        assert!(((start.1 - 100.0) / (start.0 - 200.0) + 1.0).abs() < 1e-6, "{start:?}");
        assert!(close(warp.geometry(400.0, 200.0).handles[0].at, start));
    }

    #[test]
    fn text_is_stretched_between_two_curves() {
        let geometry = TextWarp::from(WarpPreset::Triangle).geometry(400.0, 200.0);
        let long = WarpLine {
            width: 100.0,
            ascent: 8.0,
            descent: 2.0,
        };
        let short = WarpLine { width: 50.0, ..long };
        let places = geometry.place(&[long], WarpAlign::Centre);
        assert_eq!(
            places,
            vec![LinePlace::Between {
                top: 0,
                bottom: 1,
                u: (0.0, 1.0),
                v: (0.0, 1.0)
            }]
        );
        // The corners of the line go to the ends of the curves; the top
        // of its middle to the point of the triangle.
        assert!(close(geometry.map(&places[0], &long, 0.0, -8.0, 0.0), (0.0, 100.0)));
        assert!(close(geometry.map(&places[0], &long, 50.0, -8.0, 0.0), (200.0, 0.0)));
        assert!(close(geometry.map(&places[0], &long, 100.0, 2.0, 0.0), (400.0, 200.0)));
        // The baseline is four fifths of the way down.
        assert!(close(geometry.map(&places[0], &long, 50.0, 0.0, 0.0), (200.0, 160.0)));
        // Two lines share the height; the shorter one keeps its
        // proportion and is aligned against the longer.
        let places = geometry.place(&[long, short], WarpAlign::Right);
        assert_eq!(
            places[1],
            LinePlace::Between {
                top: 0,
                bottom: 1,
                u: (0.5, 1.0),
                v: (0.5, 1.0)
            }
        );
        // Three bands take a line each; six lines go two by two.
        let bands = TextWarp::from(WarpPreset::DeflateInflateDeflate).geometry(400.0, 300.0);
        let tops = |count: usize| -> Vec<usize> {
            bands
                .place(&vec![long; count], WarpAlign::Left)
                .iter()
                .map(|place| match place {
                    LinePlace::Between { top, .. } => *top,
                    LinePlace::Along { curve, .. } => *curve,
                })
                .collect()
        };
        assert_eq!(tops(2), vec![0, 2]);
        assert_eq!(tops(6), vec![0, 0, 2, 2, 4, 4]);
    }

    #[test]
    fn letters_follow_a_path_without_bending() {
        let geometry = TextWarp::from(WarpPreset::ArchUp).geometry(200.0, 200.0);
        let line = WarpLine {
            width: 20.0,
            ascent: 8.0,
            descent: 2.0,
        };
        let length = geometry.curves[0].length();
        let places = geometry.place(&[line], WarpAlign::Centre);
        let LinePlace::Along { start, scale, drop, .. } = places[0] else {
            panic!("an arch is followed");
        };
        assert!((start - (length - 20.0) / 2.0).abs() < 1e-9 && scale == 1.0 && drop == 0.0);
        // A letter in the middle of the line sits on the top of the arch,
        // upright: its foot on the curve, its head outside.
        assert!(close(geometry.map(&places[0], &line, 10.0, 0.0, 10.0), (100.0, 0.0)));
        assert!(close(geometry.map(&places[0], &line, 10.0, -8.0, 10.0), (100.0, -8.0)));
        // Its corners keep their distances: it is turned, not bent.
        let (a, b) = (geometry.map(&places[0], &line, 0.0, 0.0, 2.0), geometry.map(&places[0], &line, 4.0, -8.0, 2.0));
        assert!(((a.0 - b.0).hypot(a.1 - b.1) - 4f64.hypot(8.0)).abs() < 1e-9);
        // A line longer than the curve is made to fit.
        let long = WarpLine { width: 2.0 * length, ..line };
        assert_eq!(
            geometry.place(&[long], WarpAlign::Left),
            vec![LinePlace::Along {
                curve: 0,
                start: 0.0,
                scale: 0.5,
                drop: 0.0
            }]
        );
        // A second line on the one path runs under the first.
        let places = geometry.place(&[line, line], WarpAlign::Left);
        assert!(matches!(places[1], LinePlace::Along { drop, .. } if drop == 10.0));
    }

    #[test]
    fn dragging_a_handle_changes_its_adjustment() {
        // A handle that slides: the top of a triangle's sides.
        let warp = TextWarp::from(WarpPreset::Triangle);
        let moved = warp.with_handle_at(0, (-20.0, 50.0), 400.0, 200.0);
        assert_eq!(moved.adjust, [Some(25_000), None]);
        assert_eq!(moved.geometry(400.0, 200.0).handles[0].at, (0.0, 50.0));
        // It stops at the end of its range.
        assert_eq!(warp.with_handle_at(0, (0.0, 900.0), 400.0, 200.0).adjust, [Some(100_000), None]);
        // A handle that turns: the start of an arch.
        let warp = TextWarp::from(WarpPreset::ArchUp);
        let moved = warp.with_handle_at(0, (100.0, -500.0), 200.0, 200.0);
        assert_eq!(moved.adjust, [Some(16_200_000), None]);
        // Two adjustments on one handle: the angle and how thick the band is.
        let warp = TextWarp::from(WarpPreset::ArchUpPour);
        let moved = warp.with_handle_at(0, (40.0, 100.0), 200.0, 200.0);
        assert_eq!(moved.adjust[0], Some(10_800_000));
        let at = moved.geometry(200.0, 200.0).handles[0].at;
        assert!((at.0 - 40.0).abs() < 0.5 && (at.1 - 100.0).abs() < 0.5, "{at:?}");
        // The second handle of a wave slants it.
        let warp = TextWarp::from(WarpPreset::Wave1);
        let moved = warp.with_handle_at(1, (220.0, 200.0), 400.0, 200.0);
        assert_eq!(moved.adjust[0], None);
        assert_eq!(moved.adjust[1], Some(5_000));
        assert_eq!(moved.adjustments(), vec![("adj1", 12_500), ("adj2", 5_000)]);
    }
}
