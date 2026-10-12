//! Fonts for printing: finding the TrueType fonts installed on the
//! machine, choosing the one a cell's font name stands for, measuring
//! text with it, and falling back to the standard PDF fonts (which every
//! PDF reader has, and whose metrics are built in) when no file can be
//! used.

use crate::std14::Standard;
use crate::ttf::{self, TtfFont};
use opensheet_engine::page::TextMeasure;
use opensheet_engine::style::Font;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// One font found on disk.
#[derive(Clone, Debug)]
struct FaceInfo {
    path: PathBuf,
    /// Offset of the font in its file (non-zero inside a collection).
    offset: u32,
    family: String,
    typographic_family: String,
    full: String,
    style: ttf::StyleInfo,
}

enum FaceKind {
    TrueType(Arc<TtfFont>),
    Standard(Standard),
}

/// A font ready to measure and draw text: a TrueType font to embed, or
/// one of the standard PDF fonts.
pub struct Face {
    kind: FaceKind,
    family: String,
    bold: bool,
    italic: bool,
    /// The file has no bold (or italic) of this family: the regular
    /// outlines are thickened (or slanted) when drawing.
    synthetic_bold: bool,
    synthetic_italic: bool,
    /// Identifies the face within its `FontDb`.
    id: usize,
}

/// A character as a face draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Glyph {
    /// Glyph index in an embedded TrueType font.
    Index(u16),
    /// Character code of a standard font (Windows Latin 1).
    Code(u8),
}

/// Spaces fonts often lack: they print as the plain space (the narrow
/// no-break space is the thousands separator of French numbers).
fn is_space_like(ch: char) -> bool {
    matches!(ch, '\t' | '\u{a0}' | '\u{2000}'..='\u{200a}' | '\u{202f}' | '\u{205f}' | '\u{3000}')
}

impl Face {
    /// The family of the font actually used, which may be a substitute
    /// for the one asked for ("Helvetica Neue" for "Segoe UI").
    pub fn family(&self) -> &str {
        &self.family
    }

    pub fn bold(&self) -> bool {
        self.bold
    }

    pub fn italic(&self) -> bool {
        self.italic
    }

    /// False for the standard PDF fonts, which are not embedded.
    pub fn is_embedded(&self) -> bool {
        matches!(self.kind, FaceKind::TrueType(_))
    }

    pub(crate) fn id(&self) -> usize {
        self.id
    }

    pub(crate) fn synthetic(&self) -> (bool, bool) {
        (self.synthetic_bold, self.synthetic_italic)
    }

    pub(crate) fn ttf(&self) -> Option<&Arc<TtfFont>> {
        match &self.kind {
            FaceKind::TrueType(f) => Some(f),
            FaceKind::Standard(_) => None,
        }
    }

    pub(crate) fn standard(&self) -> Option<Standard> {
        match &self.kind {
            FaceKind::Standard(s) => Some(*s),
            FaceKind::TrueType(_) => None,
        }
    }

    /// How the face draws a character; `None` when it cannot.
    pub(crate) fn glyph(&self, ch: char) -> Option<Glyph> {
        match &self.kind {
            FaceKind::TrueType(f) => f
                .glyph(ch)
                .or_else(|| is_space_like(ch).then(|| f.glyph(' ')).flatten())
                .map(Glyph::Index),
            FaceKind::Standard(_) => {
                crate::std14::win_ansi(ch).or_else(|| is_space_like(ch).then_some(b' ')).map(Glyph::Code)
            }
        }
    }

    pub fn has_glyph(&self, ch: char) -> bool {
        self.glyph(ch).is_some()
    }

    /// Advance width of a glyph, in thousandths of the font size.
    pub(crate) fn glyph_advance(&self, glyph: Glyph) -> f64 {
        match (&self.kind, glyph) {
            (FaceKind::TrueType(f), Glyph::Index(g)) => f.advance(g) as f64 * 1000.0 / f.units_per_em as f64,
            (FaceKind::Standard(s), Glyph::Code(c)) => s.width(c) as f64,
            _ => 0.0,
        }
    }

