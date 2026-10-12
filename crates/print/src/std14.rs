//! The standard PDF fonts: Helvetica, Times and Courier in four styles
//! each. Every PDF reader has them, so they need no font file, only their
//! metrics, which are built in here. They cover Windows Latin 1 (all of
//! English, French, Spanish, German and Italian). Symbol and ZapfDingbats,
//! the other two of the fourteen, have encodings of their own and are not
//! used for cell text.

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Standard {
    Helvetica,
    HelveticaBold,
    HelveticaOblique,
    HelveticaBoldOblique,
    TimesRoman,
    TimesBold,
    TimesItalic,
    TimesBoldItalic,
    Courier,
    CourierBold,
    CourierOblique,
    CourierBoldOblique,
}

/// Vertical metrics in thousandths of the font size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Metrics {
    pub ascent: i16,
    /// Negative.
    pub descent: i16,
}

impl Standard {
    pub fn pick(serif: bool, mono: bool, bold: bool, italic: bool) -> Standard {
        use Standard::*;
        let styles = if mono {
            [Courier, CourierBold, CourierOblique, CourierBoldOblique]
        } else if serif {
            [TimesRoman, TimesBold, TimesItalic, TimesBoldItalic]
        } else {
            [Helvetica, HelveticaBold, HelveticaOblique, HelveticaBoldOblique]
        };
        styles[bold as usize + 2 * italic as usize]
    }

    /// The PDF name of the font (`/BaseFont`).
    pub fn base_font(self) -> &'static str {
        match self {
            Standard::Helvetica => "Helvetica",
            Standard::HelveticaBold => "Helvetica-Bold",
            Standard::HelveticaOblique => "Helvetica-Oblique",
            Standard::HelveticaBoldOblique => "Helvetica-BoldOblique",
            Standard::TimesRoman => "Times-Roman",
            Standard::TimesBold => "Times-Bold",
            Standard::TimesItalic => "Times-Italic",
            Standard::TimesBoldItalic => "Times-BoldItalic",
            Standard::Courier => "Courier",
            Standard::CourierBold => "Courier-Bold",
            Standard::CourierOblique => "Courier-Oblique",
            Standard::CourierBoldOblique => "Courier-BoldOblique",
        }
    }

    pub fn family(self) -> &'static str {
        match self {
            Standard::Helvetica
            | Standard::HelveticaBold
            | Standard::HelveticaOblique
            | Standard::HelveticaBoldOblique => "Helvetica",
            Standard::TimesRoman | Standard::TimesBold | Standard::TimesItalic | Standard::TimesBoldItalic => "Times",
            _ => "Courier",
        }
    }

    pub fn metrics(self) -> Metrics {
        let (ascent, descent) = match self.family() {
            "Helvetica" => (718, -207),
            "Times" => (683, -217),
            _ => (629, -157),
        };
        Metrics { ascent, descent }
    }

    /// Advance width of a Windows Latin 1 character code, in thousandths
    /// of the font size. The oblique Helveticas are as wide as the
    /// upright ones; every Courier character is 600 wide.
    pub fn width(self, code: u8) -> u16 {
        let table: &[u16; 224] = match self {
            Standard::Helvetica | Standard::HelveticaOblique => &HELVETICA,
            Standard::HelveticaBold | Standard::HelveticaBoldOblique => &HELVETICA_BOLD,
            Standard::TimesRoman => &TIMES_ROMAN,
            Standard::TimesBold => &TIMES_BOLD,
            Standard::TimesItalic => &TIMES_ITALIC,
            Standard::TimesBoldItalic => &TIMES_BOLD_ITALIC,
            _ => return 600,
        };
        if code < 32 { 0 } else { table[code as usize - 32] }
    }
}

/// The characters of codes 0x80 to 0x9F of Windows Latin 1 (0 where the
/// code is unused); the other codes are their own Unicode values.
const HIGH: [char; 32] = [
    '€', '\0', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\0', 'Ž', '\0', '\0', '‘', '’', '“', '”', '•',
    '–', '—', '˜', '™', 'š', '›', 'œ', '\0', 'ž', 'Ÿ',
];

