//! Reading TrueType fonts (`.ttf`, and the members of `.ttc` collections):
//! what is needed to measure text, to map characters to glyphs and to
//! embed a subset of the glyphs in a PDF.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

fn u16_at(data: &[u8], offset: usize) -> Option<u16> {
    let b = data.get(offset..offset + 2)?;
    Some(u16::from_be_bytes([b[0], b[1]]))
}

fn i16_at(data: &[u8], offset: usize) -> Option<i16> {
    u16_at(data, offset).map(|v| v as i16)
}

fn u32_at(data: &[u8], offset: usize) -> Option<u32> {
    let b = data.get(offset..offset + 4)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// The offsets of the fonts in a file: one for a plain font, one per
/// member for a collection.
pub fn font_offsets(header: &[u8]) -> Vec<u32> {
    if header.get(..4) == Some(b"ttcf") {
        let count = u32_at(header, 8).unwrap_or(0).min(1024);
        (0..count).filter_map(|i| u32_at(header, 12 + 4 * i as usize)).collect()
    } else {
        vec![0]
    }
}

/// Number of bytes of a file's start that hold the collection header.
pub fn collection_header_len(first_bytes: &[u8]) -> usize {
    if first_bytes.get(..4) == Some(b"ttcf") {
        12 + 4 * u32_at(first_bytes, 8).unwrap_or(0).min(1024) as usize
    } else {
        12
    }
}

/// One entry of a font's table directory.
#[derive(Clone, Copy, Debug)]
pub struct TableRecord {
    pub tag: [u8; 4],
    pub offset: u32,
    pub length: u32,
}

/// Parse a table directory. `dir` starts at the font's offset table.
pub fn table_records(dir: &[u8]) -> Vec<TableRecord> {
    let count = u16_at(dir, 4).unwrap_or(0) as usize;
    (0..count)
        .filter_map(|i| {
            let at = 12 + 16 * i;
            let tag = dir.get(at..at + 4)?;
            Some(TableRecord {
                tag: [tag[0], tag[1], tag[2], tag[3]],
                offset: u32_at(dir, at + 8)?,
                length: u32_at(dir, at + 12)?,
            })
        })
        .collect()
}

/// The names of a font, from its `name` table.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Names {
    /// Name 1: the family in the four-style model ("Arial Narrow").
    pub family: String,
    /// Name 2: "Regular", "Bold", "Italic", "Bold Italic".
    pub subfamily: String,
    /// Name 16: the family all weights and widths belong to.
    pub typographic_family: String,
    /// Name 4.
    pub full: String,
    /// Name 6.
    pub postscript: String,
}

/// Mac Roman, upper half, as far as font names use it.
fn mac_roman(byte: u8) -> char {
    const HIGH: &str = "ÄÅÇÉÑÖÜáàâäãåçéèêëíìîïñóòôöõúùûü†°¢£§•¶ß®©™´¨≠ÆØ∞±≤≥¥µ∂∑∏π∫ªºΩæø¿¡¬√ƒ≈∆«»… ÀÃÕŒœ–—“”‘’÷◊ÿŸ⁄€‹›ﬁﬂ‡·‚„‰ÂÊÁËÈÍÎÏÌÓÔ";
    if byte < 0x80 {
        byte as char
    } else {
        HIGH.chars().nth((byte - 0x80) as usize).unwrap_or('?')
    }
}

/// Read the `name` table.
pub fn parse_names(table: &[u8]) -> Names {
    let mut names = Names::default();
    // The best record seen for each name: Windows English first, then any
    // Windows or Unicode record, then Macintosh.
    let mut rank = [0u8; 5];
    let count = u16_at(table, 2).unwrap_or(0) as usize;
    let strings = u16_at(table, 4).unwrap_or(0) as usize;
    for i in 0..count {
        let at = 6 + 12 * i;
        let (Some(platform), Some(language), Some(id), Some(length), Some(offset)) = (
            u16_at(table, at),
            u16_at(table, at + 4),
            u16_at(table, at + 6),
            u16_at(table, at + 8),
            u16_at(table, at + 10),
        ) else {
            break;
        };
        let slot = match id {
            1 => 0,
            2 => 1,
            16 => 2,
            4 => 3,
            6 => 4,
            _ => continue,
        };
        let quality = match (platform, language) {
            (3, 0x409) => 4,
            (3, _) | (0, _) => 3,
            (1, 0) => 2,
            (1, _) => 1,
            _ => continue,
        };
        if quality <= rank[slot] {
            continue;
        }
        let Some(bytes) = table.get(strings + offset as usize..strings + offset as usize + length as usize) else {
            continue;
        };
        let text: String = if platform == 1 {
            bytes.iter().map(|b| mac_roman(*b)).collect()
        } else {
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        };
        if text.is_empty() {
            continue;
        }
        rank[slot] = quality;
        match slot {
            0 => names.family = text,
            1 => names.subfamily = text,
            2 => names.typographic_family = text,
            3 => names.full = text,
            _ => names.postscript = text,
        }
    }
    names
}

