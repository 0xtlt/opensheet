//! The look of the letters of a shape's text: WordArt.
//!
//! WordArt is not an object of its own in Excel: it is a text box without
//! fill or outline whose letters have a fill, an outline and a shadow
//! (`a:solidFill`, `a:ln` and `a:effectLst` of `a:rPr`). Any shape can be
//! given those, and a text can change its look from one run of characters
//! to the next ([`TextRun`]).
//!
//! A shape keeps the look of its whole text in `Shape::font` and
//! `Shape::effects`; `Shape::runs` is empty then. A text formatted in
//! parts lists its runs, which cover it from end to end; `font` and
//! `effects` then hold the look of the first run, for whoever shows a text
//! in one look only.
//!
//! Not modelled: gradient and pattern fills of letters, glow, reflection,
//! bevels and the warped text of Transform.

use crate::drawing::{Shape, ShapeTool, TextAlign, TextAnchor};
use crate::style::{Color, Font, ThemeSlot};
use std::ops::Range;

/// The line around letters (`a:ln` of a run).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextOutline {
    pub color: Color,
    /// Width in points.
    pub width: f32,
}

/// A shadow behind letters (`a:outerShdw` of a run).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextShadow {
    pub color: Color,
    /// 0 (invisible) to 1.
    pub opacity: f32,
    /// How far the shadow spreads, in points; 0 is a hard shadow.
    pub blur: f32,
    /// How far from the letters it falls, in points.
    pub distance: f32,
    /// Where it falls, in degrees clockwise from the right.
    pub angle: f32,
}

impl TextShadow {
    /// The soft shadow below and right of the letters that Excel's
    /// Shadow menu offers first ("Offset: Bottom Right").
    pub fn offset(angle: f32) -> TextShadow {
        TextShadow {
            color: Color::BLACK,
            opacity: 0.4,
            blur: 4.0,
            distance: 3.0,
            angle,
        }
    }

    /// How far right and down the shadow falls, in points.
    pub fn shift(&self) -> (f32, f32) {
        let (sin, cos) = self.angle.to_radians().sin_cos();
        (self.distance * cos, self.distance * sin)
    }
}

/// What letters have besides their font and colour.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextEffects {
    /// The letters are not filled (`a:noFill`): only their outline shows.
    pub hollow: bool,
    pub outline: Option<TextOutline>,
    pub shadow: Option<TextShadow>,
}

impl TextEffects {
    /// Nothing a plain line of text cannot show.
    pub fn is_plain(&self) -> bool {
        *self == TextEffects::default()
    }
}

/// A run of characters in one look.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    /// Length in bytes of the text of the shape.
    pub len: usize,
    pub font: Font,
    pub effects: TextEffects,
}

impl Shape {
    /// The runs of the text with the bytes each covers: those of `runs`,
    /// or the whole text in the look of the shape.
    pub fn text_runs(&self) -> Vec<(Range<usize>, Font, TextEffects)> {
        if self.runs.is_empty() {
            return vec![(0..self.text.len(), self.font.clone(), self.effects)];
        }
        let mut at = 0;
        let mut out = Vec::with_capacity(self.runs.len());
        for run in &self.runs {
            let end = (at + run.len).min(self.text.len());
            out.push((at..end, run.font.clone(), run.effects));
            at = end;
        }
        // Runs that fall short of the text (they never should): the last
        // look goes on to the end.
        if let (true, Some(last)) = (at < self.text.len(), out.last_mut()) {
            last.0.end = self.text.len();
        }
        out
    }

    /// True when a letter of the text has an outline, a shadow or no fill.
    pub fn has_text_effects(&self) -> bool {
        !self.effects.is_plain() || self.runs.iter().any(|run| !run.effects.is_plain())
    }