    /// The glyph drawn for a character the face lacks: the font's "not
    /// defined" box, or a question mark.
    pub(crate) fn missing_glyph(&self) -> Glyph {
        match &self.kind {
            FaceKind::TrueType(_) => Glyph::Index(0),
            FaceKind::Standard(_) => Glyph::Code(b'?'),
        }
    }

    /// Width of `text` on one line at `size`, as it is drawn with this
    /// face alone. Characters without a glyph count as the missing glyph.
    pub fn text_width(&self, text: &str, size: f64) -> f64 {
        let units: f64 = text
            .chars()
            .filter(|c| !c.is_control() || *c == '\t')
            .map(|c| self.glyph_advance(self.glyph(c).unwrap_or(self.missing_glyph())))
            .sum();
        units * size / 1000.0
    }

    fn em(&self, ttf: impl Fn(&TtfFont) -> i16, standard: impl Fn(Standard) -> i16) -> f64 {
        match &self.kind {
            FaceKind::TrueType(f) => ttf(f) as f64 / f.units_per_em as f64,
            FaceKind::Standard(s) => standard(*s) as f64 / 1000.0,
        }
    }

    /// Height above the baseline, as a fraction of the font size.
    pub fn ascent(&self) -> f64 {
        self.em(|f| f.ascent, |s| s.metrics().ascent)
    }

    /// Depth below the baseline, as a (positive) fraction of the size.
    pub fn descent(&self) -> f64 {
        -self.em(|f| f.descent, |s| s.metrics().descent)
    }

    /// Distance from the baseline down to the middle of the underline.
    pub fn underline_offset(&self) -> f64 {
        -self.em(|f| f.underline_position, |_| -100)
    }

    pub fn underline_thickness(&self) -> f64 {
        self.em(|f| f.underline_thickness, |_| 50)
    }

    /// Distance from the baseline up to the middle of the strikethrough.
    pub fn strikeout_offset(&self) -> f64 {
        self.em(|f| f.strikeout_position, |_| 260)
    }

    pub fn strikeout_thickness(&self) -> f64 {
        self.em(|f| f.strikeout_thickness, |_| 50)
    }
}

/// The font files of the machine, and the faces loaded from them.
pub struct FontDb {
    infos: Vec<FaceInfo>,
    /// Indices into `infos` by lowercase family name (names 1 and 16) and
    /// full name.
    by_name: HashMap<String, Vec<usize>>,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    /// File contents, shared by the members of a collection.
    files: HashMap<PathBuf, Option<Arc<[u8]>>>,
    /// Parsed fonts by index into `infos`; `None` when unusable.
    fonts: HashMap<usize, Option<Arc<TtfFont>>>,
    /// Resolved faces by (lowercase family, bold, italic).
    faces: HashMap<(String, bool, bool), Arc<Face>>,
    /// The face that draws a character other fonts lack.
    fallbacks: HashMap<char, Option<Arc<Face>>>,
    next_id: usize,
}

/// Where fonts are installed on this system.
fn system_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let mut dirs: Vec<PathBuf> = Vec::new();
    if cfg!(target_os = "macos") {
        dirs.extend(["/System/Library/Fonts", "/Library/Fonts"].map(PathBuf::from));
        dirs.extend(home.iter().map(|h| h.join("Library/Fonts")));
    } else if cfg!(target_os = "windows") {
        let windows = std::env::var_os("WINDIR").map_or_else(|| PathBuf::from("C:\\Windows"), PathBuf::from);
        dirs.push(windows.join("Fonts"));
        dirs.extend(std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Microsoft\\Windows\\Fonts")));
    } else {
        dirs.extend(["/usr/share/fonts", "/usr/local/share/fonts"].map(PathBuf::from));
        dirs.extend(home.iter().flat_map(|h| [h.join(".fonts"), h.join(".local/share/fonts")]));
    }
    dirs
}

