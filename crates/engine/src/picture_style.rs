//! What the Picture Styles group of the Picture Format tab gives a
//! picture besides a shadow, a glow and soft edges (`drawing.rs`): a
//! reflection, a bevel, a rotation in space, the styles of the gallery
//! that combine them, and the artistic effects.
//!
//! Everything here is described, not drawn: whoever has pixels (the
//! screen, the page) follows the numbers.

use crate::drawing::{Effects, Outline, Picture, PictureAdjust, Shadow, ShapePreset, NEUTRAL_TEMPERATURE};
use crate::locale::Locale;
use crate::style::Color;
use crate::validation::localized;
use std::sync::Arc;

// ---- reflection ------------------------------------------------------------

/// The picture seen again under itself, upside down and fading out
/// (`a:reflection`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reflection {
    /// How much light the reflection lets through where it starts, 0
    /// (as strong as the picture) to 1.
    pub transparency: f32,
    /// How much of the picture is seen, from its bottom edge up: 0 to 1.
    pub size: f32,
    /// The gap between the picture and its reflection, points.
    pub distance: f32,
    /// Radius of the blur, points.
    pub blur: f32,
}

impl Reflection {
    /// The nine presets of the Reflection gallery, in its order: tight,
    /// half and full, each touching, 4 points away and 8 points away.
    pub fn presets() -> [Reflection; 9] {
        let made = |size: f32, distance: f32| Reflection {
            transparency: 0.5,
            size,
            distance,
            blur: 0.5,
        };
        [
            made(0.35, 0.0),
            made(0.55, 0.0),
            made(0.9, 0.0),
            made(0.35, 4.0),
            made(0.55, 4.0),
            made(0.9, 4.0),
            made(0.35, 8.0),
            made(0.55, 8.0),
            made(0.9, 8.0),
        ]
    }

    /// The name of a preset of `presets`.
    pub fn preset_name(index: usize, locale: &Locale) -> &'static str {
        const NAMES: [[&str; 5]; 9] = [
            ["Tight Reflection: Touching", "Réflexion rapprochée : contact", "Reflejo ajustado: tocando", "Enge Spiegelung: Berührung", "Riflessione stretta: a contatto"],
            ["Half Reflection: Touching", "Réflexion moyenne : contact", "Semirreflejo: tocando", "Halbe Spiegelung: Berührung", "Riflessione a metà: a contatto"],
            ["Full Reflection: Touching", "Réflexion totale : contact", "Reflejo total: tocando", "Volle Spiegelung: Berührung", "Riflessione completa: a contatto"],
            ["Tight Reflection: 4 point offset", "Réflexion rapprochée : décalage de 4 points", "Reflejo ajustado: desplazamiento de 4 puntos", "Enge Spiegelung: 4 Punkt Offset", "Riflessione stretta: offset 4 punti"],
            ["Half Reflection: 4 point offset", "Réflexion moyenne : décalage de 4 points", "Semirreflejo: desplazamiento de 4 puntos", "Halbe Spiegelung: 4 Punkt Offset", "Riflessione a metà: offset 4 punti"],
            ["Full Reflection: 4 point offset", "Réflexion totale : décalage de 4 points", "Reflejo total: desplazamiento de 4 puntos", "Volle Spiegelung: 4 Punkt Offset", "Riflessione completa: offset 4 punti"],
            ["Tight Reflection: 8 point offset", "Réflexion rapprochée : décalage de 8 points", "Reflejo ajustado: desplazamiento de 8 puntos", "Enge Spiegelung: 8 Punkt Offset", "Riflessione stretta: offset 8 punti"],
            ["Half Reflection: 8 point offset", "Réflexion moyenne : décalage de 8 points", "Semirreflejo: desplazamiento de 8 puntos", "Halbe Spiegelung: 8 Punkt Offset", "Riflessione a metà: offset 8 punti"],
            ["Full Reflection: 8 point offset", "Réflexion totale : décalage de 8 points", "Reflejo total: desplazamiento de 8 puntos", "Volle Spiegelung: 8 Punkt Offset", "Riflessione completa: offset 8 punti"],
        ];
        localized(locale, NAMES[index.min(8)])
    }
}

// ---- bevel -----------------------------------------------------------------

/// The profiles of the Bevel gallery (`prst` of `a:bevelT`), in its
/// order: how the edge of the picture rises from its rim to its face.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BevelPreset {
    Circle,
    RelaxedInset,
    Cross,
    CoolSlant,
    Angle,
    SoftRound,
    Convex,
    Slope,
    Divot,
    Riblet,
    HardEdge,
    ArtDeco,
}

impl BevelPreset {
    pub const ALL: [BevelPreset; 12] = [
        BevelPreset::Circle,
        BevelPreset::RelaxedInset,
        BevelPreset::Cross,
        BevelPreset::CoolSlant,
        BevelPreset::Angle,
        BevelPreset::SoftRound,
        BevelPreset::Convex,
        BevelPreset::Slope,
        BevelPreset::Divot,
        BevelPreset::Riblet,
        BevelPreset::HardEdge,
        BevelPreset::ArtDeco,
    ];