    /// Replace the text, keeping the look of what stays: the characters
    /// that changed are found between what the old and the new text have
    /// in common at their start and at their end, and what was typed
    /// there takes the look of the character before it.
    pub fn set_text(&mut self, text: String) {
        if !self.runs.is_empty() {
            let (old, new) = (self.text.as_bytes(), text.as_bytes());
            let mut prefix = old.iter().zip(new).take_while(|(a, b)| a == b).count();
            while !self.text.is_char_boundary(prefix) {
                prefix -= 1;
            }
            let room = old.len().min(new.len()) - prefix;
            let mut suffix = old.iter().rev().zip(new.iter().rev()).take(room).take_while(|(a, b)| a == b).count();
            while !self.text.is_char_boundary(old.len() - suffix) {
                suffix -= 1;
            }
            let (removed, added) = (old.len() - prefix - suffix, new.len() - prefix - suffix);
            // Take the removed bytes out of the runs they were in, then
            // give the new ones to the run holding the byte before them.
            let mut at = 0;
            let mut grown = false;
            for run in &mut self.runs {
                let (start, end) = (at, at + run.len);
                at = end;
                let cut = end.min(prefix + removed).saturating_sub(start.max(prefix));
                run.len -= cut;
                if !grown && (end >= prefix && prefix > start || prefix == 0) {
                    run.len += added;
                    grown = true;
                }
            }
            if let (false, Some(last)) = (grown, self.runs.last_mut()) {
                last.len += added;
            }
            self.runs.retain(|run| run.len > 0);
        }
        self.text = text;
        self.tidy_runs();
    }

    /// Change the look of a range of the text (bytes; the whole text when
    /// it covers it or is empty).
    pub fn format_text(&mut self, range: Range<usize>, change: &dyn Fn(&mut Font, &mut TextEffects)) {
        let whole = range.is_empty() || (range.start == 0 && range.end >= self.text.len());
        if whole {
            change(&mut self.font, &mut self.effects);
            for run in &mut self.runs {
                change(&mut run.font, &mut run.effects);
            }
            self.tidy_runs();
            return;
        }
        let mut pieces = Vec::new();
        for (covered, font, effects) in self.text_runs() {
            // Cut the run where the range starts and ends inside it.
            let mut cuts = vec![covered.start, covered.end];
            cuts.extend([range.start, range.end].into_iter().filter(|at| *at > covered.start && *at < covered.end));
            cuts.sort_unstable();
            for pair in cuts.windows(2) {
                let mut run = TextRun {
                    len: pair[1] - pair[0],
                    font: font.clone(),
                    effects,
                };
                if pair[0] >= range.start && pair[1] <= range.end {
                    change(&mut run.font, &mut run.effects);
                }
                pieces.push(run);
            }
        }
        self.runs = pieces;
        self.tidy_runs();
    }

    /// Bring the runs in order: neighbours of one look become one run, a
    /// text of one look has no runs, and the look of the first run is the
    /// look of the shape.
    fn tidy_runs(&mut self) {
        let mut merged: Vec<TextRun> = Vec::with_capacity(self.runs.len());
        for run in self.runs.drain(..).filter(|run| run.len > 0) {
            match merged.last_mut() {
                Some(last) if last.font == run.font && last.effects == run.effects => last.len += run.len,
                _ => merged.push(run),
            }
        }
        if let Some(first) = merged.first() {
            self.font = first.font.clone();
            self.effects = first.effects;
        }
        if merged.len() > 1 {
            self.runs = merged;
        }
    }
}

// ---- the gallery ---------------------------------------------------------

/// The colours of the theme the gallery is made of, here with their
/// values in the Office theme: a shape that takes a style is resolved in
/// the theme of its workbook (`Theme::resolve_shape`, which `add_drawing`
/// calls), and follows it from then on.
const TEXT_1: Color = Color::themed(ThemeSlot::Dark1, 0.0, 0x000000);
const BACKGROUND_1: Color = Color::themed(ThemeSlot::Light1, 0.0, 0xFFFFFF);
const ACCENT_1: Color = Color::themed(ThemeSlot::Accent1, 0.0, 0x156082);
const ACCENT_2: Color = Color::themed(ThemeSlot::Accent2, 0.0, 0xE97132);
const ACCENT_3: Color = Color::themed(ThemeSlot::Accent3, 0.0, 0x196B24);
const ACCENT_4: Color = Color::themed(ThemeSlot::Accent4, 0.0, 0x0F9ED5);
const ACCENT_5: Color = Color::themed(ThemeSlot::Accent5, 0.0, 0xA02B93);
const GRAY: Color = Color::themed(ThemeSlot::Dark1, 0.5, 0x7F7F7F);
/// Background 1, 25 % darker; accent 1, 60 % lighter.
const SILVER: Color = Color::themed(ThemeSlot::Light1, -0.25, 0xBFBFBF);
const ACCENT_1_PALE: Color = Color::themed(ThemeSlot::Accent1, 0.6, 0xA1CFE1);