/// The Windows Latin 1 (WinAnsiEncoding) code of a character.
pub fn win_ansi(ch: char) -> Option<u8> {
    match ch as u32 {
        0x20..=0x7E | 0xA0..=0xFF => Some(ch as u32 as u8),
        _ => HIGH.iter().position(|c| *c == ch && ch != '\0').map(|i| 0x80 + i as u8),
    }
}

// Widths of codes 32 to 255, from the fonts' published metrics.
const HELVETICA: [u16; 224] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556,
    556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833,
    722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556,
    556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334,
    260, 334, 584, 0, 556, 0, 222, 556, 333, 1000, 556, 556, 333, 1000, 667, 333, 1000, 0, 611, 0, 0, 222, 222, 333,
    333, 350, 556, 1000, 333, 1000, 500, 333, 944, 0, 500, 667, 278, 333, 556, 556, 556, 556, 260, 556, 333, 737, 370,
    556, 584, 333, 737, 333, 400, 584, 333, 333, 333, 556, 537, 278, 333, 333, 365, 556, 834, 834, 834, 611, 667, 667,
    667, 667, 667, 667, 1000, 722, 667, 667, 667, 667, 278, 278, 278, 278, 722, 722, 778, 778, 778, 778, 778, 584, 778,
    722, 722, 722, 722, 667, 667, 611, 556, 556, 556, 556, 556, 556, 889, 500, 556, 556, 556, 556, 278, 278, 278, 278,
    556, 556, 556, 556, 556, 556, 556, 584, 611, 556, 556, 556, 556, 500, 556, 500,
];
const HELVETICA_BOLD: [u16; 224] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556,
    556, 556, 556, 333, 333, 584, 584, 584, 611, 975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833,
    722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 333, 278, 333, 584, 556, 333, 556, 611, 556, 611,
    556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611, 611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389,
    280, 389, 584, 0, 556, 0, 278, 556, 500, 1000, 556, 556, 333, 1000, 667, 333, 1000, 0, 611, 0, 0, 278, 278, 500,
    500, 350, 556, 1000, 333, 1000, 556, 333, 944, 0, 500, 667, 278, 333, 556, 556, 556, 556, 280, 556, 333, 737, 370,
    556, 584, 333, 737, 333, 400, 584, 333, 333, 333, 611, 556, 278, 333, 333, 365, 556, 834, 834, 834, 611, 722, 722,
    722, 722, 722, 722, 1000, 722, 667, 667, 667, 667, 278, 278, 278, 278, 722, 722, 778, 778, 778, 778, 778, 584, 778,
    722, 722, 722, 722, 667, 667, 611, 556, 556, 556, 556, 556, 556, 889, 556, 556, 556, 556, 556, 278, 278, 278, 278,
    611, 611, 611, 611, 611, 611, 611, 584, 611, 611, 611, 611, 611, 556, 611, 556,
];
const TIMES_ROMAN: [u16; 224] = [
    250, 333, 408, 500, 500, 833, 778, 180, 333, 333, 500, 564, 250, 333, 250, 278, 500, 500, 500, 500, 500, 500, 500,
    500, 500, 500, 278, 278, 564, 564, 564, 444, 921, 722, 667, 667, 722, 611, 556, 722, 722, 333, 389, 722, 611, 889,
    722, 722, 556, 722, 667, 556, 611, 722, 722, 944, 722, 722, 611, 333, 278, 333, 469, 500, 333, 444, 500, 444, 500,
    444, 333, 500, 500, 278, 278, 500, 278, 778, 500, 500, 500, 500, 333, 389, 278, 500, 500, 722, 500, 500, 444, 480,
    200, 480, 541, 0, 500, 0, 333, 500, 444, 1000, 500, 500, 333, 1000, 556, 333, 889, 0, 611, 0, 0, 333, 333, 444,
    444, 350, 500, 1000, 333, 980, 389, 333, 722, 0, 444, 722, 250, 333, 500, 500, 500, 500, 200, 500, 333, 760, 276,
    500, 564, 333, 760, 333, 400, 564, 300, 300, 333, 500, 453, 250, 333, 300, 310, 500, 750, 750, 750, 444, 722, 722,
    722, 722, 722, 722, 889, 667, 611, 611, 611, 611, 333, 333, 333, 333, 722, 722, 722, 722, 722, 722, 722, 564, 722,
    722, 722, 722, 722, 722, 556, 500, 444, 444, 444, 444, 444, 444, 667, 444, 444, 444, 444, 444, 278, 278, 278, 278,
    500, 500, 500, 500, 500, 500, 500, 564, 500, 500, 500, 500, 500, 500, 500, 500,
];
const TIMES_BOLD: [u16; 224] = [
    250, 333, 555, 500, 500, 1000, 833, 278, 333, 333, 500, 570, 250, 333, 250, 278, 500, 500, 500, 500, 500, 500, 500,
    500, 500, 500, 333, 333, 570, 570, 570, 500, 930, 722, 667, 722, 722, 667, 611, 778, 778, 389, 500, 778, 667, 944,
    722, 778, 611, 778, 722, 556, 667, 722, 722, 1000, 722, 722, 667, 333, 278, 333, 581, 500, 333, 500, 556, 444, 556,
    444, 333, 500, 556, 278, 333, 556, 278, 833, 556, 500, 556, 556, 444, 389, 333, 556, 500, 722, 500, 500, 444, 394,
    220, 394, 520, 0, 500, 0, 333, 500, 500, 1000, 500, 500, 333, 1000, 556, 333, 1000, 0, 667, 0, 0, 333, 333, 500,
    500, 350, 500, 1000, 333, 1000, 389, 333, 722, 0, 444, 722, 250, 333, 500, 500, 500, 500, 220, 500, 333, 747, 300,
    500, 570, 333, 747, 333, 400, 570, 300, 300, 333, 556, 540, 250, 333, 300, 330, 500, 750, 750, 750, 500, 722, 722,
    722, 722, 722, 722, 1000, 722, 667, 667, 667, 667, 389, 389, 389, 389, 722, 722, 778, 778, 778, 778, 778, 570, 778,
    722, 722, 722, 722, 722, 611, 556, 500, 500, 500, 500, 500, 500, 722, 444, 444, 444, 444, 444, 278, 278, 278, 278,
    500, 556, 500, 500, 500, 500, 500, 570, 500, 556, 556, 556, 556, 500, 556, 500,
];
const TIMES_ITALIC: [u16; 224] = [
    250, 333, 420, 500, 500, 833, 778, 214, 333, 333, 500, 675, 250, 333, 250, 278, 500, 500, 500, 500, 500, 500, 500,
    500, 500, 500, 333, 333, 675, 675, 675, 500, 920, 611, 611, 667, 722, 611, 611, 722, 722, 333, 444, 667, 556, 833,
    667, 722, 611, 722, 611, 500, 556, 722, 611, 833, 611, 556, 556, 389, 278, 389, 422, 500, 333, 500, 500, 444, 500,
    444, 278, 500, 500, 278, 278, 444, 278, 722, 500, 500, 500, 500, 389, 389, 278, 500, 444, 667, 444, 444, 389, 400,
    275, 400, 541, 0, 500, 0, 333, 500, 556, 889, 500, 500, 333, 1000, 500, 333, 944, 0, 556, 0, 0, 333, 333, 556, 556,
    350, 500, 889, 333, 980, 389, 333, 667, 0, 389, 556, 250, 389, 500, 500, 500, 500, 275, 500, 333, 760, 276, 500,
    675, 333, 760, 333, 400, 675, 300, 300, 333, 500, 523, 250, 333, 300, 310, 500, 750, 750, 750, 500, 611, 611, 611,
    611, 611, 611, 889, 667, 611, 611, 611, 611, 333, 333, 333, 333, 722, 667, 722, 722, 722, 722, 722, 675, 722, 722,
    722, 722, 722, 556, 611, 500, 500, 500, 500, 500, 500, 500, 667, 444, 444, 444, 444, 444, 278, 278, 278, 278, 500,
    500, 500, 500, 500, 500, 500, 675, 500, 500, 500, 500, 500, 444, 500, 444,
];
const TIMES_BOLD_ITALIC: [u16; 224] = [
    250, 389, 555, 500, 500, 833, 778, 278, 333, 333, 500, 570, 250, 333, 250, 278, 500, 500, 500, 500, 500, 500, 500,
    500, 500, 500, 333, 333, 570, 570, 570, 500, 832, 667, 667, 667, 722, 667, 667, 722, 778, 389, 500, 667, 611, 889,
    722, 722, 611, 722, 667, 556, 611, 722, 667, 889, 667, 611, 611, 333, 278, 333, 570, 500, 333, 500, 500, 444, 500,
    444, 333, 500, 556, 278, 278, 500, 278, 778, 556, 500, 500, 500, 389, 389, 278, 556, 444, 667, 500, 444, 389, 348,
    220, 348, 570, 0, 500, 0, 333, 500, 500, 1000, 500, 500, 333, 1000, 556, 333, 944, 0, 611, 0, 0, 333, 333, 500,
    500, 350, 500, 1000, 333, 1000, 389, 333, 722, 0, 389, 611, 250, 389, 500, 500, 500, 500, 220, 500, 333, 747, 266,
    500, 606, 333, 747, 333, 400, 570, 300, 300, 333, 576, 500, 250, 333, 300, 300, 500, 750, 750, 750, 500, 667, 667,
    667, 667, 667, 667, 944, 667, 667, 667, 667, 667, 389, 389, 389, 389, 722, 722, 722, 722, 722, 722, 722, 570, 722,
    722, 722, 722, 722, 611, 611, 500, 500, 500, 500, 500, 500, 500, 722, 444, 444, 444, 444, 444, 278, 278, 278, 278,
    500, 556, 500, 500, 500, 500, 500, 570, 500, 556, 556, 556, 556, 444, 500, 444,
];

