//! Headers and footers.
//!
//! A header or a footer is stored the way OOXML stores it: one string of
//! text and `&` codes holding the left, centre and right sections
//! (`&LDraft&C&"Arial,Bold"&14Budget&RPage &P of &N`). This module splits
//! such a string into its sections and expands a section for a given page
//! into runs of formatted text.

use crate::format;
use crate::locale::Locale;
use crate::style::{Color, Underline};
use crate::value::Value;
use std::sync::Arc;

/// The headers and footers of a sheet (`headerFooter` in OOXML).
#[derive(Clone, Debug, PartialEq)]
pub struct HeaderFooter {
    /// The header of every page, or of odd pages when
    /// `different_odd_even` is set.
    pub odd_header: String,
    pub odd_footer: String,
    pub even_header: String,
    pub even_footer: String,
    pub first_header: String,
    pub first_footer: String,
    pub different_odd_even: bool,
    pub different_first: bool,
    /// Headers and footers shrink or grow with the sheet's print scale.
    pub scale_with_doc: bool,
    /// The left and right sections start at the page margins rather than
    /// at a fixed distance from the paper edges.
    pub align_with_margins: bool,
    /// The pictures the `&G` codes stand for.
    pub pictures: Vec<HfPicture>,
}

impl Default for HeaderFooter {
    fn default() -> HeaderFooter {
        HeaderFooter {
            odd_header: String::new(),
            odd_footer: String::new(),
            even_header: String::new(),
            even_footer: String::new(),
            first_header: String::new(),
            first_footer: String::new(),
            different_odd_even: false,
            different_first: false,
            scale_with_doc: true,
            align_with_margins: true,
            pictures: Vec::new(),
        }
    }
}

/// Which of the three headers (or footers) a page uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HfVariant {
    Odd,
    Even,
    First,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HfSection {
    Left,
    Center,
    Right,
}

/// One section of one header or footer: where a `&G` picture sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HfSlot {
    pub variant: HfVariant,
    pub footer: bool,
    pub section: HfSection,
}

impl HfSlot {
    /// The id of the picture's shape in the sheet's legacy VML drawing
    /// (`LH`, `CF`, `RHEVEN`, `LFFIRST`...).
    pub fn vml_id(&self) -> String {
        let section = match self.section {
            HfSection::Left => 'L',
            HfSection::Center => 'C',
            HfSection::Right => 'R',
        };
        let suffix = match self.variant {
            HfVariant::Odd => "",
            HfVariant::Even => "EVEN",
            HfVariant::First => "FIRST",
        };
        format!("{section}{}{suffix}", if self.footer { 'F' } else { 'H' })
    }

    pub fn from_vml_id(id: &str) -> Option<HfSlot> {
        let mut chars = id.chars();
        let section = match chars.next()? {
            'L' => HfSection::Left,
            'C' => HfSection::Center,
            'R' => HfSection::Right,
            _ => return None,
        };
        let footer = match chars.next()? {
            'H' => false,
            'F' => true,
            _ => return None,
        };
        let variant = match chars.as_str() {
            "" => HfVariant::Odd,
            "EVEN" => HfVariant::Even,
            "FIRST" => HfVariant::First,
            _ => return None,
        };
        Some(HfSlot {
            variant,
            footer,
            section,
        })
    }
}

/// A picture placed in a header or footer by `&G`.
#[derive(Clone, Debug, PartialEq)]
pub struct HfPicture {
    pub slot: HfSlot,
    /// The image file as stored in the workbook (PNG, JPEG...).
    pub data: Arc<[u8]>,
    /// Printed width and height in points.
    pub size: (f32, f32),
}

impl HeaderFooter {
    /// True when no page prints a header or a footer.
    pub fn is_empty(&self) -> bool {
        self.odd_header.is_empty()
            && self.odd_footer.is_empty()
            && (!self.different_odd_even
                || (self.even_header.is_empty() && self.even_footer.is_empty()))
            && (!self.different_first
                || (self.first_header.is_empty() && self.first_footer.is_empty()))
    }