    pub fn ooxml_name(self) -> &'static str {
        match self {
            BevelPreset::Circle => "circle",
            BevelPreset::RelaxedInset => "relaxedInset",
            BevelPreset::Cross => "cross",
            BevelPreset::CoolSlant => "coolSlant",
            BevelPreset::Angle => "angle",
            BevelPreset::SoftRound => "softRound",
            BevelPreset::Convex => "convex",
            BevelPreset::Slope => "slope",
            BevelPreset::Divot => "divot",
            BevelPreset::Riblet => "riblet",
            BevelPreset::HardEdge => "hardEdge",
            BevelPreset::ArtDeco => "artDeco",
        }
    }

    pub fn from_ooxml_name(name: &str) -> Option<BevelPreset> {
        BevelPreset::ALL.into_iter().find(|preset| preset.ooxml_name() == name)
    }

    pub fn name(self, locale: &Locale) -> &'static str {
        let names = match self {
            BevelPreset::Circle => ["Round", "Rond", "Redondo", "Rund", "Arrotondato"],
            BevelPreset::RelaxedInset => ["Relaxed Inset", "Incrustation légère", "Bajorrelieve relajado", "Lockere Vertiefung", "Rilievo rilassato"],
            BevelPreset::Cross => ["Cross", "Croix", "Cruz", "Kreuz", "Croce"],
            BevelPreset::CoolSlant => ["Cool Slant", "Inclinaison froide", "Inclinación fría", "Kühle Schräge", "Inclinazione leggera"],
            BevelPreset::Angle => ["Angle", "Angle", "Ángulo", "Winkel", "Angolo"],
            BevelPreset::SoftRound => ["Soft Round", "Arrondi léger", "Redondo suave", "Weich abgerundet", "Arrotondato attenuato"],
            BevelPreset::Convex => ["Convex", "Convexe", "Convexo", "Konvex", "Convesso"],
            BevelPreset::Slope => ["Slant", "Pente", "Inclinación", "Abschrägung", "Inclinato"],
            BevelPreset::Divot => ["Divot", "Dépression", "Hendidura", "Kerbe", "Solco"],
            BevelPreset::Riblet => ["Riblet", "Nervure", "Ribete", "Rille", "Scanalato"],
            BevelPreset::HardEdge => ["Hard Edge", "Bord vif", "Borde fuerte", "Harte Kante", "Bordo netto"],
            BevelPreset::ArtDeco => ["Art Deco", "Art déco", "Art Decó", "Art Déco", "Art déco"],
        };
        localized(locale, names)
    }

    /// The width and the height Excel gives the bevel of the gallery,
    /// points.
    pub fn default_size(self) -> (f32, f32) {
        match self {
            BevelPreset::Circle | BevelPreset::RelaxedInset | BevelPreset::Angle | BevelPreset::Convex | BevelPreset::Riblet => (6.0, 6.0),
            BevelPreset::Cross => (6.0, 6.0),
            BevelPreset::CoolSlant => (13.0, 6.0),
            BevelPreset::SoftRound => (12.0, 6.0),
            BevelPreset::Slope => (5.0, 13.0),
            BevelPreset::Divot => (11.0, 6.0),
            BevelPreset::HardEdge => (11.0, 6.0),
            BevelPreset::ArtDeco => (11.0, 6.0),
        }
    }

    /// How high the edge stands at `t` of its width (0 at the rim, 1
    /// where the flat face begins), as a fraction of the bevel's height.
    pub fn profile(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        // A quarter of a circle from 0 up to 1.
        let round = |t: f32| (1.0 - (1.0 - t) * (1.0 - t)).max(0.0).sqrt();
        match self {
            BevelPreset::Circle => round(t),
            BevelPreset::Angle => t,
            BevelPreset::SoftRound => (t * std::f32::consts::FRAC_PI_2).sin(),
            // A gentle slope after a small step at the rim.
            BevelPreset::RelaxedInset => 0.3 * round((t * 4.0).min(1.0)) + 0.7 * t,
            // A step half way up, a ledge, then the rest.
            BevelPreset::Cross => {
                if t < 0.3 {
                    0.5 * t / 0.3
                } else if t < 0.7 {
                    0.5
                } else {
                    0.5 + 0.5 * (t - 0.7) / 0.3
                }
            }
            // Steep at the rim, then a long flat slant.
            BevelPreset::CoolSlant => {
                if t < 0.2 {
                    0.6 * t / 0.2
                } else {
                    0.6 + 0.4 * (t - 0.2) / 0.8
                }
            }
            // Rises above the face and comes back down to it.
            BevelPreset::Convex => t + 0.25 * (t * std::f32::consts::PI).sin(),
            // One steep straight side.
            BevelPreset::Slope => (t * 1.6).min(1.0),
            // A groove inside the rim.
            BevelPreset::Divot => {
                if t < 0.3 {
                    t / 0.3
                } else if t < 0.6 {
                    1.0 - 0.6 * ((t - 0.3) / 0.3 * std::f32::consts::PI).sin()
                } else {
                    1.0
                }
            }
            // A rounded rib standing on the rim.
            BevelPreset::Riblet => {
                if t < 0.5 {
                    0.4 * (t / 0.5 * std::f32::consts::PI).sin()
                } else {
                    (t - 0.5) / 0.5
                }
            }
            // A straight side that ends sharply at the face.
            BevelPreset::HardEdge => (t * 1.15).min(1.0),
            // Three steps.
            BevelPreset::ArtDeco => {
                let step = (t * 3.0).floor().min(2.0);
                let within = t * 3.0 - step;
                (step + (within * 2.5).min(1.0)) / 3.0
            }
        }
    }
}

/// The edges of a picture cut at a slant and lit (`a:bevelT` of
/// `a:sp3d`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bevel {
    pub preset: BevelPreset,
    /// How far in from the rim the edge reaches, points.
    pub width: f32,
    /// How high the face stands above the rim, points.
    pub height: f32,
}

impl Bevel {
    /// The bevel the gallery gives for a preset.
    pub fn preset(preset: BevelPreset) -> Bevel {
        let (width, height) = preset.default_size();
        Bevel { preset, width, height }
    }
}

// ---- rotation in space -----------------------------------------------------

/// A picture turned in space and seen by a camera (`a:camera` of
/// `a:scene3d` with its `a:rot`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rotation3d {
    /// Turn about the vertical axis, degrees (`lon`): "X Rotation".
    pub x: f32,
    /// Turn about the horizontal axis, degrees (`lat`): "Y Rotation".
    pub y: f32,
    /// Turn in the plane of the screen, degrees (`rev`): "Z Rotation".
    pub z: f32,
    /// The angle of view of the camera, degrees; 0 for a parallel
    /// projection (far things as large as near ones).
    pub perspective: f32,
}

impl Rotation3d {
    /// True when the picture is seen as it lies.
    pub fn is_flat(&self) -> bool {
        self.x.rem_euclid(360.0) == 0.0 && self.y.rem_euclid(360.0) == 0.0 && self.z.rem_euclid(360.0) == 0.0
    }

    /// Where the corner (`u`, `v`) of a frame `width` by `height`
    /// (fractions, (0, 0) its top-left corner) is seen: x and y from the
    /// middle of the frame, in the units of `width`, and the depth it
    /// was divided by (larger: nearer).
    pub fn project(&self, width: f32, height: f32, u: f32, v: f32) -> (f32, f32, f32) {
        let (x, y) = ((u - 0.5) * width, (v - 0.5) * height);
        // Office turns the camera, not the object: seen from the front,
        // the object turns the other way round each axis.
        let (lon, lat, rev) = ((-self.x).to_radians(), (-self.y).to_radians(), (-self.z).to_radians());
        // About the vertical axis.
        let (sin, cos) = lon.sin_cos();
        let (x, z) = (x * cos, -x * sin);
        // About the horizontal axis.
        let (sin, cos) = lat.sin_cos();
        let (y, z) = (y * cos - z * sin, y * sin + z * cos);
        // In the plane of the screen.
        let (sin, cos) = rev.sin_cos();
        let (x, y) = (x * cos - y * sin, x * sin + y * cos);
        if self.perspective <= 0.0 {
            return (x, y, 1.0);
        }
        // The camera stands where the frame, lying flat, fills its
        // angle of view; what comes nearer grows.
        let half = (self.perspective.clamp(1.0, 120.0) / 2.0).to_radians();
        let distance = width.max(height) / 2.0 / half.tan() + width.max(height) / 2.0;
        let w = distance / (distance - z).max(distance * 0.05);
        (x * w, y * w, w)
    }

    /// The presets of the 3-D Rotation gallery: its three headings, each
    /// with its named cameras.
    pub fn presets() -> Vec<(Camera, Rotation3d)> {
        Camera::ALL.into_iter().map(|camera| (camera, camera.rotation())).collect()
    }
}

/// The cameras of the 3-D Rotation gallery that are a turn of the
/// picture seen in parallel or in perspective. (The oblique ones shear
/// the picture instead; they are left out.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Camera {
    IsometricLeftDown,
    IsometricRightUp,
    IsometricTopUp,
    IsometricBottomDown,
    OffAxis1Left,
    OffAxis1Right,
    OffAxis1Top,
    OffAxis2Left,
    OffAxis2Right,
    OffAxis2Top,
    PerspectiveFront,
    PerspectiveLeft,
    PerspectiveRight,
    PerspectiveBelow,
    PerspectiveAbove,
    PerspectiveRelaxedModerately,
    PerspectiveRelaxed,
    PerspectiveContrastingLeft,
    PerspectiveContrastingRight,
    PerspectiveHeroicExtremeLeft,
    PerspectiveHeroicExtremeRight,
}