/// Size of the text of a new WordArt, in points.
pub const WORDART_SIZE_PT: f32 = 54.0;

/// The styles of the WordArt gallery (Insert ▸ WordArt and the WordArt
/// Styles of the Shape Format tab), in Excel's order: three rows of five.
/// Gradients, bevels, reflections and glows are left out: each style
/// keeps its fill, its outline and its shadow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WordArtStyle(pub u8);

impl WordArtStyle {
    pub const COUNT: u8 = 15;

    pub fn all() -> impl Iterator<Item = WordArtStyle> {
        (0..WordArtStyle::COUNT).map(WordArtStyle)
    }

    /// The colour of the letters and what they have besides.
    pub fn look(self) -> (Color, TextEffects) {
        let soft = |color: Color, opacity: f32| TextShadow {
            color,
            opacity,
            blur: 3.0,
            distance: 3.0,
            angle: 45.0,
        };
        let hard = |color: Color| TextShadow {
            color,
            opacity: 1.0,
            blur: 0.0,
            distance: 3.0,
            angle: 45.0,
        };
        let line = |color: Color, width: f32| Some(TextOutline { color, width });
        let (fill, outline, shadow) = match self.0 {
            // Fill: text colour 1; shadow.
            0 => (TEXT_1, None, Some(soft(TEXT_1, 0.4))),
            // Fill: accent 1; shadow.
            1 => (ACCENT_1, None, Some(soft(TEXT_1, 0.4))),
            // Fill: accent 2; outline: accent 2.
            2 => (ACCENT_2, line(ACCENT_2, 0.5), None),
            // Fill: white; outline: accent 1; shadow.
            3 => (BACKGROUND_1, line(ACCENT_1, 1.0), Some(soft(ACCENT_1, 0.4))),
            // Fill: accent 4 (the bevel is left out).
            4 => (ACCENT_4, None, Some(soft(TEXT_1, 0.3))),
            // Gradient fill: grey.
            5 => (GRAY, None, None),
            // Gradient fill: accent 5 (the reflection is left out).
            6 => (ACCENT_5, None, None),
            // Gradient fill: accent 4; outline: accent 4.
            7 => (ACCENT_4, line(ACCENT_4, 0.75), None),
            // Fill: white; outline: accent 5; shadow.
            8 => (BACKGROUND_1, line(ACCENT_5, 1.0), Some(soft(ACCENT_5, 0.4))),
            // Fill: accent 3 (the bevel is left out).
            9 => (ACCENT_3, None, Some(soft(TEXT_1, 0.3))),
            // Fill: text colour 1; outline: white; hard shadow: white.
            10 => (TEXT_1, line(BACKGROUND_1, 0.75), Some(hard(SILVER))),
            // Fill: text colour 1; outline: white; hard shadow: accent 5.
            11 => (TEXT_1, line(BACKGROUND_1, 0.75), Some(hard(ACCENT_5))),
            // Fill: accent 1; outline: white; hard shadow: accent 1.
            12 => (ACCENT_1, line(BACKGROUND_1, 0.75), Some(hard(ACCENT_1_PALE))),
            // Fill: white; outline: accent 1 (the glow is left out).
            13 => (BACKGROUND_1, line(ACCENT_1, 1.5), None),
            // Pattern fill: accent 1; hard shadow: accent 1.
            _ => (ACCENT_1_PALE, line(ACCENT_1, 0.5), Some(hard(ACCENT_1))),
        };
        (
            fill,
            TextEffects {
                hollow: false,
                outline,
                shadow,
            },
        )
    }

    /// Give a font and its effects the look of the style; the font, its
    /// size and its weight stay.
    pub fn apply(self, font: &mut Font, effects: &mut TextEffects) {
        let (fill, look) = self.look();
        font.color = Some(fill);
        *effects = look;
    }