    /// The variant a page uses. `first` is true for the first page the
    /// sheet prints; odd and even follow the printed page number.
    pub fn variant(&self, first: bool, page_number: i32) -> HfVariant {
        if first && self.different_first {
            HfVariant::First
        } else if self.different_odd_even && page_number % 2 == 0 {
            HfVariant::Even
        } else {
            HfVariant::Odd
        }
    }

    pub fn header(&self, variant: HfVariant) -> &str {
        match variant {
            HfVariant::Odd => &self.odd_header,
            HfVariant::Even => &self.even_header,
            HfVariant::First => &self.first_header,
        }
    }

    pub fn footer(&self, variant: HfVariant) -> &str {
        match variant {
            HfVariant::Odd => &self.odd_footer,
            HfVariant::Even => &self.even_footer,
            HfVariant::First => &self.first_footer,
        }
    }

    pub fn header_mut(&mut self, variant: HfVariant) -> &mut String {
        match variant {
            HfVariant::Odd => &mut self.odd_header,
            HfVariant::Even => &mut self.even_header,
            HfVariant::First => &mut self.first_header,
        }
    }

    pub fn footer_mut(&mut self, variant: HfVariant) -> &mut String {
        match variant {
            HfVariant::Odd => &mut self.odd_footer,
            HfVariant::Even => &mut self.even_footer,
            HfVariant::First => &mut self.first_footer,
        }
    }

    pub fn picture(&self, slot: HfSlot) -> Option<&HfPicture> {
        self.pictures.iter().find(|p| p.slot == slot)
    }
}

// ------------------------------------------------------------------ tokens

/// One element of a code string.
#[derive(Clone, Debug, PartialEq)]
enum Tok<'a> {
    Text(&'a str),
    /// `&&`.
    Ampersand,
    Section(HfSection),
    /// `&P`, with the number added by `&P+n` / `&P-n`.
    Page(i64),
    Pages,
    Date,
    Time,
    Path,
    File,
    Tab,
    Picture,
    /// `&"name,style"`.
    Font(&'a str, &'a str),
    Size(u32),
    Bold,
    Italic,
    Underline,
    DoubleUnderline,
    Strike,
    Superscript,
    Subscript,
    Color(Color),
    /// A code Excel does not know: it prints nothing.
    Unknown,
}

/// Split a code string into tokens. Every token also reports the slice of
/// the source it was read from, so callers can copy codes through.
fn tokens(code: &str) -> Vec<(Tok<'_>, &str)> {
    let mut out = Vec::new();
    let bytes = code.as_bytes();
    let mut i = 0;
    let mut text_start = 0;
    while i < bytes.len() {
        if bytes[i] != b'&' {
            i += 1;
            continue;
        }
        if text_start < i {
            out.push((Tok::Text(&code[text_start..i]), &code[text_start..i]));
        }
        let start = i;
        i += 1;
        let Some(ch) = code[i..].chars().next() else {
            // A lone trailing ampersand prints nothing.
            out.push((Tok::Unknown, &code[start..i]));
            text_start = i;
            break;
        };
        i += ch.len_utf8();
        let tok = match ch.to_ascii_uppercase() {
            '&' => Tok::Ampersand,
            'L' => Tok::Section(HfSection::Left),
            'C' => Tok::Section(HfSection::Center),
            'R' => Tok::Section(HfSection::Right),
            'P' => {
                // `&P+3` and `&P-1` shift the page number.
                let rest = &code[i..];
                let digits = rest
                    .strip_prefix(['+', '-'])
                    .map(|d| d.bytes().take_while(|b| b.is_ascii_digit()).count())
                    .unwrap_or(0);
                if digits > 0 {
                    let n: i64 = rest[1..1 + digits].parse().unwrap_or(0);
                    let n = if rest.starts_with('-') { -n } else { n };
                    i += 1 + digits;
                    Tok::Page(n)
                } else {
                    Tok::Page(0)
                }
            }
            'N' => Tok::Pages,
            'D' => Tok::Date,
            'T' => Tok::Time,
            'Z' => Tok::Path,
            'F' => Tok::File,
            'A' => Tok::Tab,
            'G' => Tok::Picture,
            'B' => Tok::Bold,
            'I' => Tok::Italic,
            'U' => Tok::Underline,
            'E' => Tok::DoubleUnderline,
            'S' => Tok::Strike,
            'X' => Tok::Superscript,
            'Y' => Tok::Subscript,
            '"' => match code[i..].find('"') {
                Some(end) => {
                    let inner = &code[i..i + end];
                    i += end + 1;
                    let (name, style) = inner.split_once(',').unwrap_or((inner, ""));
                    Tok::Font(name.trim(), style.trim())
                }
                None => {
                    i = bytes.len();
                    Tok::Unknown
                }
            },
            '0'..='9' => {
                let more = code[i..]
                    .bytes()
                    .take(2)
                    .take_while(|b| b.is_ascii_digit())
                    .count();
                let n: u32 = code[i - 1..i + more].parse().unwrap_or(11);
                i += more;
                Tok::Size(n)
            }
            'K' => match parse_color(&code[i..]) {
                Some((color, len)) => {
                    i += len;
                    Tok::Color(color)
                }
                None => Tok::Unknown,
            },
            _ => Tok::Unknown,
        };
        out.push((tok, &code[start..i]));
        text_start = i;
    }
    if text_start < bytes.len() {
        out.push((Tok::Text(&code[text_start..]), &code[text_start..]));
    }
    out
}