/// The style of a font, from its `head` and `OS/2` tables.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StyleInfo {
    pub bold: bool,
    pub italic: bool,
    /// 100 to 900; 400 is regular, 700 bold.
    pub weight: u16,
    /// 1 to 9; 5 is normal, lower is condensed.
    pub width: u16,
    /// The embedding licence bits (`fsType`).
    pub fs_type: u16,
}

pub fn parse_style(head: &[u8], os2: Option<&[u8]>) -> StyleInfo {
    let mac_style = u16_at(head, 44).unwrap_or(0);
    let mut style = StyleInfo {
        bold: mac_style & 1 != 0,
        italic: mac_style & 2 != 0,
        weight: if mac_style & 1 != 0 { 700 } else { 400 },
        width: 5,
        fs_type: 0,
    };
    if let Some(os2) = os2 {
        if let Some(weight) = u16_at(os2, 4).filter(|w| (1..=1000).contains(w)) {
            style.weight = weight;
        }
        if let Some(width) = u16_at(os2, 6).filter(|w| (1..=9).contains(w)) {
            style.width = width;
        }
        style.fs_type = u16_at(os2, 8).unwrap_or(0);
        let selection = u16_at(os2, 62).unwrap_or(0);
        style.bold |= selection & 0x20 != 0;
        style.italic |= selection & 0x01 != 0;
    }
    style
}

/// One step of the outline of a glyph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OutlineStep {
    Move(f32, f32),
    Line(f32, f32),
    /// A quadratic curve: its control point, then its end.
    Quad(f32, f32, f32, f32),
    Close,
}

/// A TrueType font loaded in memory.
pub struct TtfFont {
    /// The whole file (a collection's members share it).
    data: Arc<[u8]>,
    tables: Vec<TableRecord>,
    pub names: Names,
    pub style: StyleInfo,
    pub units_per_em: u16,
    /// Distances from the baseline in font units; `descent` is negative.
    pub ascent: i16,
    pub descent: i16,
    pub cap_height: i16,
    /// xMin, yMin, xMax, yMax of all glyphs.
    pub bbox: [i16; 4],
    pub italic_angle: f32,
    pub fixed_pitch: bool,
    pub underline_position: i16,
    pub underline_thickness: i16,
    pub strikeout_position: i16,
    pub strikeout_thickness: i16,
    pub num_glyphs: u16,
    long_loca: bool,
    /// Advance width of every glyph, in font units.
    advances: Vec<u16>,
    cmap: HashMap<u32, u16>,
}

