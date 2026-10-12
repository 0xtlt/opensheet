//! The text of a shape as it is drawn: its font and colour, and its lines
//! wrapped at the width of the box they sit in. The lines are wrapped the
//! way `widgets::text_area` wraps them, so that a text does not move
//! between being typed (in that widget, placed over the shape) and being
//! shown.

use gpui::{
    Font, Hsla, Pixels, SharedString, StrikethroughStyle, TextRun, UnderlineStyle, WindowTextSystem, WrappedLine, px,
};
use opensheet_engine::drawing::{TextAlign, TextAnchor};

/// How the text of a shape looks.
#[derive(Clone, Debug, PartialEq)]
pub struct TextLook {
    pub font: Font,
    /// Size in pixels.
    pub size: f32,
    pub color: Hsla,
    pub underline: bool,
    pub strike: bool,
    pub align: TextAlign,
    pub anchor: TextAnchor,
}

impl TextLook {
    pub fn line_height(&self) -> Pixels {
        px((self.size * 1.2).round().max(1.0))
    }

    /// How each line sits in the width of its box. A justified text is
    /// drawn aligned left: GPUI does not stretch lines.
    pub fn align(&self) -> gpui::TextAlign {
        match self.align {
            TextAlign::Left | TextAlign::Justify => gpui::TextAlign::Left,
            TextAlign::Center => gpui::TextAlign::Center,
            TextAlign::Right => gpui::TextAlign::Right,
        }
    }

    /// The paragraphs of a text wrapped in a box `width` wide, and the
    /// number of rows they take.
    pub fn wrap(&self, text: &str, width: f32, ts: &WindowTextSystem) -> (Vec<WrappedLine>, usize) {
        let decoration = (self.size / 14.0).max(1.0);
        let run = TextRun {
            len: text.len(),
            font: self.font.clone(),
            color: self.color,
            background_color: None,
            underline: self.underline.then_some(UnderlineStyle {
                color: Some(self.color),
                thickness: px(decoration),
                wavy: false,
            }),
            strikethrough: self.strike.then_some(StrikethroughStyle {
                color: Some(self.color),
                thickness: px(decoration),
            }),
        };
        let lines: Vec<WrappedLine> = ts
            .shape_text(SharedString::from(text.to_string()), px(self.size), &[run], Some(px(width.max(8.0))), None)
            .map(|lines| lines.into_iter().collect())
            .unwrap_or_default();
        let rows = lines.iter().map(|line| line.wrap_boundaries().len() + 1).sum();
        (lines, rows)
    }

    /// How far below the top of a box `height` high the first of `rows`
    /// rows starts: a text too tall for its box overflows it on the side
    /// it is not held at, on both when it is held in the middle.
    pub fn top(&self, rows: usize, height: f32) -> f32 {
        let total = rows as f32 * f32::from(self.line_height());
        match self.anchor {
            TextAnchor::Top => 0.0,
            TextAnchor::Middle => ((height - total) / 2.0).round(),
            TextAnchor::Bottom => height - total,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::ui_font;

    #[test]
    fn text_is_held_at_the_top_the_middle_or_the_bottom() {
        let mut look = TextLook {
            font: ui_font(false),
            size: 15.0,
            color: gpui::black(),
            underline: false,
            strike: false,
            align: TextAlign::Justify,
            anchor: TextAnchor::Top,
        };
        assert_eq!(look.line_height(), px(18.0));
        assert_eq!(look.align(), gpui::TextAlign::Left);
        assert_eq!(look.top(2, 100.0), 0.0);
        look.anchor = TextAnchor::Middle;
        assert_eq!(look.top(2, 100.0), 32.0);
        // Too tall for its box, it overflows above and below alike.
        assert_eq!(look.top(10, 100.0), -40.0);
        look.anchor = TextAnchor::Bottom;
        assert_eq!(look.top(2, 100.0), 64.0);
        look.align = TextAlign::Center;
        assert_eq!(look.align(), gpui::TextAlign::Center);
    }
}