/// Colours of the Office theme, in the order header and footer codes
/// number them: background 1, text 1, background 2, text 2, six accents,
/// hyperlink, followed hyperlink.
const THEME: [u32; 12] = [
    0xFFFFFF, 0x000000, 0xE8E8E8, 0x0E2841, 0x156082, 0xE97132, 0x196B24, 0x0F9ED5, 0xA02B93,
    0x4EA72E, 0x467886, 0x96607D,
];

/// The argument of `&K`: `RRGGBB`, or a theme colour `TT+NNN` / `TT-NNN`
/// (theme index, then how much lighter or darker in hundredths). Returns
/// the colour and the number of bytes read.
fn parse_color(s: &str) -> Option<(Color, usize)> {
    let head = s.get(..6)?;
    let b = head.as_bytes();
    if b.iter().all(|c| c.is_ascii_hexdigit()) {
        return Color::from_hex(head).map(|c| (c, 6));
    }
    let digits = |r: std::ops::Range<usize>| b[r].iter().all(|c| c.is_ascii_digit());
    if digits(0..2) && (b[2] == b'+' || b[2] == b'-') && digits(3..6) {
        let index: usize = head[..2].parse().ok()?;
        let amount: f32 = head[3..].parse::<f32>().ok()? / 100.0;
        let base = *THEME.get(index)?;
        let lighter = b[2] == b'+';
        let channel = |shift: u32| -> u32 {
            let v = ((base >> shift) & 0xFF) as f32;
            let out = if lighter {
                v + (255.0 - v) * amount
            } else {
                v * (1.0 - amount)
            };
            (out.round().clamp(0.0, 255.0) as u32) << shift
        };
        return Some((Color(channel(16) | channel(8) | channel(0)), 6));
    }
    None
}

// ---------------------------------------------------------------- sections

/// The left, centre and right sections of one header or footer, each a
/// code string without its `&L` / `&C` / `&R` marker.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HfSections {
    pub left: String,
    pub center: String,
    pub right: String,
}

impl HfSections {
    /// Split a stored header or footer. Text before any section marker
    /// belongs to the centre, as in Excel.
    pub fn parse(code: &str) -> HfSections {
        let mut out = HfSections::default();
        let mut current = HfSection::Center;
        for (tok, src) in tokens(code) {
            match tok {
                Tok::Section(s) => current = s,
                _ => out.get_mut(current).push_str(src),
            }
        }
        out
    }