impl TtfFont {
    /// Parse the font at `offset` of `data`. `None` when the data is not
    /// a font with TrueType outlines (OpenType fonts with PostScript
    /// outlines cannot be embedded as TrueType) or lacks a table.
    pub fn parse(data: Arc<[u8]>, offset: u32) -> Option<TtfFont> {
        let dir = data.get(offset as usize..)?;
        let version = u32_at(dir, 0)?;
        if version != 0x0001_0000 && version != u32::from_be_bytes(*b"true") {
            return None;
        }
        let tables = table_records(dir);
        let find = |tag: &[u8; 4]| -> Option<&[u8]> {
            let t = tables.iter().find(|t| &t.tag == tag)?;
            data.get(t.offset as usize..t.offset as usize + t.length as usize)
        };
        let head = find(b"head")?;
        let hhea = find(b"hhea")?;
        let maxp = find(b"maxp")?;
        let hmtx = find(b"hmtx")?;
        find(b"glyf")?;
        find(b"loca")?;
        let os2 = find(b"OS/2");

        let units_per_em = u16_at(head, 18).filter(|u| *u > 0)?;
        let num_glyphs = u16_at(maxp, 4)?;
        let num_h_metrics = (u16_at(hhea, 34)? as usize).min(num_glyphs as usize);
        let mut advances = Vec::with_capacity(num_glyphs as usize);
        let mut last = 0;
        for gid in 0..num_glyphs as usize {
            if gid < num_h_metrics {
                last = u16_at(hmtx, 4 * gid).unwrap_or(last);
            }
            advances.push(last);
        }

        let mut ascent = i16_at(hhea, 4)?;
        let mut descent = i16_at(hhea, 6)?;
        let mut cap_height = 0;
        let mut strikeout_position = 0;
        let mut strikeout_thickness = 0;
        if let Some(os2) = os2 {
            if ascent == 0 && descent == 0 {
                ascent = i16_at(os2, 68).unwrap_or(0);
                descent = i16_at(os2, 70).unwrap_or(0);
            }
            strikeout_thickness = i16_at(os2, 26).unwrap_or(0);
            strikeout_position = i16_at(os2, 28).unwrap_or(0);
            if u16_at(os2, 0).unwrap_or(0) >= 2 {
                cap_height = i16_at(os2, 88).unwrap_or(0);
            }
        }
        let em = units_per_em as f32;
        if cap_height <= 0 {
            cap_height = (ascent as f32 * 0.75) as i16;
        }
        if strikeout_thickness <= 0 {
            strikeout_thickness = (em * 0.05) as i16;
            strikeout_position = (em * 0.26) as i16;
        }
        let (mut italic_angle, mut fixed_pitch) = (0.0, false);
        let (mut underline_position, mut underline_thickness) = (0, 0);
        if let Some(post) = find(b"post") {
            italic_angle = u32_at(post, 4).map_or(0.0, |v| v as i32 as f32 / 65536.0);
            underline_position = i16_at(post, 8).unwrap_or(0);
            underline_thickness = i16_at(post, 10).unwrap_or(0);
            fixed_pitch = u32_at(post, 12).unwrap_or(0) != 0;
        }
        if underline_thickness <= 0 {
            underline_position = -(em * 0.1) as i16;
            underline_thickness = (em * 0.05) as i16;
        }

        Some(TtfFont {
            names: find(b"name").map(parse_names).unwrap_or_default(),
            style: parse_style(head, os2),
            units_per_em,
            ascent,
            descent,
            cap_height,
            bbox: [
                i16_at(head, 36).unwrap_or(0),
                i16_at(head, 38).unwrap_or(0),
                i16_at(head, 40).unwrap_or(0),
                i16_at(head, 42).unwrap_or(0),
            ],
            italic_angle,
            fixed_pitch,
            underline_position,
            underline_thickness,
            strikeout_position,
            strikeout_thickness,
            num_glyphs,
            long_loca: i16_at(head, 50).unwrap_or(0) != 0,
            advances,
            cmap: find(b"cmap").map(parse_cmap).unwrap_or_default(),
            tables,
            data,
        })
    }

    fn table(&self, tag: &[u8; 4]) -> Option<&[u8]> {
        let t = self.tables.iter().find(|t| &t.tag == tag)?;
        self.data.get(t.offset as usize..t.offset as usize + t.length as usize)
    }

    /// The glyph of a character; `None` when the font has none.
    pub fn glyph(&self, ch: char) -> Option<u16> {
        self.cmap.get(&(ch as u32)).copied()
    }

    /// Advance width of a glyph in font units.
    pub fn advance(&self, glyph: u16) -> u16 {
        self.advances.get(glyph as usize).copied().unwrap_or(0)
    }

    /// The bytes of a glyph's outline in the `glyf` table (empty for a
    /// glyph without outline, like the space).
    fn glyph_data(&self, glyph: u16) -> &[u8] {
        let (Some(loca), Some(glyf)) = (self.table(b"loca"), self.table(b"glyf")) else {
            return &[];
        };
        let at = |i: usize| -> Option<usize> {
            if self.long_loca {
                u32_at(loca, 4 * i).map(|v| v as usize)
            } else {
                u16_at(loca, 2 * i).map(|v| v as usize * 2)
            }
        };
        match (at(glyph as usize), at(glyph as usize + 1)) {
            (Some(start), Some(end)) if start < end => glyf.get(start..end).unwrap_or(&[]),
            _ => &[],
        }
    }