#[cfg(test)]
mod tests {
    use super::*;

    /// The character of a Windows Latin 1 code.
    fn win_ansi_char(code: u8) -> Option<char> {
        match code {
            0x20..=0x7E | 0xA0..=0xFF => Some(code as char),
            0x80..=0x9F => Some(HIGH[code as usize - 0x80]).filter(|c| *c != '\0'),
            _ => None,
        }
    }

    #[test]
    fn encoding() {
        assert_eq!(win_ansi('A'), Some(0x41));
        assert_eq!(win_ansi('é'), Some(0xE9));
        assert_eq!(win_ansi('€'), Some(0x80));
        assert_eq!(win_ansi('œ'), Some(0x9C));
        assert_eq!(win_ansi('\u{7f}'), None);
        assert_eq!(win_ansi('\0'), None);
        assert_eq!(win_ansi('Ω'), None);
        for code in 32..=255u8 {
            if let Some(ch) = win_ansi_char(code) {
                assert_eq!(win_ansi(ch), Some(code));
            }
        }
    }

    #[test]
    fn metrics() {
        // Values every copy of the Adobe metrics agrees on.
        assert_eq!(Standard::Helvetica.width(b' '), 278);
        assert_eq!(Standard::Helvetica.width(b'A'), 667);
        assert_eq!(Standard::Helvetica.width(b'W'), 944);
        assert_eq!(Standard::HelveticaOblique.width(b'a'), 556);
        assert_eq!(Standard::HelveticaBold.width(b'A'), 722);
        assert_eq!(Standard::HelveticaBoldOblique.width(b'a'), 556);
        assert_eq!(Standard::TimesRoman.width(b' '), 250);
        assert_eq!(Standard::TimesRoman.width(b'A'), 722);
        assert_eq!(Standard::TimesRoman.width(b'a'), 444);
        assert_eq!(Standard::TimesBold.width(b'a'), 500);
        assert_eq!(Standard::TimesItalic.width(b'A'), 611);
        assert_eq!(Standard::TimesBoldItalic.width(b'A'), 667);
        assert_eq!(Standard::CourierBold.width(b'i'), 600);
        // Accented letters are as wide as their base letter; digits all
        // have one width.
        for font in [Standard::Helvetica, Standard::TimesRoman, Standard::TimesBoldItalic] {
            assert_eq!(font.width(0xE9), font.width(b'e'));
            assert_eq!(font.width(0xC0), font.width(b'A'));
            assert!((b'0'..=b'9').all(|d| font.width(d) == font.width(b'0')));
        }
        assert_eq!(Standard::pick(false, false, true, true), Standard::HelveticaBoldOblique);
        assert_eq!(Standard::pick(true, false, false, true), Standard::TimesItalic);
        assert_eq!(Standard::pick(false, true, true, false), Standard::CourierBold);
        assert_eq!(Standard::TimesBoldItalic.base_font(), "Times-BoldItalic");
    }
}