    /// The stored form: `&L...&C...&R...`, empty sections left out.
    pub fn to_code(&self) -> String {
        let mut out = String::new();
        for (marker, text) in [
            ("&L", &self.left),
            ("&C", &self.center),
            ("&R", &self.right),
        ] {
            if !text.is_empty() {
                out.push_str(marker);
                out.push_str(text);
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.left.is_empty() && self.center.is_empty() && self.right.is_empty()
    }

    pub fn get(&self, section: HfSection) -> &str {
        match section {
            HfSection::Left => &self.left,
            HfSection::Center => &self.center,
            HfSection::Right => &self.right,
        }
    }

    pub fn get_mut(&mut self, section: HfSection) -> &mut String {
        match section {
            HfSection::Left => &mut self.left,
            HfSection::Center => &mut self.center,
            HfSection::Right => &mut self.right,
        }
    }
}

// --------------------------------------------------------------- expansion

/// What the codes of a header or footer stand for on a given page.
#[derive(Clone, Copy)]
pub struct HfContext<'a> {
    /// `&P`.
    pub page: i32,
    /// `&N`.
    pub pages: u32,
    /// The moment of printing, as a date serial number (`&D`, `&T`).
    pub now: f64,
    /// `&F`: the workbook's file name ("Budget.xlsx", or "Book1" before
    /// the first save).
    pub file_name: &'a str,
    /// `&Z`: the folder of the file, with its trailing separator; empty
    /// before the first save.
    pub file_path: &'a str,
    /// `&A`.
    pub sheet_name: &'a str,
    pub locale: &'static Locale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Script {
    #[default]
    Normal,
    Superscript,
    Subscript,
}

/// Character formatting of a run of header or footer text.
#[derive(Clone, Debug, PartialEq)]
pub struct HfFormat {
    /// `None` = the workbook's default font.
    pub font: Option<Arc<str>>,
    /// Points.
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: Underline,
    pub strike: bool,
    pub script: Script,
    /// `None` = automatic (black).
    pub color: Option<Color>,
}

impl Default for HfFormat {
    fn default() -> HfFormat {
        HfFormat {
            font: None,
            size: 11.0,
            bold: false,
            italic: false,
            underline: Underline::None,
            strike: false,
            script: Script::Normal,
            color: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum HfRun {
    /// Text in one format; may contain line breaks.
    Text { text: String, format: HfFormat },
    /// The section's picture (`&G`).
    Picture,
}

/// The words of a font style ("Bold Italic", "Gras italique") in the
/// languages Excel writes them in.
fn font_style(style: &str) -> (bool, bool) {
    let lower = style.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    (
        has(&["bold", "gras", "negrita", "fett", "grassetto"]),
        has(&[
            "italic", "italique", "cursiva", "kursiv", "corsivo", "oblique",
        ]),
    )
}

/// The time format of `&T`: the locale's, without the seconds.
fn short_time_format(locale: &Locale) -> String {
    locale.time_format.replace(":ss", "")
}

/// Expand one section of a header or footer for a page.
pub fn expand(section: &str, ctx: &HfContext) -> Vec<HfRun> {
    let mut runs: Vec<HfRun> = Vec::new();
    let mut fmt = HfFormat::default();
    let push = |runs: &mut Vec<HfRun>, fmt: &HfFormat, s: &str| {
        if s.is_empty() {
            return;
        }
        if let Some(HfRun::Text { text, format }) = runs.last_mut() {
            if format == fmt {
                text.push_str(s);
                return;
            }
        }
        runs.push(HfRun::Text {
            text: s.to_string(),
            format: fmt.clone(),
        });
    };
    for (tok, _) in tokens(section) {
        match tok {
            Tok::Text(s) => push(&mut runs, &fmt, s),
            Tok::Ampersand => push(&mut runs, &fmt, "&"),
            Tok::Section(_) | Tok::Unknown => {}
            Tok::Page(delta) => push(&mut runs, &fmt, &(ctx.page as i64 + delta).to_string()),
            Tok::Pages => push(&mut runs, &fmt, &ctx.pages.to_string()),
            Tok::Date => {
                let text = format::format_value(
                    &Value::Number(ctx.now.floor()),
                    ctx.locale.short_date_format,
                    ctx.locale,
                );
                push(&mut runs, &fmt, &text.text);
            }
            Tok::Time => {
                let text = format::format_value(
                    &Value::Number(ctx.now),
                    &short_time_format(ctx.locale),
                    ctx.locale,
                );
                push(&mut runs, &fmt, &text.text);
            }
            Tok::Path => push(&mut runs, &fmt, ctx.file_path),
            Tok::File => push(&mut runs, &fmt, ctx.file_name),
            Tok::Tab => push(&mut runs, &fmt, ctx.sheet_name),
            Tok::Picture => runs.push(HfRun::Picture),
            Tok::Font(name, style) => {
                fmt.font = (!name.is_empty() && name != "-").then(|| Arc::from(name));
                (fmt.bold, fmt.italic) = font_style(style);
            }
            Tok::Size(n) => fmt.size = n.clamp(1, 409) as f32,
            Tok::Bold => fmt.bold = !fmt.bold,
            Tok::Italic => fmt.italic = !fmt.italic,
            Tok::Underline => {
                fmt.underline = if fmt.underline == Underline::Single {
                    Underline::None
                } else {
                    Underline::Single
                }
            }
            Tok::DoubleUnderline => {
                fmt.underline = if fmt.underline == Underline::Double {
                    Underline::None
                } else {
                    Underline::Double
                }
            }
            Tok::Strike => fmt.strike = !fmt.strike,
            Tok::Superscript => {
                fmt.script = if fmt.script == Script::Superscript {
                    Script::Normal
                } else {
                    Script::Superscript
                }
            }
            Tok::Subscript => {
                fmt.script = if fmt.script == Script::Subscript {
                    Script::Normal
                } else {
                    Script::Subscript
                }
            }
            Tok::Color(c) => fmt.color = Some(c),
        }
    }
    runs
}

/// Break expanded runs into the lines they print on.
pub fn lines(runs: Vec<HfRun>) -> Vec<Vec<HfRun>> {
    let mut out: Vec<Vec<HfRun>> = vec![Vec::new()];
    for run in runs {
        match run {
            HfRun::Picture => out.last_mut().unwrap().push(HfRun::Picture),
            HfRun::Text { text, format } => {
                for (i, part) in text.split('\n').enumerate() {
                    if i > 0 {
                        out.push(Vec::new());
                    }
                    let part = part.trim_end_matches('\r');
                    if !part.is_empty() {
                        out.last_mut().unwrap().push(HfRun::Text {
                            text: part.to_string(),
                            format: format.clone(),
                        });
                    }
                }
            }
        }
    }
    out
}

/// A header or footer as plain text, its sections joined the way the
/// preset lists of the Page Setup dialog show them ("Sheet1, Page 1").
pub fn plain_text(code: &str, ctx: &HfContext) -> String {
    let sections = HfSections::parse(code);
    let sep = if ctx.locale.arg_sep == ';' {
        "; "
    } else {
        ", "
    };
    let mut parts: Vec<String> = Vec::new();
    for section in [&sections.left, &sections.center, &sections.right] {
        let text: String = expand(section, ctx)
            .into_iter()
            .map(|run| match run {
                HfRun::Text { text, .. } => text,
                HfRun::Picture => String::new(),
            })
            .collect();
        if !text.is_empty() {
            parts.push(text);
        }
    }
    parts.join(sep)
}

// ------------------------------------------------------- dialog spelling

/// The field codes as the Custom Header / Footer dialog spells them, in
/// en, fr, es, de, it order.
const FIELDS: [(&str, [&str; 5]); 8] = [
    ("&P", ["Page", "Page", "Página", "Seite", "Pagina"]),
    ("&N", ["Pages", "Pages", "Páginas", "Seiten", "Pagine"]),
    ("&D", ["Date", "Date", "Fecha", "Datum", "Data"]),
    ("&T", ["Time", "Heure", "Hora", "Zeit", "Ora"]),
    (
        "&Z",
        [
            "Path",
            "Chemin d'accès",
            "Ruta de acceso",
            "Pfad",
            "Percorso",
        ],
    ),
    ("&F", ["File", "Fichier", "Archivo", "Datei", "File"]),
    ("&A", ["Tab", "Onglet", "Etiqueta", "Register", "Scheda"]),
    ("&G", ["Picture", "Image", "Imagen", "Grafik", "Immagine"]),
];

fn field_locale(locale: &Locale) -> usize {
    match locale.id {
        "fr" => 1,
        "es" => 2,
        "de" => 3,
        "it" => 4,
        _ => 0,
    }
}

/// A section's code string as the Custom Header / Footer dialog shows it:
/// field codes spelled out (`&[Page]`, `&[Onglet]`), the rest unchanged.
pub fn to_dialog_text(section: &str, locale: &Locale) -> String {
    let li = field_locale(locale);
    let mut out = String::new();
    for (tok, src) in tokens(section) {
        let code = match tok {
            Tok::Page(_) => "&P",
            Tok::Pages => "&N",
            Tok::Date => "&D",
            Tok::Time => "&T",
            Tok::Path => "&Z",
            Tok::File => "&F",
            Tok::Tab => "&A",
            Tok::Picture => "&G",
            _ => {
                out.push_str(src);
                continue;
            }
        };
        let name = FIELDS
            .iter()
            .find(|(c, _)| *c == code)
            .map_or("", |(_, names)| names[li]);
        out.push_str("&[");
        out.push_str(name);
        out.push(']');
        // What follows the two letters of the code (`+1` of `&P+1`).
        out.push_str(&src[2..]);
    }
    out
}

/// The reverse of [`to_dialog_text`]. The English spellings are accepted
/// in every language; anything else is kept as typed.
pub fn from_dialog_text(text: &str, locale: &Locale) -> String {
    let li = field_locale(locale);
    let mut out = String::new();
    let mut rest = text;
    while let Some(pos) = rest.find("&[") {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 2..];
        let field = after.find(']').and_then(|end| {
            let name = after[..end].trim().to_lowercase();
            FIELDS
                .iter()
                .find(|(_, names)| {
                    names[li].to_lowercase() == name || names[0].to_lowercase() == name
                })
                .map(|(code, _)| (*code, end))
        });
        match field {
            Some((code, end)) => {
                out.push_str(code);
                rest = &after[end + 1..];
            }
            None => {
                out.push_str("&[");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

// ----------------------------------------------------------------- presets

/// The words of the built-in headers and footers: "Page", "of",
/// "Confidential", "Prepared by", in en, fr, es, de, it order.
const WORDS: [[&str; 4]; 5] = [
    ["Page", "of", "Confidential", "Prepared by"],
    ["Page", "de", "Confidentiel", "Préparé par"],
    ["Página", "de", "Confidencial", "Preparado por"],
    ["Seite", "von", "Vertraulich", "Erstellt von"],
    ["Pagina", "di", "Riservato", "Preparato da"],
];

/// The built-in headers and footers of the Page Setup dialog, as code
/// strings, in Excel's order; the first is "(none)". `user` is the name
/// Excel signs with (the Office user name).
pub fn presets(locale: &Locale, user: &str) -> Vec<String> {
    let [page, of, confidential, prepared] = WORDS[field_locale(locale)];
    // A literal ampersand in the user's name must not start a code.
    let user = user.replace('&', "&&");
    vec![
        String::new(),
        format!("&C{page} &P"),
        format!("&C{page} &P {of} &N"),
        "&C&A".to_string(),
        format!("&L{confidential}&C&D&R{page} &P"),
        "&C&F".to_string(),
        "&C&Z&F".to_string(),
        format!("&C&A&R{page} &P"),
        format!("&L&A&C{confidential}&R{page} &P"),
        format!("&C&F&R{page} &P"),
        format!("&C&Z&F&R{page} &P"),
        format!("&C{page} &P&R&A"),
        format!("&C{page} &P&R&F"),
        format!("&C{page} &P&R&Z&F"),
        format!("&L{user}&C{page} &P&R&D"),
        format!("&C{prepared} {user} &D&R{page} &P"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(locale: &'static Locale) -> HfContext<'static> {
        HfContext {
            page: 3,
            pages: 12,
            // 12 October 2026, 15:45.
            now: 46307.65625,
            file_name: "Budget.xlsx",
            file_path: "/Users/ana/",
            sheet_name: "T&L",
            locale,
        }
    }

    fn text(runs: &[HfRun]) -> String {
        runs.iter()
            .map(|r| match r {
                HfRun::Text { text, .. } => text.as_str(),
                HfRun::Picture => "<pic>",
            })
            .collect()
    }

    #[test]
    fn sections_split_and_join() {
        let s = HfSections::parse("&LLeft && more&C&\"Arial,Bold\"&14Title&RPage &P of &N");
        assert_eq!(s.left, "Left && more");
        assert_eq!(s.center, "&\"Arial,Bold\"&14Title");
        assert_eq!(s.right, "Page &P of &N");
        assert_eq!(
            s.to_code(),
            "&LLeft && more&C&\"Arial,Bold\"&14Title&RPage &P of &N"
        );
        // Without a marker the text is the centre section.
        assert_eq!(HfSections::parse("Hello").center, "Hello");
        // `&&L` is an ampersand followed by an L, not a section.
        assert_eq!(HfSections::parse("&CA&&L").center, "A&&L");
        // A font name containing a section letter is not a section.
        assert_eq!(
            HfSections::parse("&L&\"Lucida,Regular\"x").left,
            "&\"Lucida,Regular\"x"
        );
        assert!(HfSections::parse("").is_empty());
    }

    #[test]
    fn fields_expand() {
        let en = ctx(Locale::en());
        assert_eq!(text(&expand("Page &P of &N", &en)), "Page 3 of 12");
        assert_eq!(text(&expand("&P+2/&P-1", &en)), "5/2");
        assert_eq!(text(&expand("&D &T", &en)), "10/12/2026 3:45 PM");
        assert_eq!(
            text(&expand("&D &T", &ctx(Locale::fr()))),
            "12/10/2026 15:45"
        );
        assert_eq!(text(&expand("&D", &ctx(Locale::de()))), "12.10.2026");
        assert_eq!(
            text(&expand("&Z&F [&A]", &en)),
            "/Users/ana/Budget.xlsx [T&L]"
        );
        assert_eq!(text(&expand("R&&D &G&Q", &en)), "R&D <pic>");
        assert_eq!(text(&expand("a&", &en)), "a");
        // Codes are not case sensitive.
        assert_eq!(text(&expand("&p/&n", &en)), "3/12");
    }

    #[test]
    fn formatting_codes() {
        let en = ctx(Locale::en());
        let runs = expand(
            "a&Bb&Ic&B&I&\"Arial,Bold Italic\"&8d&\"-,Regular\"&K FF0000e&KFF0000f",
            &en,
        );
        let fmts: Vec<(String, HfFormat)> = runs
            .into_iter()
            .map(|r| match r {
                HfRun::Text { text, format } => (text, format),
                HfRun::Picture => unreachable!(),
            })
            .collect();
        assert_eq!(fmts[0].0, "a");
        assert_eq!(fmts[0].1, HfFormat::default());
        assert!(fmts[1].1.bold && !fmts[1].1.italic);
        assert!(fmts[2].1.bold && fmts[2].1.italic);
        assert_eq!(fmts[3].0, "d");
        assert_eq!(fmts[3].1.font.as_deref(), Some("Arial"));
        assert!(fmts[3].1.bold && fmts[3].1.italic);
        assert_eq!(fmts[3].1.size, 8.0);
        // `-` is the default font; an unreadable colour is ignored.
        assert_eq!(fmts[4].0, " FF0000e");
        assert_eq!(fmts[4].1.font, None);
        assert!(!fmts[4].1.bold);
        assert_eq!(fmts[4].1.color, None);
        assert_eq!(fmts[5].1.color, Some(Color(0xFF0000)));

        let runs = expand("&Ua&U&Eb&E&Sc&S&Xd&X&Ye", &en);
        let get = |i: usize| match &runs[i] {
            HfRun::Text { format, .. } => format.clone(),
            HfRun::Picture => unreachable!(),
        };
        assert_eq!(get(0).underline, Underline::Single);
        assert_eq!(get(1).underline, Underline::Double);
        assert!(get(2).strike && get(2).underline == Underline::None);
        assert_eq!(get(3).script, Script::Superscript);
        assert_eq!(get(4).script, Script::Subscript);

        // Theme colours: text 1 is black, accent 1 lightened by 40 %.
        let color = |code: &str| match &expand(code, &en)[0] {
            HfRun::Text { format, .. } => format.color,
            HfRun::Picture => None,
        };
        assert_eq!(color("&K01+000x"), Some(Color(0x000000)));
        assert_eq!(color("&K04+040x"), Some(Color(0x73A0B4)));
        assert_eq!(color("&K00-050x"), Some(Color(0x808080)));
    }

    #[test]
    fn lines_and_plain_text() {
        let en = ctx(Locale::en());
        let l = lines(expand("one\ntwo&B bold\n\nfour", &en));
        assert_eq!(l.len(), 4);
        assert_eq!(text(&l[1]), "two bold");
        assert!(l[2].is_empty());
        assert_eq!(
            plain_text("&L&A&CConfidential&RPage &P", &en),
            "T&L, Confidential, Page 3"
        );
        assert_eq!(
            plain_text("&CPage &P de &N", &ctx(Locale::fr())),
            "Page 3 de 12"
        );
        assert_eq!(
            plain_text("&L&A&RPage &P", &ctx(Locale::fr())),
            "T&L; Page 3"
        );
    }

    #[test]
    fn dialog_spelling_round_trips() {
        let fr = Locale::fr();
        let code = "&\"Arial,Bold\"Page &P+1 de &N - &Z&F - &A &D &T &G";
        let shown = to_dialog_text(code, fr);
        assert_eq!(
            shown,
            "&\"Arial,Bold\"Page &[Page]+1 de &[Pages] - &[Chemin d'accès]&[Fichier] - &[Onglet] &[Date] &[Heure] &[Image]"
        );
        assert_eq!(from_dialog_text(&shown, fr), code);
        assert_eq!(
            from_dialog_text("&[Tab] &[onglet] &[Nope] &[", fr),
            "&A &A &[Nope] &["
        );
        assert_eq!(
            to_dialog_text("&P of &N", Locale::en()),
            "&[Page] of &[Pages]"
        );
    }

    #[test]
    fn built_in_presets() {
        let p = presets(Locale::en(), "Ana & Co");
        assert_eq!(p.len(), 16);
        assert_eq!(p[0], "");
        assert_eq!(p[2], "&CPage &P of &N");
        assert_eq!(p[15], "&CPrepared by Ana && Co &D&RPage &P");
        let en = ctx(Locale::en());
        assert_eq!(plain_text(&p[4], &en), "Confidential, 10/12/2026, Page 3");
        assert_eq!(plain_text(&p[14], &en), "Ana & Co, Page 3, 10/12/2026");
        let fr = presets(Locale::fr(), "Ana");
        assert_eq!(fr[2], "&CPage &P de &N");
        assert_eq!(fr[15], "&CPréparé par Ana &D&RPage &P");
    }

    #[test]
    fn variants_and_slots() {
        let mut hf = HeaderFooter::default();
        assert!(hf.is_empty());
        hf.odd_header = "&Codd".into();
        hf.even_header = "&Ceven".into();
        hf.first_header = "&Cfirst".into();
        assert_eq!(hf.header(hf.variant(true, 1)), "&Codd");
        assert_eq!(hf.header(hf.variant(false, 2)), "&Codd");
        hf.different_first = true;
        hf.different_odd_even = true;
        assert_eq!(hf.header(hf.variant(true, 1)), "&Cfirst");
        assert_eq!(hf.header(hf.variant(false, 2)), "&Ceven");
        assert_eq!(hf.header(hf.variant(false, 3)), "&Codd");
        assert!(!hf.is_empty());

        let slot = HfSlot {
            variant: HfVariant::Even,
            footer: true,
            section: HfSection::Right,
        };
        assert_eq!(slot.vml_id(), "RFEVEN");
        assert_eq!(HfSlot::from_vml_id("RFEVEN"), Some(slot));
        assert_eq!(
            HfSlot::from_vml_id("LH").map(|s| (s.variant, s.footer)),
            Some((HfVariant::Odd, false))
        );
        assert_eq!(HfSlot::from_vml_id("XH"), None);
    }
}