    /// The glyphs a composite glyph is built from.
    fn components(&self, glyph: u16) -> Vec<u16> {
        let data = self.glyph_data(glyph);
        let mut out = Vec::new();
        if i16_at(data, 0).unwrap_or(0) >= 0 {
            return out;
        }
        let mut at = 10;
        loop {
            let (Some(flags), Some(component)) = (u16_at(data, at), u16_at(data, at + 2)) else {
                break;
            };
            out.push(component);
            // Skip the arguments (words or bytes) and the transformation.
            at += 4 + if flags & 0x0001 != 0 { 4 } else { 2 };
            at += if flags & 0x0008 != 0 {
                2
            } else if flags & 0x0040 != 0 {
                4
            } else if flags & 0x0080 != 0 {
                8
            } else {
                0
            };
            if flags & 0x0020 == 0 {
                break;
            }
        }
        out
    }

    /// The outline of a glyph as lines and quadratic curves, in font
    /// units with y upwards; empty for a glyph without outline (a
    /// space). Whoever draws text as shapes rather than as text (warped
    /// letters) starts from this.
    pub fn outline(&self, glyph: u16) -> Vec<OutlineStep> {
        let mut out = Vec::new();
        self.add_outline(glyph, [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], 0, &mut out);
        out
    }

    /// Add the outline of a glyph seen through a transformation
    /// (`x' = a·x + c·y + e`, `y' = b·x + d·y + f`).
    fn add_outline(&self, glyph: u16, m: [f32; 6], depth: usize, out: &mut Vec<OutlineStep>) {
        let data = self.glyph_data(glyph);
        let Some(contours) = i16_at(data, 0) else {
            return;
        };
        if contours < 0 {
            // A glyph made of other glyphs, each moved and scaled.
            if depth > 8 {
                return;
            }
            let mut at = 10;
            loop {
                let (Some(flags), Some(component)) = (u16_at(data, at), u16_at(data, at + 2)) else {
                    break;
                };
                at += 4;
                let (dx, dy) = if flags & 0x0001 != 0 {
                    let pair = (i16_at(data, at).unwrap_or(0) as f32, i16_at(data, at + 2).unwrap_or(0) as f32);
                    at += 4;
                    pair
                } else {
                    let pair = (
                        data.get(at).map_or(0.0, |v| *v as i8 as f32),
                        data.get(at + 1).map_or(0.0, |v| *v as i8 as f32),
                    );
                    at += 2;
                    pair
                };
                // Offsets given as point numbers to match are not followed.
                let (dx, dy) = if flags & 0x0002 != 0 { (dx, dy) } else { (0.0, 0.0) };
                let fixed = |at: usize| i16_at(data, at).unwrap_or(0) as f32 / 16384.0;
                let (a, b, c, d) = if flags & 0x0008 != 0 {
                    let scale = fixed(at);
                    at += 2;
                    (scale, 0.0, 0.0, scale)
                } else if flags & 0x0040 != 0 {
                    let pair = (fixed(at), fixed(at + 2));
                    at += 4;
                    (pair.0, 0.0, 0.0, pair.1)
                } else if flags & 0x0080 != 0 {
                    let all = (fixed(at), fixed(at + 2), fixed(at + 4), fixed(at + 6));
                    at += 8;
                    all
                } else {
                    (1.0, 0.0, 0.0, 1.0)
                };
                let inner = [
                    m[0] * a + m[2] * b,
                    m[1] * a + m[3] * b,
                    m[0] * c + m[2] * d,
                    m[1] * c + m[3] * d,
                    m[0] * dx + m[2] * dy + m[4],
                    m[1] * dx + m[3] * dy + m[5],
                ];
                self.add_outline(component, inner, depth + 1, out);
                if flags & 0x0020 == 0 {
                    break;
                }
            }
            return;
        }
        let contours = contours as usize;
        let mut ends = Vec::with_capacity(contours);
        for i in 0..contours {
            let Some(end) = u16_at(data, 10 + 2 * i) else {
                return;
            };
            ends.push(end as usize);
        }
        let Some(count) = ends.last().map(|last| last + 1) else {
            return;
        };
        let Some(instructions) = u16_at(data, 10 + 2 * contours) else {
            return;
        };
        let mut at = 12 + 2 * contours + instructions as usize;
        // One flag per point; a flag may say it repeats.
        let mut flags = Vec::with_capacity(count);
        while flags.len() < count {
            let Some(&flag) = data.get(at) else {
                return;
            };
            at += 1;
            flags.push(flag);
            if flag & 0x08 != 0 {
                let Some(&repeat) = data.get(at) else {
                    return;
                };
                at += 1;
                for _ in 0..repeat {
                    flags.push(flag);
                }
            }
        }
        flags.truncate(count);
        // Coordinates are steps from the point before: a byte with its
        // sign in the flag, a word, or nothing for "the same".
        let mut read = |short: u8, same: u8| -> Option<Vec<f32>> {
            let mut value = 0i32;
            let mut values = Vec::with_capacity(count);
            for flag in &flags {
                if flag & short != 0 {
                    let step = *data.get(at)? as i32;
                    at += 1;
                    value += if flag & same != 0 { step } else { -step };
                } else if flag & same == 0 {
                    value += i16_at(data, at)? as i32;
                    at += 2;
                }
                values.push(value as f32);
            }
            Some(values)
        };
        let (Some(xs), Some(ys)) = (read(0x02, 0x10), read(0x04, 0x20)) else {
            return;
        };
        let point = |i: usize| (m[0] * xs[i] + m[2] * ys[i] + m[4], m[1] * xs[i] + m[3] * ys[i] + m[5]);
        let middle = |a: (f32, f32), b: (f32, f32)| ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
        let mut first = 0;
        for end in ends {
            if end < first || end >= count {
                break;
            }
            let on = |i: usize| flags[i] & 0x01 != 0;
            let size = end - first + 1;
            let nth = |k: usize| first + k % size;
            // Start on a point of the curve: the first one that is, or
            // half-way between two control points.
            let start_at = (0..size).find(|k| on(nth(*k)));
            let (start, begin) = match start_at {
                Some(k) => (point(nth(k)), k + 1),
                None => (middle(point(nth(size - 1)), point(nth(0))), 0),
            };
            out.push(OutlineStep::Move(start.0, start.1));
            let mut control: Option<(f32, f32)> = None;
            for k in begin..begin + size - usize::from(start_at.is_some()) {
                let (here, here_on) = (point(nth(k)), on(nth(k)));
                match (control, here_on) {
                    (None, true) => out.push(OutlineStep::Line(here.0, here.1)),
                    (None, false) => control = Some(here),
                    (Some(c), true) => {
                        out.push(OutlineStep::Quad(c.0, c.1, here.0, here.1));
                        control = None;
                    }
                    // Two control points in a row: the curve passes
                    // half-way between them.
                    (Some(c), false) => {
                        let between = middle(c, here);
                        out.push(OutlineStep::Quad(c.0, c.1, between.0, between.1));
                        control = Some(here);
                    }
                }
            }
            if let Some(c) = control {
                out.push(OutlineStep::Quad(c.0, c.1, start.0, start.1));
            }
            out.push(OutlineStep::Close);
            first = end + 1;
        }
    }