impl Camera {
    pub const ALL: [Camera; 21] = [
        Camera::IsometricLeftDown,
        Camera::IsometricRightUp,
        Camera::IsometricTopUp,
        Camera::IsometricBottomDown,
        Camera::OffAxis1Left,
        Camera::OffAxis1Right,
        Camera::OffAxis1Top,
        Camera::OffAxis2Left,
        Camera::OffAxis2Right,
        Camera::OffAxis2Top,
        Camera::PerspectiveFront,
        Camera::PerspectiveLeft,
        Camera::PerspectiveRight,
        Camera::PerspectiveBelow,
        Camera::PerspectiveAbove,
        Camera::PerspectiveRelaxedModerately,
        Camera::PerspectiveRelaxed,
        Camera::PerspectiveContrastingLeft,
        Camera::PerspectiveContrastingRight,
        Camera::PerspectiveHeroicExtremeLeft,
        Camera::PerspectiveHeroicExtremeRight,
    ];

    /// True for the cameras under the "Parallel" heading.
    pub fn parallel(self) -> bool {
        self.rotation().perspective == 0.0
    }

    /// The `prst` of `a:camera`.
    pub fn ooxml_name(self) -> &'static str {
        match self {
            Camera::IsometricLeftDown => "isometricLeftDown",
            Camera::IsometricRightUp => "isometricRightUp",
            Camera::IsometricTopUp => "isometricTopUp",
            Camera::IsometricBottomDown => "isometricBottomDown",
            Camera::OffAxis1Left => "isometricOffAxis1Left",
            Camera::OffAxis1Right => "isometricOffAxis1Right",
            Camera::OffAxis1Top => "isometricOffAxis1Top",
            Camera::OffAxis2Left => "isometricOffAxis2Left",
            Camera::OffAxis2Right => "isometricOffAxis2Right",
            Camera::OffAxis2Top => "isometricOffAxis2Top",
            Camera::PerspectiveFront => "perspectiveFront",
            Camera::PerspectiveLeft => "perspectiveLeft",
            Camera::PerspectiveRight => "perspectiveRight",
            Camera::PerspectiveBelow => "perspectiveBelow",
            Camera::PerspectiveAbove => "perspectiveAbove",
            Camera::PerspectiveRelaxedModerately => "perspectiveRelaxedModerately",
            Camera::PerspectiveRelaxed => "perspectiveRelaxed",
            Camera::PerspectiveContrastingLeft => "perspectiveContrastingLeftFacing",
            Camera::PerspectiveContrastingRight => "perspectiveContrastingRightFacing",
            Camera::PerspectiveHeroicExtremeLeft => "perspectiveHeroicExtremeLeftFacing",
            Camera::PerspectiveHeroicExtremeRight => "perspectiveHeroicExtremeRightFacing",
        }
    }

    pub fn from_ooxml_name(name: &str) -> Option<Camera> {
        Camera::ALL.into_iter().find(|camera| camera.ooxml_name() == name)
    }

    /// The turn and the angle of view Office gives the camera (the
    /// angles of its preset table, to a tenth of a degree).
    pub fn rotation(self) -> Rotation3d {
        let (y, x, z, perspective) = match self {
            Camera::IsometricLeftDown => (35.0, 45.0, 0.0, 0.0),
            Camera::IsometricRightUp => (35.0, 315.0, 0.0, 0.0),
            Camera::IsometricTopUp => (324.7, 314.7, 60.2, 0.0),
            Camera::IsometricBottomDown => (35.3, 314.7, 299.8, 0.0),
            Camera::OffAxis1Left => (18.0, 64.0, 0.0, 0.0),
            Camera::OffAxis1Right => (18.0, 334.0, 0.0, 0.0),
            Camera::OffAxis1Top => (306.5, 301.3, 57.6, 0.0),
            Camera::OffAxis2Left => (18.0, 26.0, 0.0, 0.0),
            Camera::OffAxis2Right => (18.0, 296.0, 0.0, 0.0),
            Camera::OffAxis2Top => (306.5, 58.7, 302.4, 0.0),
            Camera::PerspectiveFront => (0.0, 0.0, 0.0, 45.0),
            Camera::PerspectiveLeft => (0.0, 20.0, 0.0, 45.0),
            Camera::PerspectiveRight => (0.0, 340.0, 0.0, 45.0),
            Camera::PerspectiveBelow => (20.0, 0.0, 0.0, 45.0),
            Camera::PerspectiveAbove => (340.0, 0.0, 0.0, 45.0),
            Camera::PerspectiveRelaxedModerately => (324.8, 0.0, 0.0, 45.0),
            Camera::PerspectiveRelaxed => (309.6, 0.0, 0.0, 45.0),
            Camera::PerspectiveContrastingLeft => (10.4, 43.9, 10.4, 45.0),
            Camera::PerspectiveContrastingRight => (10.4, 316.1, 349.6, 45.0),
            Camera::PerspectiveHeroicExtremeLeft => (8.1, 34.5, 357.1, 80.0),
            Camera::PerspectiveHeroicExtremeRight => (8.1, 325.5, 2.9, 80.0),
        };
        Rotation3d { x, y, z, perspective }
    }

    pub fn name(self, locale: &Locale) -> &'static str {
        let names = match self {
            Camera::IsometricLeftDown => ["Isometric: Left Down", "Isométrique : gauche vers le bas", "Isométrico: izquierda abajo", "Isometrisch: links unten", "Isometrica: sinistra in basso"],
            Camera::IsometricRightUp => ["Isometric: Right Up", "Isométrique : droite vers le haut", "Isométrico: derecha arriba", "Isometrisch: rechts oben", "Isometrica: destra in alto"],
            Camera::IsometricTopUp => ["Isometric: Top Up", "Isométrique : haut vers le haut", "Isométrico: superior arriba", "Isometrisch: oben aufwärts", "Isometrica: alto verso l'alto"],
            Camera::IsometricBottomDown => ["Isometric: Bottom Down", "Isométrique : bas vers le bas", "Isométrico: inferior abajo", "Isometrisch: unten abwärts", "Isometrica: basso verso il basso"],
            Camera::OffAxis1Left => ["Isometric: Off Axis 1 Left", "Isométrique : hors axe 1, gauche", "Isométrico: fuera del eje 1, izquierda", "Isometrisch: achsenfern 1, links", "Isometrica: fuori asse 1 a sinistra"],
            Camera::OffAxis1Right => ["Isometric: Off Axis 1 Right", "Isométrique : hors axe 1, droite", "Isométrico: fuera del eje 1, derecha", "Isometrisch: achsenfern 1, rechts", "Isometrica: fuori asse 1 a destra"],
            Camera::OffAxis1Top => ["Isometric: Off Axis 1 Top", "Isométrique : hors axe 1, haut", "Isométrico: fuera del eje 1, superior", "Isometrisch: achsenfern 1, oben", "Isometrica: fuori asse 1 in alto"],
            Camera::OffAxis2Left => ["Isometric: Off Axis 2 Left", "Isométrique : hors axe 2, gauche", "Isométrico: fuera del eje 2, izquierda", "Isometrisch: achsenfern 2, links", "Isometrica: fuori asse 2 a sinistra"],
            Camera::OffAxis2Right => ["Isometric: Off Axis 2 Right", "Isométrique : hors axe 2, droite", "Isométrico: fuera del eje 2, derecha", "Isometrisch: achsenfern 2, rechts", "Isometrica: fuori asse 2 a destra"],
            Camera::OffAxis2Top => ["Isometric: Off Axis 2 Top", "Isométrique : hors axe 2, haut", "Isométrico: fuera del eje 2, superior", "Isometrisch: achsenfern 2, oben", "Isometrica: fuori asse 2 in alto"],
            Camera::PerspectiveFront => ["Perspective: Front", "Perspective : avant", "Perspectiva: frontal", "Perspektive: vorne", "Prospettiva: frontale"],
            Camera::PerspectiveLeft => ["Perspective: Left", "Perspective : gauche", "Perspectiva: izquierda", "Perspektive: links", "Prospettiva: sinistra"],
            Camera::PerspectiveRight => ["Perspective: Right", "Perspective : droite", "Perspectiva: derecha", "Perspektive: rechts", "Prospettiva: destra"],
            Camera::PerspectiveBelow => ["Perspective: Below", "Perspective : dessous", "Perspectiva: abajo", "Perspektive: unten", "Prospettiva: dal basso"],
            Camera::PerspectiveAbove => ["Perspective: Above", "Perspective : dessus", "Perspectiva: arriba", "Perspektive: oben", "Prospettiva: dall'alto"],
            Camera::PerspectiveRelaxedModerately => ["Perspective: Relaxed Moderately", "Perspective : modérément relâchée", "Perspectiva: relajada moderadamente", "Perspektive: mäßig entspannt", "Prospettiva: moderatamente rilassata"],
            Camera::PerspectiveRelaxed => ["Perspective: Relaxed", "Perspective : relâchée", "Perspectiva: relajada", "Perspektive: entspannt", "Prospettiva: rilassata"],
            Camera::PerspectiveContrastingLeft => ["Perspective: Contrasting Left", "Perspective : contrastée à gauche", "Perspectiva: en contraste a la izquierda", "Perspektive: Kontrast links", "Prospettiva: contrasto a sinistra"],
            Camera::PerspectiveContrastingRight => ["Perspective: Contrasting Right", "Perspective : contrastée à droite", "Perspectiva: en contraste a la derecha", "Perspektive: Kontrast rechts", "Prospettiva: contrasto a destra"],
            Camera::PerspectiveHeroicExtremeLeft => ["Perspective: Heroic Extreme Left", "Perspective : héroïque extrême à gauche", "Perspectiva: heroica extrema a la izquierda", "Perspektive: heroisch extrem links", "Prospettiva: eroica estrema a sinistra"],
            Camera::PerspectiveHeroicExtremeRight => ["Perspective: Heroic Extreme Right", "Perspective : héroïque extrême à droite", "Perspectiva: heroica extrema a la derecha", "Perspektive: heroisch extrem rechts", "Prospettiva: eroica estrema a destra"],
        };
        localized(locale, names)
    }
}