    /// A new WordArt as Excel inserts it: a text box without fill or
    /// outline whose text is not wrapped, in large letters of the style.
    pub fn shape(self, text: &str) -> Shape {
        let mut shape = ShapeTool::TextBox.shape();
        shape.fill = None;
        shape.outline = None;
        shape.wrap = false;
        shape.align = TextAlign::Left;
        shape.anchor = TextAnchor::Top;
        shape.text = text.to_string();
        shape.font.set_size_pt(WORDART_SIZE_PT);
        self.apply(&mut shape.font, &mut shape.effects);
        shape
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rich(text: &str) -> Shape {
        let mut shape = ShapeTool::Rectangle.shape();
        shape.text = text.to_string();
        shape
    }

    fn lens(shape: &Shape) -> Vec<usize> {
        shape.runs.iter().map(|run| run.len).collect()
    }

    #[test]
    fn a_range_takes_a_look_of_its_own() {
        let mut shape = rich("Total sales");
        assert_eq!(shape.text_runs().len(), 1);
        shape.format_text(6..11, &|font, _| font.bold = true);
        assert_eq!(lens(&shape), vec![6, 5]);
        assert!(!shape.runs[0].font.bold && shape.runs[1].font.bold);
        // The look of the shape is that of its first run.
        assert!(!shape.font.bold);
        // The same look next to it joins the run; over everything, the
        // runs go.
        shape.format_text(0..6, &|font, _| font.bold = true);
        assert!(shape.runs.is_empty() && shape.font.bold);
        // A change of the whole text keeps what tells the runs apart.
        shape.format_text(0..5, &|font, _| font.italic = true);
        shape.format_text(0..0, &|font, _| font.color = Some(Color(0xFF0000)));
        assert_eq!(lens(&shape), vec![5, 6]);
        assert!(shape.runs.iter().all(|run| run.font.color == Some(Color(0xFF0000)) && run.font.bold));
        assert!(shape.runs[0].font.italic && !shape.runs[1].font.italic);
        // A range inside one run cuts it in three.
        shape.format_text(1..3, &|_, effects| effects.hollow = true);
        assert_eq!(lens(&shape), vec![1, 2, 2, 6]);
        assert!(shape.has_text_effects());
    }

    #[test]
    fn typing_keeps_the_runs() {
        let mut shape = rich("Total sales");
        shape.format_text(6..11, &|font, _| font.bold = true);
        // Typed where the first run ends: it takes the look of the
        // character before it.
        shape.set_text("Total net sales".to_string());
        assert_eq!(lens(&shape), vec![10, 5]);
        shape.set_text("Totals net sales".to_string());
        assert_eq!(lens(&shape), vec![11, 5]);
        // Deleted inside the first run.
        shape.set_text("Tot sales".to_string());
        assert_eq!(lens(&shape), vec![4, 5]);
        // Typed in the second.
        shape.set_text("Tot sXales".to_string());
        assert_eq!(lens(&shape), vec![4, 6]);
        shape.set_text("Tot sales".to_string());
        // A run that lost all its characters is gone.
        shape.set_text("sales".to_string());
        assert!(shape.runs.is_empty() && shape.font.bold);
        // Accented letters are not cut in two.
        let mut shape = rich("été");
        shape.format_text(0..2, &|font, _| font.italic = true);
        shape.set_text("èté".to_string());
        assert_eq!(lens(&shape), vec![2, 3]);
        shape.set_text(String::new());
        assert!(shape.runs.is_empty());
        assert_eq!(shape.text_runs(), vec![(0..0, shape.font.clone(), shape.effects)]);
    }

    #[test]
    fn styles_of_the_gallery() {
        assert_eq!(WordArtStyle::all().count(), 15);
        let shape = WordArtStyle(3).shape("Your text here");
        assert!(shape.text_box && shape.fill.is_none() && shape.outline.is_none() && !shape.wrap);
        assert_eq!(shape.font.size_pt(), WORDART_SIZE_PT);
        assert_eq!(shape.font.color, Some(BACKGROUND_1));
        assert!(shape.font.color.is_some_and(|color| color.is_themed() && color.value() == 0xFFFFFF));
        assert_eq!(shape.effects.outline.map(|line| line.color), Some(ACCENT_1));
        assert!(shape.effects.shadow.is_some() && shape.has_text_effects());
        // A style changes the look of letters, not their font.
        let mut font = Font {
            bold: true,
            ..Font::default()
        };
        let mut effects = TextEffects::default();
        WordArtStyle(2).apply(&mut font, &mut effects);
        assert!(font.bold && font.size == Font::default().size);
        assert_eq!(effects.outline, Some(TextOutline { color: ACCENT_2, width: 0.5 }));
        // A shadow falls where its angle says.
        let (dx, dy) = TextShadow::offset(90.0).shift();
        assert!(dx.abs() < 1e-4 && (dy - 3.0).abs() < 1e-4);
    }
}