    /// A font file holding only the outlines of `glyphs` (and of what
    /// they are composed of). Glyph numbers do not change, so text
    /// written with the full font's glyph ids shows with the subset.
    pub fn subset(&self, glyphs: &BTreeSet<u16>) -> Vec<u8> {
        let mut keep: BTreeSet<u16> = BTreeSet::new();
        let mut pending: Vec<u16> = glyphs.iter().copied().chain([0]).collect();
        while let Some(g) = pending.pop() {
            if g < self.num_glyphs && keep.insert(g) {
                pending.extend(self.components(g));
            }
        }
        let mut glyf: Vec<u8> = Vec::new();
        let mut loca: Vec<u8> = Vec::with_capacity(4 * (self.num_glyphs as usize + 1));
        for g in 0..self.num_glyphs {
            loca.extend((glyf.len() as u32).to_be_bytes());
            if keep.contains(&g) {
                glyf.extend(self.glyph_data(g));
                while glyf.len() % 4 != 0 {
                    glyf.push(0);
                }
            }
        }
        loca.extend((glyf.len() as u32).to_be_bytes());

        let mut head = self.table(b"head").unwrap_or(&[]).to_vec();
        if head.len() >= 54 {
            // The checksum adjustment is recomputed below; offsets are long.
            head[8..12].fill(0);
            head[50..52].copy_from_slice(&1i16.to_be_bytes());
        }
        let mut tables: Vec<([u8; 4], Vec<u8>)> = vec![(*b"glyf", glyf), (*b"head", head), (*b"loca", loca)];
        // Metrics, and the hinting programs the outlines may call.
        for tag in [b"hhea", b"hmtx", b"maxp", b"cvt ", b"fpgm", b"prep"] {
            if let Some(t) = self.table(tag) {
                tables.push((*tag, t.to_vec()));
            }
        }
        tables.sort_by_key(|(tag, _)| *tag);
        build_font(&tables)
    }
}