// ---- picture styles ----------------------------------------------------------

/// The styles of the Picture Styles gallery that can be drawn, in the
/// order of Excel's gallery. (Left out: "Double Frame, Black" and
/// "Compound Frame, Black", whose outline is two lines of different
/// widths with a shadow inside.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PictureStyle {
    SimpleFrameWhite,
    BeveledMatteWhite,
    MetalFrame,
    DropShadowRectangle,
    ReflectedRoundedRectangle,
    SoftEdgeRectangle,
    ThickMatteBlack,
    SimpleFrameBlack,
    BeveledOvalBlack,
    ModerateFrameBlack,
    CenterShadowRectangle,
    RoundedDiagonalCornerWhite,
    SnipDiagonalCornerWhite,
    ModerateFrameWhite,
    RotatedWhite,
    PerspectiveShadowWhite,
    RelaxedPerspectiveWhite,
    SoftEdgeOval,
    BevelRectangle,
    BevelPerspective,
    ReflectedPerspectiveRight,
    BevelPerspectiveLeftWhite,
    ReflectedBevelBlack,
    ReflectedBevelWhite,
    MetalRoundedRectangle,
    MetalOval,
}

/// What a style gives a picture: the outline it is cut to, its border
/// and its effects.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StyleLook {
    pub shape: ShapePreset,
    pub outline: Option<Outline>,
    pub effects: Effects,
}

impl PictureStyle {
    pub const ALL: [PictureStyle; 26] = [
        PictureStyle::SimpleFrameWhite,
        PictureStyle::BeveledMatteWhite,
        PictureStyle::MetalFrame,
        PictureStyle::DropShadowRectangle,
        PictureStyle::ReflectedRoundedRectangle,
        PictureStyle::SoftEdgeRectangle,
        PictureStyle::ThickMatteBlack,
        PictureStyle::SimpleFrameBlack,
        PictureStyle::BeveledOvalBlack,
        PictureStyle::ModerateFrameBlack,
        PictureStyle::CenterShadowRectangle,
        PictureStyle::RoundedDiagonalCornerWhite,
        PictureStyle::SnipDiagonalCornerWhite,
        PictureStyle::ModerateFrameWhite,
        PictureStyle::RotatedWhite,
        PictureStyle::PerspectiveShadowWhite,
        PictureStyle::RelaxedPerspectiveWhite,
        PictureStyle::SoftEdgeOval,
        PictureStyle::BevelRectangle,
        PictureStyle::BevelPerspective,
        PictureStyle::ReflectedPerspectiveRight,
        PictureStyle::BevelPerspectiveLeftWhite,
        PictureStyle::ReflectedBevelBlack,
        PictureStyle::ReflectedBevelWhite,
        PictureStyle::MetalRoundedRectangle,
        PictureStyle::MetalOval,
    ];