/// Fonts tried, in order, when a family is not installed. The first lists
/// are the Office fonts, with the stand-ins the application shows them
/// with on screen; the last three are the generic sans-serif, serif and
/// monospaced choices.
fn substitutes(lower: &str) -> &'static [&'static str] {
    match lower {
        "aptos narrow" => &["Aptos", "Calibri", "Carlito", "Seravek"],
        "aptos" | "aptos display" => &["Calibri", "Carlito", "Seravek"],
        "calibri" | "calibri light" => &["Carlito", "Seravek"],
        "cambria" => &["Georgia", "Times New Roman"],
        "segoe ui" => &["Helvetica Neue", "Arial"],
        "consolas" => &["Menlo", "Courier New"],
        "helvetica" => &["Helvetica Neue", "Arial", "Liberation Sans"],
        "arial" => &["Helvetica Neue", "Helvetica", "Liberation Sans"],
        "times new roman" => &["Times", "Liberation Serif"],
        "courier new" => &["Courier", "Liberation Mono"],
        _ => &[],
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Generic {
    Sans,
    Serif,
    Mono,
}

fn generic(lower: &str) -> Generic {
    let has = |words: &[&str]| words.iter().any(|w| lower.contains(w));
    if has(&["courier", "consolas", "mono", "menlo", "typewriter", "fixed"]) {
        Generic::Mono
    } else if has(&["times", "cambria", "georgia", "garamond", "serif", "roman", "book", "palatino", "century"])
        && !lower.contains("sans")
    {
        Generic::Serif
    } else {
        Generic::Sans
    }
}

fn generic_families(kind: Generic) -> &'static [&'static str] {
    match kind {
        Generic::Sans => &["Helvetica Neue", "Arial", "Helvetica", "Liberation Sans", "DejaVu Sans", "Noto Sans"],
        Generic::Serif => &["Times New Roman", "Times", "Georgia", "Liberation Serif", "DejaVu Serif"],
        Generic::Mono => &["Menlo", "Courier New", "Courier", "Liberation Mono", "DejaVu Sans Mono"],
    }
}

/// Fonts with wide character coverage, tried for characters the cell's
/// font lacks.
const FALLBACK_FAMILIES: [&str; 14] = [
    "Helvetica Neue",
    "Arial",
    "Arial Unicode MS",
    "Apple Symbols",
    "Menlo",
    "Hiragino Sans GB",
    "PingFang SC",
    "Apple SD Gothic Neo",
    "Geeza Pro",
    "Segoe UI",
    "Segoe UI Symbol",
    "DejaVu Sans",
    "Noto Sans",
    "Noto Sans CJK SC",
];

/// Read the tables of a font file needed to list it, without loading the
/// whole file (collections of Asian fonts run to tens of megabytes).
fn scan_file(path: &Path) -> Vec<FaceInfo> {
    let mut out = Vec::new();
    let Ok(mut file) = File::open(path) else {
        return out;
    };
    let mut read_at = |offset: u64, len: usize| -> Option<Vec<u8>> {
        // No table needed here is larger than this.
        if len > 4 << 20 {
            return None;
        }
        let mut buf = vec![0u8; len];
        file.seek(SeekFrom::Start(offset)).ok()?;
        file.read_exact(&mut buf).ok()?;
        Some(buf)
    };
    let Some(start) = read_at(0, 12) else {
        return out;
    };
    let Some(header) = read_at(0, ttf::collection_header_len(&start)) else {
        return out;
    };
    for offset in ttf::font_offsets(&header) {
        let Some(dir_head) = read_at(offset as u64, 12) else {
            continue;
        };
        let version = u32::from_be_bytes([dir_head[0], dir_head[1], dir_head[2], dir_head[3]]);
        if version != 0x0001_0000 && version != u32::from_be_bytes(*b"true") {
            continue;
        }
        let count = u16::from_be_bytes([dir_head[4], dir_head[5]]) as usize;
        let Some(dir) = read_at(offset as u64, 12 + 16 * count) else {
            continue;
        };
        let records = ttf::table_records(&dir);
        let mut table = |tag: &[u8; 4], max: u32| -> Option<Vec<u8>> {
            let t = records.iter().find(|t| &t.tag == tag)?;
            read_at(t.offset as u64, t.length.min(max) as usize)
        };
        if !records.iter().any(|t| &t.tag == b"glyf") {
            continue;
        }
        let (Some(name), Some(head)) = (table(b"name", u32::MAX), table(b"head", 54)) else {
            continue;
        };
        let os2 = table(b"OS/2", 96);
        let names = ttf::parse_names(&name);
        if names.family.is_empty() {
            continue;
        }
        out.push(FaceInfo {
            path: path.to_path_buf(),
            offset,
            family: names.family,
            typographic_family: names.typographic_family,
            full: names.full,
            style: ttf::parse_style(&head, os2.as_deref()),
        });
    }
    out
}

fn scan_dir(dir: &Path, depth: u32, out: &mut Vec<FaceInfo>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    // A stable order, so the same font wins every time two files hold the
    // same face.
    paths.sort();
    for path in paths {
        if path.is_dir() {
            if depth < 4 {
                scan_dir(&path, depth + 1, out);
            }
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if ext == "ttf" || ext == "ttc" {
            out.extend(scan_file(&path));
        }
    }
}

impl FontDb {
    /// The fonts installed on this machine. They are listed once, the
    /// first time this is called.
    pub fn system() -> &'static FontDb {
        static SYSTEM: OnceLock<FontDb> = OnceLock::new();
        SYSTEM.get_or_init(|| FontDb::from_dirs(&system_dirs()))
    }

    /// The `.ttf` and `.ttc` fonts of some directories (searched with
    /// their subdirectories), for fonts shipped with the application.
    pub fn from_dirs(dirs: &[PathBuf]) -> FontDb {
        let mut infos = Vec::new();
        for dir in dirs {
            scan_dir(dir, 0, &mut infos);
        }
        let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, info) in infos.iter().enumerate() {
            let mut keys = vec![info.family.to_lowercase()];
            for extra in [&info.typographic_family, &info.full] {
                let key = extra.to_lowercase();
                if !key.is_empty() && !keys.contains(&key) {
                    keys.push(key);
                }
            }
            for key in keys {
                by_name.entry(key).or_default().push(i);
            }
        }
        FontDb {
            infos,
            by_name,
            state: Mutex::new(State::default()),
        }
    }

    /// No font files at all: everything prints with the standard PDF
    /// fonts. The output then does not depend on the machine.
    pub fn standard_only() -> FontDb {
        FontDb::from_dirs(&[])
    }

    /// The names of the installed families, sorted.
    pub fn families(&self) -> Vec<String> {
        let mut names: Vec<String> = self.infos.iter().map(|i| i.family.clone()).collect();
        names.sort();
        names.dedup();
        names
    }

    /// The font file that best renders a family in a style, if the
    /// family is installed: index into `infos`.
    fn best_match(&self, lower_family: &str, bold: bool, italic: bool) -> Option<usize> {
        let target = if bold { 700i32 } else { 400 };
        self.by_name.get(lower_family)?.iter().copied().min_by_key(|i| {
            let info = &self.infos[*i];
            let s = &info.style;
            // An exact family name beats a face that only belongs to the
            // wider typographic family (a "Light" or "Condensed" cut).
            let named = if info.family.to_lowercase() == lower_family { 0 } else { 2000 };
            let slant = if s.italic == italic { 0 } else { 10_000 };
            let width = (s.width as i32 - 5).abs() * 1000;
            named + slant + width + (s.weight as i32 - target).abs()
        })
    }

    fn load(&self, state: &mut State, index: usize) -> Option<Arc<TtfFont>> {
        if let Some(font) = state.fonts.get(&index) {
            return font.clone();
        }
        let info = &self.infos[index];
        let data =
            state.files.entry(info.path.clone()).or_insert_with(|| std::fs::read(&info.path).ok().map(Arc::from)).clone();
        // A font whose licence forbids embedding (and nothing else) is
        // not used.
        let font = data
            .and_then(|d| TtfFont::parse(d, info.offset))
            .filter(|f| f.style.fs_type & 0x000F != 0x0002)
            .filter(|f| f.glyph(' ').is_some() || f.glyph('a').is_some())
            .map(Arc::new);
        state.fonts.insert(index, font.clone());
        font
    }

    fn truetype_face(&self, state: &mut State, lower: &str, bold: bool, italic: bool) -> Option<Face> {
        let index = self.best_match(lower, bold, italic)?;
        let font = self.load(state, index)?;
        let info = &self.infos[index];
        Some(Face {
            family: info.family.clone(),
            bold,
            italic,
            synthetic_bold: bold && info.style.weight < 600,
            synthetic_italic: italic && !info.style.italic,
            kind: FaceKind::TrueType(font),
            id: 0,
        })
    }

    /// The face that prints a font family in a style: the installed
    /// font of that name, else a substitute, else a standard PDF font.
    pub fn face(&self, family: &str, bold: bool, italic: bool) -> Arc<Face> {
        let lower = family.trim().to_lowercase();
        let mut state = self.state.lock().unwrap();
        let key = (lower.clone(), bold, italic);
        if let Some(face) = state.faces.get(&key) {
            return face.clone();
        }
        let kind = generic(&lower);
        let mut face = [lower.as_str()]
            .into_iter()
            .chain(substitutes(&lower).iter().copied())
            .chain(generic_families(kind).iter().copied())
            .find_map(|name| self.truetype_face(&mut state, &name.to_lowercase(), bold, italic))
            .unwrap_or_else(|| {
                let standard = Standard::pick(kind == Generic::Serif, kind == Generic::Mono, bold, italic);
                Face {
                    family: standard.family().to_string(),
                    bold,
                    italic,
                    synthetic_bold: false,
                    synthetic_italic: false,
                    kind: FaceKind::Standard(standard),
                    id: 0,
                }
            });
        face.id = state.next_id;
        state.next_id += 1;
        let face = Arc::new(face);
        state.faces.insert(key, face.clone());
        face
    }

    /// The face that prints a cell font.
    pub fn face_for(&self, font: &Font) -> Arc<Face> {
        self.face(&font.name, font.bold, font.italic)
    }

    /// A face that has a glyph for `ch`, for text whose own font lacks it.
    fn fallback(&self, ch: char) -> Option<Arc<Face>> {
        if let Some(found) = self.state.lock().unwrap().fallbacks.get(&ch) {
            return found.clone();
        }
        let found = FALLBACK_FAMILIES.iter().find_map(|family| {
            // Only a font really installed under that name: `face` would
            // answer with a substitute.
            self.by_name.contains_key(&family.to_lowercase()).then(|| self.face(family, false, false)).filter(|f| f.has_glyph(ch))
        });
        self.state.lock().unwrap().fallbacks.insert(ch, found.clone());
        found
    }

    /// Split text into runs that one face each can draw: `face` wherever
    /// it has the glyphs, other installed fonts for the characters it
    /// lacks. Control characters are dropped.
    pub fn runs(&self, face: &Arc<Face>, text: &str) -> Vec<(Arc<Face>, String)> {
        let mut out: Vec<(Arc<Face>, String)> = Vec::new();
        for ch in text.chars() {
            if ch.is_control() && ch != '\t' {
                continue;
            }
            let chosen = if face.has_glyph(ch) { face.clone() } else { self.fallback(ch).unwrap_or_else(|| face.clone()) };
            match out.last_mut() {
                Some((last, run)) if Arc::ptr_eq(last, &chosen) => run.push(ch),
                _ => out.push((chosen, ch.to_string())),
            }
        }
        out
    }

    /// Width of one character at `size`, as it is drawn within text
    /// set in `face`.
    pub fn char_width(&self, face: &Arc<Face>, ch: char, size: f64) -> f64 {
        match face.glyph(ch) {
            Some(glyph) => face.glyph_advance(glyph) * size / 1000.0,
            None => self.text_width(face, ch.encode_utf8(&mut [0; 4]), size),
        }
    }

    /// Width of `text` on one line at `size`, exactly as [`FontDb::runs`]
    /// draws it.
    pub fn text_width(&self, face: &Arc<Face>, text: &str, size: f64) -> f64 {
        // Nearly all text is drawn by its own font: skip building runs.
        if text.chars().all(|c| face.has_glyph(c)) {
            return face.text_width(text, size);
        }
        self.runs(face, text).iter().map(|(f, run)| f.text_width(run, size)).sum()
    }
}