fn checksum(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    for chunk in data.chunks(4) {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        sum = sum.wrapping_add(u32::from_be_bytes(word));
    }
    sum
}

/// Assemble a font file from its tables (sorted by tag).
fn build_font(tables: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let n = tables.len() as u16;
    let entry_selector = 15 - n.max(1).leading_zeros() as u16;
    let search_range = 16 * (1u16 << entry_selector);
    let mut out: Vec<u8> = Vec::new();
    out.extend(0x0001_0000u32.to_be_bytes());
    out.extend(n.to_be_bytes());
    out.extend(search_range.to_be_bytes());
    out.extend(entry_selector.to_be_bytes());
    out.extend((n * 16 - search_range).to_be_bytes());
    let mut offset = 12 + 16 * tables.len();
    let mut head_at = None;
    for (tag, data) in tables {
        if tag == b"head" {
            head_at = Some(offset);
        }
        out.extend(tag);
        out.extend(checksum(data).to_be_bytes());
        out.extend((offset as u32).to_be_bytes());
        out.extend((data.len() as u32).to_be_bytes());
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in tables {
        out.extend(data);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    if let Some(at) = head_at.filter(|at| at + 12 <= out.len()) {
        let adjustment = 0xB1B0_AFBAu32.wrapping_sub(checksum(&out));
        out[at + 8..at + 12].copy_from_slice(&adjustment.to_be_bytes());
    }
    out
}

/// Read the character map: the Unicode subtable with the widest coverage,
/// or the map of a symbol font (Wingdings and the like).
fn parse_cmap(table: &[u8]) -> HashMap<u32, u16> {
    let count = u16_at(table, 2).unwrap_or(0) as usize;
    let mut best: Option<(u8, usize)> = None;
    for i in 0..count {
        let at = 4 + 8 * i;
        let (Some(platform), Some(encoding), Some(offset)) =
            (u16_at(table, at), u16_at(table, at + 2), u32_at(table, at + 4))
        else {
            break;
        };
        let offset = offset as usize;
        let format = u16_at(table, offset).unwrap_or(0);
        let rank = match (platform, encoding, format) {
            (3, 10, 12) | (0, 4 | 6, 12) => 4,
            (3, 1, 4) => 3,
            (0, _, 4) => 2,
            (3, 0, 4) => 1,
            _ => 0,
        };
        if rank > best.map_or(0, |b| b.0) {
            best = Some((rank, offset));
        }
    }
    let mut map = HashMap::new();
    let Some((rank, offset)) = best else {
        return map;
    };
    let Some(sub) = table.get(offset..) else {
        return map;
    };
    if u16_at(sub, 0) == Some(12) {
        let groups = u32_at(sub, 12).unwrap_or(0) as usize;
        for g in 0..groups {
            let at = 16 + 12 * g;
            let (Some(start), Some(end), Some(glyph)) = (u32_at(sub, at), u32_at(sub, at + 4), u32_at(sub, at + 8))
            else {
                break;
            };
            if end < start || end > 0x10FFFF {
                continue;
            }
            for code in start..=end {
                let gid = glyph + (code - start);
                if gid != 0 && gid <= u16::MAX as u32 {
                    map.insert(code, gid as u16);
                }
            }
        }
    } else {
        let segments = u16_at(sub, 6).unwrap_or(0) as usize / 2;
        let ends = 14;
        let starts = ends + 2 * segments + 2;
        let deltas = starts + 2 * segments;
        let ranges = deltas + 2 * segments;
        for s in 0..segments {
            let (Some(end), Some(start), Some(delta), Some(range)) = (
                u16_at(sub, ends + 2 * s),
                u16_at(sub, starts + 2 * s),
                u16_at(sub, deltas + 2 * s),
                u16_at(sub, ranges + 2 * s),
            ) else {
                break;
            };
            if end < start {
                continue;
            }
            for code in start..=end {
                if code == 0xFFFF {
                    break;
                }
                let gid = if range == 0 {
                    code.wrapping_add(delta)
                } else {
                    // The offset is relative to its own position in the table.
                    let at = ranges + 2 * s + range as usize + 2 * (code - start) as usize;
                    match u16_at(sub, at) {
                        Some(0) | None => 0,
                        Some(g) => g.wrapping_add(delta),
                    }
                };
                if gid != 0 {
                    map.insert(code as u32, gid);
                }
            }
        }
    }
    if rank == 1 {
        // Symbol fonts keep their glyphs at U+F020 to U+F0FF; text set in
        // them is typed with the plain characters 0x20 to 0xFF.
        for code in 0xF020..=0xF0FFu32 {
            if let Some(glyph) = map.get(&code).copied() {
                map.entry(code - 0xF000).or_insert(glyph);
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A font of three glyphs made by hand: .notdef (a box), "A" (a
    /// triangle) and a composite of the two mapped to "B".
    pub(crate) fn tiny_font() -> Vec<u8> {
        let simple = |points: &[(i16, i16)]| -> Vec<u8> {
            let mut g: Vec<u8> = Vec::new();
            g.extend(1i16.to_be_bytes());
            for v in [0i16, 0, 500, 700] {
                g.extend(v.to_be_bytes());
            }
            g.extend((points.len() as u16 - 1).to_be_bytes());
            g.extend(0u16.to_be_bytes());
            g.extend(std::iter::repeat_n(1u8, points.len()));
            let (mut px, mut py) = (0, 0);
            let mut xs: Vec<u8> = Vec::new();
            let mut ys: Vec<u8> = Vec::new();
            for (x, y) in points {
                xs.extend((x - px).to_be_bytes());
                ys.extend((y - py).to_be_bytes());
                (px, py) = (*x, *y);
            }
            g.extend(xs);
            g.extend(ys);
            while g.len() % 4 != 0 {
                g.push(0);
            }
            g
        };
        let notdef = simple(&[(0, 0), (500, 0), (500, 700), (0, 700)]);
        let a = simple(&[(0, 0), (500, 0), (250, 700)]);
        let mut composite: Vec<u8> = Vec::new();
        composite.extend((-1i16).to_be_bytes());
        for v in [0i16, 0, 500, 700] {
            composite.extend(v.to_be_bytes());
        }
        // Component 1 (more follow), then component 0, byte offsets.
        composite.extend([0x00, 0x22, 0x00, 0x01, 0, 0]);
        composite.extend([0x00, 0x02, 0x00, 0x00, 0, 0]);
        while composite.len() % 4 != 0 {
            composite.push(0);
        }
        let mut glyf = Vec::new();
        let mut loca: Vec<u8> = Vec::new();
        for g in [&notdef, &a, &composite] {
            loca.extend(((glyf.len() / 2) as u16).to_be_bytes());
            glyf.extend(g.iter());
        }
        loca.extend(((glyf.len() / 2) as u16).to_be_bytes());

        let mut head = vec![0u8; 54];
        head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());
        head[40..42].copy_from_slice(&500i16.to_be_bytes());
        head[42..44].copy_from_slice(&700i16.to_be_bytes());
        let mut hhea = vec![0u8; 36];
        hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
        hhea[6..8].copy_from_slice(&(-200i16).to_be_bytes());
        hhea[34..36].copy_from_slice(&2u16.to_be_bytes());
        let mut maxp = vec![0u8; 32];
        maxp[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        maxp[4..6].copy_from_slice(&3u16.to_be_bytes());
        // Two full metrics; the third glyph repeats the last advance.
        let mut hmtx: Vec<u8> = Vec::new();
        for v in [600u16, 0, 550, 0, 0] {
            hmtx.extend(v.to_be_bytes());
        }
        // One format 4 segment mapping A and B to glyphs 1 and 2.
        let mut cmap: Vec<u8> = Vec::new();
        for v in [0u16, 1, 3, 1, 0, 12] {
            cmap.extend(v.to_be_bytes());
        }
        for v in [4u16, 32, 0, 4, 4, 1, 0] {
            cmap.extend(v.to_be_bytes());
        }
        for v in [0x42u16, 0xFFFF, 0, 0x41, 0xFFFF, (1u16).wrapping_sub(0x41), 1, 0, 0] {
            cmap.extend(v.to_be_bytes());
        }
        let mut name: Vec<u8> = Vec::new();
        let family: Vec<u8> = "Tiny".encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        for v in [0u16, 1, 18, 3, 1, 0x409, 1, family.len() as u16, 0] {
            name.extend(v.to_be_bytes());
        }
        name.extend(&family);
        let mut tables = vec![
            (*b"cmap", cmap),
            (*b"glyf", glyf),
            (*b"head", head),
            (*b"hhea", hhea),
            (*b"hmtx", hmtx),
            (*b"loca", loca),
            (*b"maxp", maxp),
            (*b"name", name),
        ];
        tables.sort_by_key(|(tag, _)| *tag);
        build_font(&tables)
    }

    #[test]
    fn parses_a_font() {
        let font = TtfFont::parse(tiny_font().into(), 0).unwrap();
        assert_eq!(font.names.family, "Tiny");
        assert_eq!(font.units_per_em, 1000);
        assert_eq!((font.ascent, font.descent), (800, -200));
        assert_eq!(font.num_glyphs, 3);
        assert_eq!(font.glyph('A'), Some(1));
        assert_eq!(font.glyph('B'), Some(2));
        assert_eq!(font.glyph('C'), None);
        assert_eq!((font.advance(0), font.advance(1), font.advance(2)), (600, 550, 550));
        assert_eq!(font.components(2), vec![1, 0]);
        assert!(!font.style.bold && font.style.weight == 400);
        // Not fonts.
        assert!(TtfFont::parse(Arc::from(&b"OTTO\0\0\0\0"[..]), 0).is_none());
        assert!(TtfFont::parse(Arc::from(&b""[..]), 0).is_none());
    }

    #[test]
    fn outlines_of_simple_and_composite_glyphs() {
        use OutlineStep::{Close, Line, Move};
        let font = TtfFont::parse(tiny_font().into(), 0).unwrap();
        let triangle = [Move(0.0, 0.0), Line(500.0, 0.0), Line(250.0, 700.0), Close];
        assert_eq!(font.outline(1), triangle);
        // The composite: the triangle, then the box.
        let both = font.outline(2);
        assert_eq!(both[..4], triangle);
        assert_eq!(both[4..], [Move(0.0, 0.0), Line(500.0, 0.0), Line(500.0, 700.0), Line(0.0, 700.0), Close]);
        assert!(font.outline(99).is_empty());
    }

    #[test]
    fn subset_keeps_glyph_numbers() {
        let font = TtfFont::parse(tiny_font().into(), 0).unwrap();
        // Only "A": the composite goes, the box (.notdef) always stays.
        let bytes = font.subset(&BTreeSet::from([1]));
        let sub = TtfFont::parse(bytes.clone().into(), 0).unwrap();
        assert_eq!(sub.num_glyphs, 3);
        assert_eq!(sub.glyph_data(1), font.glyph_data(1));
        assert_eq!(sub.glyph_data(0), font.glyph_data(0));
        assert!(sub.glyph_data(2).is_empty());
        assert_eq!(sub.advance(1), 550);
        // The file checks out: the sum of all its words is the magic number.
        assert_eq!(checksum(&bytes), 0xB1B0_AFBA);
        // A composite brings its parts along.
        let sub = TtfFont::parse(font.subset(&BTreeSet::from([2])).into(), 0).unwrap();
        assert!(!sub.glyph_data(1).is_empty() && !sub.glyph_data(2).is_empty());
    }

    #[test]
    fn symbol_fonts_map_plain_characters() {
        // A Windows symbol subtable mapping U+F041 and U+F042 to glyphs
        // 1 and 2: the letters A and B type them.
        let mut cmap: Vec<u8> = Vec::new();
        for v in [0u16, 1, 3, 0, 0, 12] {
            cmap.extend(v.to_be_bytes());
        }
        for v in [4u16, 32, 0, 4, 4, 1, 0] {
            cmap.extend(v.to_be_bytes());
        }
        for v in [0xF042u16, 0xFFFF, 0, 0xF041, 0xFFFF, 1u16.wrapping_sub(0xF041), 1, 0, 0] {
            cmap.extend(v.to_be_bytes());
        }
        let map = parse_cmap(&cmap);
        assert_eq!(map.get(&0x41), Some(&1));
        assert_eq!(map.get(&0x42), Some(&2));
        assert_eq!(map.get(&0xF042), Some(&2));
        assert_eq!(map.get(&0x43), None);
    }

    #[test]
    fn collection_offsets() {
        let mut ttc: Vec<u8> = b"ttcf".to_vec();
        for v in [0x0001_0000u32, 2, 20, 4000] {
            ttc.extend(v.to_be_bytes());
        }
        assert_eq!(font_offsets(&ttc), vec![20, 4000]);
        assert_eq!(collection_header_len(&ttc), 20);
        assert_eq!(font_offsets(&[0, 1, 0, 0]), vec![0]);
    }
}