    pub fn name(self, locale: &Locale) -> &'static str {
        let names = match self {
            PictureStyle::SimpleFrameWhite => ["Simple Frame, White", "Cadre simple, blanc", "Marco sencillo, blanco", "Einfacher Rahmen, weiß", "Cornice semplice, bianco"],
            PictureStyle::BeveledMatteWhite => ["Beveled Matte, White", "Mat biseauté, blanc", "Mate biselado, blanco", "Abgeschrägt, matt, weiß", "Opaco smussato, bianco"],
            PictureStyle::MetalFrame => ["Metal Frame", "Cadre métallique", "Marco metálico", "Metallrahmen", "Cornice metallica"],
            PictureStyle::DropShadowRectangle => ["Drop Shadow Rectangle", "Rectangle avec ombre portée", "Rectángulo sombreado", "Rechteck mit Schlagschatten", "Rettangolo con ombreggiatura"],
            PictureStyle::ReflectedRoundedRectangle => ["Reflected Rounded Rectangle", "Rectangle à coins arrondis avec réflexion", "Rectángulo redondeado reflejado", "Abgerundetes Rechteck, gespiegelt", "Rettangolo arrotondato riflesso"],
            PictureStyle::SoftEdgeRectangle => ["Soft Edge Rectangle", "Rectangle à bords adoucis", "Rectángulo de bordes suaves", "Rechteck mit weichen Kanten", "Rettangolo con contorno sfumato"],
            PictureStyle::ThickMatteBlack => ["Thick Matte, Black", "Mat épais, noir", "Mate grueso, negro", "Dick, matt, schwarz", "Opaco spesso, nero"],
            PictureStyle::SimpleFrameBlack => ["Simple Frame, Black", "Cadre simple, noir", "Marco sencillo, negro", "Einfacher Rahmen, schwarz", "Cornice semplice, nero"],
            PictureStyle::BeveledOvalBlack => ["Beveled Oval, Black", "Ovale biseauté, noir", "Óvalo biselado, negro", "Abgeschrägtes Oval, schwarz", "Ovale smussato, nero"],
            PictureStyle::ModerateFrameBlack => ["Moderate Frame, Black", "Cadre modéré, noir", "Marco moderado, negro", "Mittlerer Rahmen, schwarz", "Cornice moderata, nero"],
            PictureStyle::CenterShadowRectangle => ["Center Shadow Rectangle", "Rectangle avec ombre au centre", "Rectángulo de sombra en el centro", "Rechteck mit zentriertem Schatten", "Rettangolo con ombreggiatura al centro"],
            PictureStyle::RoundedDiagonalCornerWhite => ["Rounded Diagonal Corner, White", "Coin diagonal arrondi, blanc", "Esquina diagonal redondeada, blanco", "Abgerundete diagonale Ecke, weiß", "Angolo diagonale arrotondato, bianco"],
            PictureStyle::SnipDiagonalCornerWhite => ["Snip Diagonal Corner, White", "Coin diagonal rogné, blanc", "Esquina diagonal recortada, blanco", "Abgeschnittene diagonale Ecke, weiß", "Angolo diagonale ritagliato, bianco"],
            PictureStyle::ModerateFrameWhite => ["Moderate Frame, White", "Cadre modéré, blanc", "Marco moderado, blanco", "Mittlerer Rahmen, weiß", "Cornice moderata, bianco"],
            PictureStyle::RotatedWhite => ["Rotated, White", "Pivoté, blanc", "Girado, blanco", "Gedreht, weiß", "Ruotato, bianco"],
            PictureStyle::PerspectiveShadowWhite => ["Perspective Shadow, White", "Ombre en perspective, blanc", "Sombra en perspectiva, blanco", "Perspektivschatten, weiß", "Ombreggiatura in prospettiva, bianco"],
            PictureStyle::RelaxedPerspectiveWhite => ["Relaxed Perspective, White", "Perspective relâchée, blanc", "Perspectiva relajada, blanco", "Entspannte Perspektive, weiß", "Prospettiva rilassata, bianco"],
            PictureStyle::SoftEdgeOval => ["Soft Edge Oval", "Ovale à bords adoucis", "Óvalo de bordes suaves", "Oval mit weichen Kanten", "Ovale con contorno sfumato"],
            PictureStyle::BevelRectangle => ["Bevel Rectangle", "Rectangle biseauté", "Rectángulo biselado", "Abgeschrägtes Rechteck", "Rettangolo smussato"],
            PictureStyle::BevelPerspective => ["Bevel Perspective", "Perspective biseautée", "Perspectiva biselada", "Abgeschrägte Perspektive", "Prospettiva smussata"],
            PictureStyle::ReflectedPerspectiveRight => ["Reflected Perspective Right", "Perspective à droite avec réflexion", "Perspectiva reflejada a la derecha", "Gespiegelte Perspektive rechts", "Prospettiva riflessa a destra"],
            PictureStyle::BevelPerspectiveLeftWhite => ["Bevel Perspective Left, White", "Perspective biseautée à gauche, blanc", "Perspectiva biselada a la izquierda, blanco", "Abgeschrägte Perspektive links, weiß", "Prospettiva smussata a sinistra, bianco"],
            PictureStyle::ReflectedBevelBlack => ["Reflected Bevel, Black", "Biseau avec réflexion, noir", "Bisel reflejado, negro", "Gespiegelte Abschrägung, schwarz", "Smusso riflesso, nero"],
            PictureStyle::ReflectedBevelWhite => ["Reflected Bevel, White", "Biseau avec réflexion, blanc", "Bisel reflejado, blanco", "Gespiegelte Abschrägung, weiß", "Smusso riflesso, bianco"],
            PictureStyle::MetalRoundedRectangle => ["Metal Rounded Rectangle", "Rectangle à coins arrondis métallique", "Rectángulo redondeado metálico", "Abgerundetes Rechteck, Metall", "Rettangolo arrotondato metallico"],
            PictureStyle::MetalOval => ["Metal Oval", "Ovale métallique", "Óvalo metálico", "Oval, Metall", "Ovale metallico"],
        };
        localized(locale, names)
    }

    /// The look of the style: the numbers of Office's own definitions,
    /// in what the engine can say.
    pub fn look(self) -> StyleLook {
        const WHITE: Color = Color(0xFFFFFF);
        const BLACK: Color = Color(0x000000);
        const METAL: Color = Color(0xC8C6BD);
        let frame = |color: Color, width: f32| Some(Outline::solid(color, width));
        let shadow = |blur: f32, distance: f32, angle: f32, transparency: f32| {
            Some(Shadow {
                color: BLACK,
                transparency,
                size: 1.0,
                blur,
                angle,
                distance,
                ..Shadow::default()
            })
        };
        let bevel = |preset: BevelPreset, width: f32, height: f32| Some(Bevel { preset, width, height });
        let reflection = |transparency: f32| {
            Some(Reflection {
                transparency,
                size: 0.28,
                distance: 0.4,
                blur: 1.0,
            })
        };
        let camera = |camera: Camera| Some(camera.rotation());
        let mut look = StyleLook {
            shape: ShapePreset::Rect,
            outline: None,
            effects: Effects::default(),
        };
        let effects = &mut look.effects;
        match self {
            PictureStyle::SimpleFrameWhite => {
                look.outline = frame(WHITE, 7.0);
                effects.shadow = shadow(4.3, 1.4, 90.0, 0.6);
            }
            PictureStyle::BeveledMatteWhite => {
                look.outline = frame(Color(0xEAEAEA), 7.0);
                effects.shadow = shadow(4.3, 1.4, 90.0, 0.6);
                effects.bevel = bevel(BevelPreset::Circle, 2.0, 1.5);
            }
            PictureStyle::MetalFrame => {
                look.outline = frame(METAL, 15.0);
                effects.shadow = shadow(3.9, 4.0, 45.0, 0.59);
                effects.bevel = bevel(BevelPreset::HardEdge, 2.0, 2.0);
            }
            PictureStyle::DropShadowRectangle => effects.shadow = Some(Shadow {
                color: Color(0x333333),
                ..shadow(23.0, 11.0, 45.0, 0.35).unwrap_or_default()
            }),
            PictureStyle::ReflectedRoundedRectangle => {
                look.shape = ShapePreset::RoundRect;
                effects.reflection = reflection(0.62);
            }
            PictureStyle::SoftEdgeRectangle => effects.soft_edge = 8.86,
            PictureStyle::ThickMatteBlack => {
                look.outline = frame(BLACK, 15.0);
                effects.shadow = shadow(4.5, 4.0, 45.0, 0.6);
            }
            PictureStyle::SimpleFrameBlack => {
                look.shape = ShapePreset::RoundRect;
                look.outline = frame(BLACK, 3.0);
            }
            PictureStyle::BeveledOvalBlack => {
                look.shape = ShapePreset::Ellipse;
                look.outline = frame(Color(0x333333), 5.0);
                effects.shadow = Some(Shadow {
                    size: 0.8,
                    squash: -0.225,
                    ..shadow(30.0, 23.0, 90.0, 0.78).unwrap_or_default()
                });
                effects.bevel = bevel(BevelPreset::Circle, 7.5, 2.5);
            }
            PictureStyle::ModerateFrameBlack => {
                look.outline = frame(BLACK, 10.0);
                effects.shadow = shadow(4.5, 4.0, 45.0, 0.6);
            }
            PictureStyle::CenterShadowRectangle => effects.shadow = shadow(15.0, 0.0, 0.0, 0.3),
            PictureStyle::RoundedDiagonalCornerWhite => {
                look.shape = ShapePreset::Round2DiagRect;
                look.outline = frame(WHITE, 7.0);
                effects.shadow = shadow(20.0, 0.0, 0.0, 0.57);
            }
            PictureStyle::SnipDiagonalCornerWhite => {
                look.shape = ShapePreset::Snip2DiagRect;
                look.outline = frame(WHITE, 7.0);
                effects.shadow = shadow(7.0, 0.0, 0.0, 0.55);
            }
            PictureStyle::ModerateFrameWhite => {
                look.outline = frame(WHITE, 15.0);
                effects.shadow = shadow(5.1, 3.0, 45.0, 0.6);
            }
            PictureStyle::RotatedWhite => {
                look.outline = frame(WHITE, 15.0);
                effects.shadow = shadow(5.1, 4.0, 215.0, 0.7);
                effects.rotation = Some(Rotation3d { x: 0.0, y: 0.0, z: 6.0, perspective: 0.0 });
            }
            PictureStyle::PerspectiveShadowWhite => {
                look.outline = frame(WHITE, 7.0);
                effects.shadow = Some(Shadow {
                    size: 0.97,
                    squash: 0.237,
                    skew: 15.0,
                    ..shadow(6.0, 7.5, 175.0, 0.8).unwrap_or_default()
                });
            }
            PictureStyle::RelaxedPerspectiveWhite => {
                look.outline = frame(WHITE, 10.0);
                effects.shadow = shadow(12.0, 15.0, 90.0, 0.7);
                effects.rotation = Some(Rotation3d { x: 0.0, y: 330.0, z: 0.0, perspective: 45.0 });
            }
            PictureStyle::SoftEdgeOval => {
                look.shape = ShapePreset::Ellipse;
                effects.soft_edge = 8.86;
            }
            PictureStyle::BevelRectangle => {
                look.shape = ShapePreset::RoundRect;
                effects.shadow = shadow(6.0, 3.0, 130.0, 0.56);
                effects.bevel = bevel(BevelPreset::SoftRound, 12.0, 4.0);
            }
            PictureStyle::BevelPerspective => {
                effects.shadow = Some(Shadow {
                    size: 0.9,
                    squash: -0.21,
                    ..shadow(12.0, 25.0, 90.0, 0.7).unwrap_or_default()
                });
                effects.bevel = bevel(BevelPreset::HardEdge, 24.0, 12.0);
                effects.rotation = camera(Camera::PerspectiveRelaxedModerately);
            }
            PictureStyle::ReflectedPerspectiveRight => {
                effects.reflection = reflection(0.67);
                effects.rotation = camera(Camera::PerspectiveContrastingLeft);
            }
            PictureStyle::BevelPerspectiveLeftWhite => {
                look.outline = frame(WHITE, 7.0);
                effects.shadow = shadow(6.0, 4.0, 90.0, 0.6);
                effects.bevel = bevel(BevelPreset::Circle, 3.0, 3.0);
                effects.rotation = camera(Camera::PerspectiveContrastingRight);
            }
            PictureStyle::ReflectedBevelBlack => {
                look.shape = ShapePreset::RoundRect;
                look.outline = frame(Color(0x292929), 6.0);
                effects.reflection = reflection(0.67);
                effects.bevel = bevel(BevelPreset::HardEdge, 3.0, 3.0);
            }
            PictureStyle::ReflectedBevelWhite => {
                look.shape = ShapePreset::RoundRect;
                look.outline = frame(WHITE, 6.0);
                effects.reflection = reflection(0.7);
                effects.bevel = bevel(BevelPreset::HardEdge, 3.0, 3.0);
            }
            PictureStyle::MetalRoundedRectangle => {
                look.shape = ShapePreset::RoundRect;
                look.outline = frame(METAL, 15.0);
                effects.shadow = shadow(8.0, 0.0, 0.0, 0.58);
                effects.bevel = bevel(BevelPreset::Circle, 12.0, 4.0);
            }
            PictureStyle::MetalOval => {
                look.shape = ShapePreset::Ellipse;
                look.outline = frame(METAL, 15.0);
                effects.shadow = shadow(10.0, 0.0, 0.0, 0.65);
                effects.bevel = bevel(BevelPreset::Circle, 12.0, 4.0);
            }
        }
        look
    }

    /// The style a picture wears: the one whose look is its own, to the
    /// precision a file keeps numbers with.
    pub fn of(shape: ShapePreset, outline: Option<Outline>, effects: &Effects) -> Option<PictureStyle> {
        let near = |a: f32, b: f32| (a - b).abs() <= 2e-3 * a.abs().max(b.abs()).max(1.0);
        let both = |a: Option<&[f32]>, b: Option<&[f32]>| match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => a.iter().zip(b).all(|(a, b)| near(*a, *b)),
            _ => false,
        };
        let line = |o: &Outline| [o.color.value() as f32, o.width, o.dash as u8 as f32];
        let shadow = |s: &Shadow| [s.color.value() as f32, s.transparency, s.size, s.blur, s.angle, s.distance, s.squash, s.skew];
        let glow = |g: &crate::drawing::Glow| [g.color.value() as f32, g.transparency, g.radius];
        let reflection = |r: &Reflection| [r.transparency, r.size, r.distance, r.blur];
        let bevel = |b: &Bevel| [b.preset as u8 as f32, b.width, b.height];
        let rotation = |r: &Rotation3d| [r.x, r.y, r.z, r.perspective];
        PictureStyle::ALL.into_iter().find(|style| {
            let look = style.look();
            let (a, b) = (&look.effects, effects);
            look.shape == shape
                && both(look.outline.as_ref().map(line).as_ref().map(|v| &v[..]), outline.as_ref().map(line).as_ref().map(|v| &v[..]))
                && both(a.shadow.as_ref().map(shadow).as_ref().map(|v| &v[..]), b.shadow.as_ref().map(shadow).as_ref().map(|v| &v[..]))
                && both(a.glow.as_ref().map(glow).as_ref().map(|v| &v[..]), b.glow.as_ref().map(glow).as_ref().map(|v| &v[..]))
                && both(a.reflection.as_ref().map(reflection).as_ref().map(|v| &v[..]), b.reflection.as_ref().map(reflection).as_ref().map(|v| &v[..]))
                && both(a.bevel.as_ref().map(bevel).as_ref().map(|v| &v[..]), b.bevel.as_ref().map(bevel).as_ref().map(|v| &v[..]))
                && both(a.rotation.as_ref().map(rotation).as_ref().map(|v| &v[..]), b.rotation.as_ref().map(rotation).as_ref().map(|v| &v[..]))
                && near(a.soft_edge, b.soft_edge)
        })
    }
}