/// Text measured the way it prints: pass a `FontDb` to
/// `page::paginate` so that text overflowing its cell is paginated the
/// way the printer draws it.
impl TextMeasure for FontDb {
    fn text_width(&self, text: &str, font: &Font) -> f32 {
        // Pixels at 100 %: 96 to the inch, the font size being in points.
        let size = font.size_pt() as f64 * 96.0 / 72.0;
        FontDb::text_width(self, &self.face_for(font), text, size) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_fonts_when_nothing_is_installed() {
        let db = FontDb::standard_only();
        assert!(db.families().is_empty());
        let face = db.face("Aptos Narrow", false, false);
        assert!(!face.is_embedded());
        assert_eq!(face.family(), "Helvetica");
        assert_eq!(db.face("Cambria", true, false).standard(), Some(Standard::TimesBold));
        assert_eq!(db.face("Consolas", false, true).standard(), Some(Standard::CourierOblique));
        // The same request gives the same face.
        assert!(Arc::ptr_eq(&face, &db.face("aptos narrow ", false, false)));
        // Helvetica: "Hello" is 722 + 556 + 222 + 222 + 556 thousandths.
        assert!((face.text_width("Hello", 10.0) - 22.78).abs() < 1e-9);
        // The narrow no-break space of French numbers is a space; what
        // the font cannot draw at all is a question mark.
        assert_eq!(face.glyph('\u{202f}'), Some(Glyph::Code(b' ')));
        assert_eq!(face.glyph('é'), Some(Glyph::Code(0xE9)));
        assert_eq!(face.glyph('€'), Some(Glyph::Code(0x80)));
        assert_eq!(face.glyph('汉'), None);
        assert!((face.text_width("汉", 10.0) - 5.56).abs() < 1e-9);
        assert_eq!(db.runs(&face, "a\u{1}汉").len(), 1);
        assert!((face.ascent() - 0.718).abs() < 1e-9 && (face.descent() - 0.207).abs() < 1e-9);
        // Measuring for pagination: 11 pt is 14.67 px.
        let font = Font::default();
        let px = TextMeasure::text_width(&db, "Hello", &font);
        assert!((px - 2.278 * 11.0 * 96.0 / 72.0).abs() < 1e-3);
    }

    #[test]
    fn generic_classes() {
        assert!(generic("courier new") == Generic::Mono);
        assert!(generic("times new roman") == Generic::Serif);
        assert!(generic("ms sans serif") == Generic::Sans);
        assert!(generic("aptos narrow") == Generic::Sans);
    }

    /// The fonts of this machine, whatever they are: every face found
    /// measures and subsets.
    #[test]
    fn system_fonts_load() {
        let db = FontDb::system();
        let face = db.face("Aptos Narrow", false, false);
        let width = face.text_width("Hello, world", 11.0);
        assert!(width > 20.0 && width < 120.0, "{width}");
        assert!(face.ascent() > 0.5 && face.descent() > 0.05);
        let bold = db.face("Aptos Narrow", true, false);
        assert!(bold.bold());
        if let Some(ttf) = face.ttf() {
            let glyphs = "Hello".chars().filter_map(|c| ttf.glyph(c)).collect();
            let subset = ttf.subset(&glyphs);
            assert!(TtfFont::parse(subset.into(), 0).is_some());
        }
    }
}