// ---- artistic effects --------------------------------------------------------

/// The artistic effects that are real filters of the image, in the order
/// of Excel's gallery. (Left out, for want of the textures Office paints
/// them with: Paint Strokes, Watercolor Sponge, Cement, Texturizer,
/// Crisscross Etching, Pastels Smooth and Plastic Wrap.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ArtisticKind {
    Marker,
    PencilGrayscale,
    PencilSketch,
    LineDrawing,
    ChalkSketch,
    PaintBrush,
    GlowDiffused,
    Blur,
    LightScreen,
    FilmGrain,
    MosaicBubbles,
    Glass,
    Cutout,
    Photocopy,
    GlowEdges,
}

impl ArtisticKind {
    pub const ALL: [ArtisticKind; 15] = [
        ArtisticKind::Marker,
        ArtisticKind::PencilGrayscale,
        ArtisticKind::PencilSketch,
        ArtisticKind::LineDrawing,
        ArtisticKind::ChalkSketch,
        ArtisticKind::PaintBrush,
        ArtisticKind::GlowDiffused,
        ArtisticKind::Blur,
        ArtisticKind::LightScreen,
        ArtisticKind::FilmGrain,
        ArtisticKind::MosaicBubbles,
        ArtisticKind::Glass,
        ArtisticKind::Cutout,
        ArtisticKind::Photocopy,
        ArtisticKind::GlowEdges,
    ];

    /// The element of `a14:imgEffect` (Office's own spelling, "Mosiaic"
    /// included) and the attribute that holds the effect's one setting.
    pub fn ooxml(self) -> (&'static str, &'static str) {
        match self {
            ArtisticKind::Marker => ("artisticMarker", "size"),
            ArtisticKind::PencilGrayscale => ("artisticPencilGrayscale", "pencilSize"),
            ArtisticKind::PencilSketch => ("artisticPencilSketch", "pressure"),
            ArtisticKind::LineDrawing => ("artisticLineDrawing", "pencilSize"),
            ArtisticKind::ChalkSketch => ("artisticChalkSketch", "pressure"),
            ArtisticKind::PaintBrush => ("artisticPaintBrush", "brushSize"),
            ArtisticKind::GlowDiffused => ("artisticGlowDiffused", "intensity"),
            ArtisticKind::Blur => ("artisticBlur", "radius"),
            ArtisticKind::LightScreen => ("artisticLightScreen", "gridSize"),
            ArtisticKind::FilmGrain => ("artisticFilmGrain", "grainSize"),
            ArtisticKind::MosaicBubbles => ("artisticMosiaicBubbles", "pressure"),
            ArtisticKind::Glass => ("artisticGlass", "scaling"),
            ArtisticKind::Cutout => ("artisticCutout", "numberOfShades"),
            ArtisticKind::Photocopy => ("artisticPhotocopy", "detail"),
            ArtisticKind::GlowEdges => ("artisticGlowEdges", "smoothness"),
        }
    }

    pub fn from_ooxml_name(name: &str) -> Option<ArtisticKind> {
        ArtisticKind::ALL.into_iter().find(|kind| kind.ooxml().0 == name)
    }

    /// The range of the effect's setting and the value the gallery
    /// gives it (Excel's).
    pub fn amount_range(self) -> (f32, f32, f32) {
        match self {
            ArtisticKind::Marker => (0.0, 100.0, 80.0),
            ArtisticKind::PencilGrayscale => (0.0, 100.0, 52.0),
            ArtisticKind::PencilSketch => (0.0, 100.0, 14.0),
            ArtisticKind::LineDrawing => (0.0, 100.0, 6.0),
            ArtisticKind::ChalkSketch => (0.0, 4.0, 4.0),
            ArtisticKind::PaintBrush => (0.0, 10.0, 2.0),
            ArtisticKind::GlowDiffused => (0.0, 10.0, 5.0),
            ArtisticKind::Blur => (0.0, 100.0, 10.0),
            ArtisticKind::LightScreen => (0.0, 10.0, 3.0),
            ArtisticKind::FilmGrain => (0.0, 100.0, 40.0),
            ArtisticKind::MosaicBubbles => (0.0, 100.0, 22.0),
            ArtisticKind::Glass => (0.0, 100.0, 50.0),
            ArtisticKind::Cutout => (0.0, 6.0, 1.0),
            ArtisticKind::Photocopy => (0.0, 10.0, 3.0),
            ArtisticKind::GlowEdges => (0.0, 10.0, 5.0),
        }
    }

    pub fn name(self, locale: &Locale) -> &'static str {
        let names = match self {
            ArtisticKind::Marker => ["Marker", "Marqueur", "Marcador", "Marker", "Pennarello"],
            ArtisticKind::PencilGrayscale => ["Pencil Grayscale", "Crayon - Nuances de gris", "Escala de grises con lápiz", "Bleistift: Graustufen", "Matita in scala di grigi"],
            ArtisticKind::PencilSketch => ["Pencil Sketch", "Croquis au crayon", "Boceto con lápiz", "Bleistiftskizze", "Schizzo a matita"],
            ArtisticKind::LineDrawing => ["Line Drawing", "Dessin au trait", "Dibujo de línea", "Strichzeichnung", "Disegno a linee"],
            ArtisticKind::ChalkSketch => ["Chalk Sketch", "Croquis à la craie", "Boceto con tiza", "Kreideskizze", "Schizzo a gessetto"],
            ArtisticKind::PaintBrush => ["Paint Brush", "Pinceau", "Pincel", "Pinsel", "Pennello"],
            ArtisticKind::GlowDiffused => ["Glow Diffused", "Lueur diffuse", "Iluminado difuso", "Weiches Leuchten", "Bagliore diffuso"],
            ArtisticKind::Blur => ["Blur", "Flou", "Desenfoque", "Weichzeichnen", "Sfocatura"],
            ArtisticKind::LightScreen => ["Light Screen", "Écran clair", "Pantalla de luz", "Heller Raster", "Schermo chiaro"],
            ArtisticKind::FilmGrain => ["Film Grain", "Grain de pellicule", "Película granulada", "Filmkörnung", "Grana pellicola"],
            ArtisticKind::MosaicBubbles => ["Mosaic Bubbles", "Bulles de mosaïque", "Burbujas de mosaico", "Mosaikblasen", "Bolle mosaico"],
            ArtisticKind::Glass => ["Glass", "Verre", "Cristal", "Glas", "Vetro"],
            ArtisticKind::Cutout => ["Cutout", "Découpage", "Cuarteado", "Ausschneiden", "Ritaglio"],
            ArtisticKind::Photocopy => ["Photocopy", "Photocopie", "Fotocopia", "Fotokopie", "Fotocopia"],
            ArtisticKind::GlowEdges => ["Glow Edges", "Bords brillants", "Bordes iluminados", "Leuchtende Kanten", "Contorni luminosi"],
        };
        localized(locale, names)
    }

    /// The name of the effect's setting in the Format Picture pane.
    pub fn amount_name(self, locale: &Locale) -> &'static str {
        let names = match self.ooxml().1 {
            "size" => ["Size", "Taille", "Tamaño", "Größe", "Dimensioni"],
            "pencilSize" => ["Pencil Size", "Taille du crayon", "Tamaño del lápiz", "Stiftgröße", "Dimensioni matita"],
            "pressure" => ["Pressure", "Pression", "Presión", "Druck", "Pressione"],
            "brushSize" => ["Brush Size", "Taille du pinceau", "Tamaño del pincel", "Pinselgröße", "Dimensioni pennello"],
            "intensity" => ["Intensity", "Intensité", "Intensidad", "Intensität", "Intensità"],
            "radius" => ["Radius", "Rayon", "Radio", "Radius", "Raggio"],
            "gridSize" => ["Grid Size", "Taille de la grille", "Tamaño de cuadrícula", "Rastergröße", "Dimensioni griglia"],
            "grainSize" => ["Grain Size", "Taille du grain", "Tamaño del grano", "Körnungsgröße", "Dimensioni grana"],
            "scaling" => ["Scaling", "Mise à l'échelle", "Escala", "Skalierung", "Proporzioni"],
            "numberOfShades" => ["Number of Shades", "Nombre de nuances", "Número de sombras", "Anzahl der Schattierungen", "Numero di sfumature"],
            "detail" => ["Detail", "Détail", "Detalle", "Detail", "Dettaglio"],
            _ => ["Smoothness", "Lissage", "Suavizado", "Glättung", "Uniformità"],
        };
        localized(locale, names)
    }
}

/// An artistic effect on a picture (`a14:imgEffect`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Artistic {
    pub kind: ArtisticKind,
    /// How much of the image as it was shows through the effect, 0 to 1.
    pub transparency: f32,
    /// The effect's one setting, in Excel's units (`amount_range`).
    pub amount: f32,
}

impl Artistic {
    /// The effect as the gallery gives it.
    pub fn preset(kind: ArtisticKind) -> Artistic {
        Artistic {
            kind,
            transparency: 0.0,
            amount: kind.amount_range().2,
        }
    }
}

/// The image of a picture with the effects a file keeps as a second
/// image: the artistic effect, the saturation and the colour temperature
/// (Excel stores the result in the picture's blip, and the image as it
/// was beside it). It is made by whoever has the pixels; it stands for
/// the picture as long as it was made for the settings the picture has.
#[derive(Clone, Debug, PartialEq)]
pub struct Baked {
    /// An image file (PNG).
    pub data: Arc<[u8]>,
    pub saturation: f32,
    pub temperature: f32,
    pub artistic: Option<Artistic>,
}

impl Picture {
    /// True when the picture has effects that are kept as a second image.
    pub fn has_baked_effects(&self) -> bool {
        self.artistic.is_some() || self.adjust.saturation != 1.0 || self.adjust.temperature != NEUTRAL_TEMPERATURE
    }

    /// The second image, when it is the one of the picture as it is set.
    pub fn baked(&self) -> Option<&Baked> {
        self.baked.as_ref().filter(|baked| {
            self.has_baked_effects()
                && baked.artistic == self.artistic
                && baked.saturation == self.adjust.saturation
                && baked.temperature == self.adjust.temperature
        })
    }

    /// True when the second image has to be made (again).
    pub fn needs_baking(&self) -> bool {
        self.has_baked_effects() && self.baked().is_none()
    }

    /// What to draw the picture from: an image file and what is still to
    /// be done to its pixels. With its second image, that one and the
    /// rest of the adjustments; without (it is not made yet), the image
    /// as it was with all the adjustments — the artistic effect then
    /// does not show.
    pub fn shown(&self) -> (&Arc<[u8]>, PictureAdjust) {
        match self.baked() {
            Some(baked) => (
                &baked.data,
                PictureAdjust {
                    saturation: 1.0,
                    temperature: NEUTRAL_TEMPERATURE,
                    ..self.adjust
                },
            ),
            None => (&self.data, self.adjust),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_style_has_a_look_of_its_own() {
        for (i, style) in PictureStyle::ALL.into_iter().enumerate() {
            let look = style.look();
            assert_eq!(PictureStyle::of(look.shape, look.outline, &look.effects), Some(style), "{style:?}");
            for other in &PictureStyle::ALL[..i] {
                assert_ne!(other.look(), look, "{style:?} and {other:?}");
                assert_ne!(other.name(Locale::en()), style.name(Locale::en()));
            }
        }
        assert_eq!(PictureStyle::of(ShapePreset::Rect, None, &Effects::default()), None);
    }

    #[test]
    fn bevel_profiles_rise_from_the_rim_to_the_face() {
        for preset in BevelPreset::ALL {
            assert!(preset.profile(0.0).abs() < 1e-4, "{preset:?}");
            assert!((preset.profile(1.0) - 1.0).abs() < 1e-3, "{preset:?}");
            assert_eq!(BevelPreset::from_ooxml_name(preset.ooxml_name()), Some(preset));
            for step in 0..=20 {
                let height = preset.profile(step as f32 / 20.0);
                assert!((-0.01..=1.6).contains(&height), "{preset:?} {height}");
            }
        }
    }

    #[test]
    fn a_camera_projects_the_corners_of_a_frame() {
        let flat = Rotation3d { x: 0.0, y: 0.0, z: 0.0, perspective: 0.0 };
        assert!(flat.is_flat());
        assert_eq!(flat.project(200.0, 100.0, 0.0, 0.0), (-100.0, -50.0, 1.0));
        // A quarter turn in the plane: the top-left corner goes to the
        // top right (clockwise on a screen whose y grows downwards).
        let turned = Rotation3d { z: 270.0, ..flat };
        let (x, y, _) = turned.project(200.0, 100.0, 0.0, 0.0);
        assert!((x - 50.0).abs() < 1e-3 && (y + 100.0).abs() < 1e-3, "{x} {y}");
        // Turned about the vertical axis, seen in parallel: narrower,
        // as tall.
        let side = Rotation3d { x: 60.0, ..flat };
        let (x, y, _) = side.project(200.0, 100.0, 1.0, 1.0);
        assert!((x - 50.0).abs() < 1e-3 && (y - 50.0).abs() < 1e-3);
        // In perspective the near side is taller than the far one.
        let seen = Rotation3d { x: 40.0, perspective: 45.0, ..flat };
        let (_, left, _) = seen.project(200.0, 100.0, 0.0, 1.0);
        let (_, right, _) = seen.project(200.0, 100.0, 1.0, 1.0);
        assert!((left - right).abs() > 5.0);
        for camera in Camera::ALL {
            assert_eq!(Camera::from_ooxml_name(camera.ooxml_name()), Some(camera));
            assert_eq!(camera.parallel(), !camera.ooxml_name().starts_with("perspective"));
        }
    }

    #[test]
    fn a_second_image_stands_for_the_settings_it_was_made_for() {
        // The smallest PNG header that says 2 by 1 pixels.
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x02\0\0\0\x01\x08\x06\0\0\0".to_vec();
        png.extend_from_slice(&[0; 8]);
        let mut picture = Picture::new(png.clone()).unwrap();
        assert!(!picture.has_baked_effects() && !picture.needs_baking());
        assert!(Arc::ptr_eq(picture.shown().0, &picture.data));
        picture.artistic = Some(Artistic::preset(ArtisticKind::Blur));
        picture.adjust.saturation = 2.0;
        assert!(picture.needs_baking());
        assert_eq!(picture.shown().1.saturation, 2.0);
        picture.baked = Some(Baked {
            data: png.into(),
            saturation: 2.0,
            temperature: NEUTRAL_TEMPERATURE,
            artistic: picture.artistic,
        });
        assert!(!picture.needs_baking());
        let (bytes, adjust) = picture.shown();
        assert!(!Arc::ptr_eq(bytes, &picture.data));
        assert_eq!(adjust.saturation, 1.0);
        // Another setting: the image no longer stands for the picture.
        picture.artistic = Some(Artistic { amount: 30.0, ..Artistic::preset(ArtisticKind::Blur) });
        assert!(picture.needs_baking());
        for kind in ArtisticKind::ALL {
            let (low, high, preset) = kind.amount_range();
            assert!(low <= preset && preset <= high);
            assert_eq!(ArtisticKind::from_ooxml_name(kind.ooxml().0), Some(kind));
        }
    }
}
