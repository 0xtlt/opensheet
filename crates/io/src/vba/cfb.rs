//! Compound files (MS-CFB), the container of a VBA project: a little file
//! system of storages and streams inside one file.
//!
//! [`Compound::parse`] reads one, whatever its version (sectors of 512 or
//! 4096 bytes); [`write`] makes a version 3 file, small streams in the
//! mini stream as the format requires.

use std::collections::HashSet;

const SIGNATURE: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
const END_OF_CHAIN: u32 = 0xFFFF_FFFE;
const FREE_SECTOR: u32 = 0xFFFF_FFFF;
const FAT_SECTOR: u32 = 0xFFFF_FFFD;
const NO_STREAM: u32 = 0xFFFF_FFFF;
/// Streams shorter than this are kept in the mini stream.
const MINI_CUTOFF: usize = 4096;
const MINI_SECTOR: usize = 64;
/// No project comes near this many directory entries or sectors; a file
/// that claims more is damaged.
const MAX_ENTRIES: usize = 1 << 16;

const KIND_STORAGE: u8 = 1;
const KIND_STREAM: u8 = 2;
const KIND_ROOT: u8 = 5;

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

#[derive(Clone, Debug)]
struct Entry {
    name: String,
    kind: u8,
    left: u32,
    right: u32,
    child: u32,
    start: u32,
    size: u64,
}

/// A compound file that was read.
#[derive(Clone, Debug)]
pub(crate) struct Compound<'a> {
    bytes: &'a [u8],
    sector_size: usize,
    fat: Vec<u32>,
    mini_fat: Vec<u32>,
    /// The mini stream: the content of the root entry.
    mini: Vec<u8>,
    entries: Vec<Entry>,
}

impl<'a> Compound<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Compound<'a>, String> {
        let damaged = || "the compound file is damaged".to_string();
        if bytes.len() < 512 || bytes[..8] != SIGNATURE {
            return Err("not a compound file".to_string());
        }
        let sector_shift = u16_at(bytes, 0x1E).ok_or_else(damaged)?;
        if sector_shift != 9 && sector_shift != 12 {
            return Err(damaged());
        }
        let sector_size = 1usize << sector_shift;
        let mut file = Compound {
            bytes,
            sector_size,
            fat: Vec::new(),
            mini_fat: Vec::new(),
            mini: Vec::new(),
            entries: Vec::new(),
        };

        // The sectors holding the allocation table: 109 are named in the
        // header, the others in a chain of sectors of their own.
        let mut fat_sectors: Vec<u32> = (0..109)
            .filter_map(|i| u32_at(bytes, 0x4C + 4 * i))
            .filter(|s| *s < FAT_SECTOR)
            .collect();
        let mut next = u32_at(bytes, 0x44).ok_or_else(damaged)?;
        let mut seen = HashSet::new();
        while next < FAT_SECTOR && seen.insert(next) {
            let sector = file.sector(next).ok_or_else(damaged)?;
            let per = sector_size / 4 - 1;
            fat_sectors.extend(
                (0..per)
                    .filter_map(|i| u32_at(sector, 4 * i))
                    .filter(|s| *s < FAT_SECTOR),
            );
            next = u32_at(sector, 4 * per).ok_or_else(damaged)?;
        }
        for sector in fat_sectors {
            let sector = file.sector(sector).ok_or_else(damaged)?;
            file.fat.extend(
                sector
                    .chunks_exact(4)
                    .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])),
            );
        }

        let directory = file
            .chain(u32_at(bytes, 0x30).ok_or_else(damaged)?, None)
            .ok_or_else(damaged)?;
        for raw in directory.chunks_exact(128).take(MAX_ENTRIES) {
            let name_len = (u16_at(raw, 0x40).unwrap_or(0) as usize).min(64);
            let units: Vec<u16> = raw[..name_len.saturating_sub(2)]
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            let mut size = u64::from_le_bytes(raw[0x78..0x80].try_into().map_err(|_| damaged())?);
            if sector_shift == 9 {
                // Version 3 files only use the low half of the size.
                size &= 0xFFFF_FFFF;
            }
            file.entries.push(Entry {
                name: String::from_utf16_lossy(&units),
                kind: raw[0x42],
                left: u32_at(raw, 0x44).unwrap_or(NO_STREAM),
                right: u32_at(raw, 0x48).unwrap_or(NO_STREAM),
                child: u32_at(raw, 0x4C).unwrap_or(NO_STREAM),
                start: u32_at(raw, 0x74).unwrap_or(END_OF_CHAIN),
                size,
            });
        }
        let root = file
            .entries
            .first()
            .filter(|e| e.kind == KIND_ROOT)
            .ok_or_else(damaged)?
            .clone();

        let mini_fat = file
            .chain(u32_at(bytes, 0x3C).ok_or_else(damaged)?, None)
            .ok_or_else(damaged)?;
        file.mini_fat = mini_fat
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        if root.size > 0 {
            file.mini = file
                .chain(root.start, Some(root.size as usize))
                .ok_or_else(damaged)?;
        }
        Ok(file)
    }

    fn sector(&self, number: u32) -> Option<&'a [u8]> {
        let start = (number as usize)
            .checked_add(1)?
            .checked_mul(self.sector_size)?;
        self.bytes.get(start..start.checked_add(self.sector_size)?)
    }

    /// The sectors chained from `start`, cut to `size` bytes when given.
    /// `None` when the chain leaves the file or turns in a circle.
    fn chain(&self, start: u32, size: Option<usize>) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        let mut next = start;
        let mut seen = HashSet::new();
        while next < FAT_SECTOR {
            if !seen.insert(next) {
                return None;
            }
            out.extend_from_slice(self.sector(next)?);
            if size.is_some_and(|size| out.len() >= size) {
                break;
            }
            next = *self.fat.get(next as usize)?;
        }
        if let Some(size) = size {
            if out.len() < size {
                return None;
            }
            out.truncate(size);
        }
        Some(out)
    }

    fn mini_chain(&self, start: u32, size: usize) -> Option<Vec<u8>> {
        let mut out = Vec::with_capacity(size);
        let mut next = start;
        let mut seen = HashSet::new();
        while next < FAT_SECTOR && out.len() < size {
            if !seen.insert(next) {
                return None;
            }
            let at = (next as usize).checked_mul(MINI_SECTOR)?;
            out.extend_from_slice(self.mini.get(at..at + MINI_SECTOR)?);
            next = *self.mini_fat.get(next as usize)?;
        }
        if out.len() < size {
            return None;
        }
        out.truncate(size);
        Some(out)
    }

    /// The entries of a storage, by walking the tree of its children.
    fn children(&self, storage: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stack = vec![self.entries[storage].child];
        let mut seen = HashSet::new();
        while let Some(id) = stack.pop() {
            let Some(entry) = self.entries.get(id as usize) else {
                continue;
            };
            if !seen.insert(id) {
                continue;
            }
            out.push(id as usize);
            stack.push(entry.left);
            stack.push(entry.right);
        }
        out
    }

    /// The entry at a path from the root; names compare without case.
    fn find(&self, path: &[&str]) -> Option<usize> {
        let mut at = 0;
        for name in path {
            at = self
                .children(at)
                .into_iter()
                .find(|id| self.entries[*id].name.eq_ignore_ascii_case(name))?;
        }
        Some(at)
    }

    /// The content of the stream at a path (`["VBA", "dir"]`).
    pub fn stream(&self, path: &[&str]) -> Option<Vec<u8>> {
        let entry = &self.entries[self.find(path)?];
        if entry.kind != KIND_STREAM {
            return None;
        }
        let size = usize::try_from(entry.size).ok()?;
        if size > self.bytes.len() {
            return None;
        }
        if size < MINI_CUTOFF {
            self.mini_chain(entry.start, size)
        } else {
            self.chain(entry.start, Some(size))
        }
    }

    /// The names of the streams of the storage at a path.
    #[cfg(test)]
    pub fn stream_names(&self, path: &[&str]) -> Vec<String> {
        let Some(storage) = self.find(path) else {
            return Vec::new();
        };
        let mut names: Vec<String> = self
            .children(storage)
            .into_iter()
            .map(|id| &self.entries[id])
            .filter(|e| e.kind == KIND_STREAM)
            .map(|e| e.name.clone())
            .collect();
        names.sort();
        names
    }
}

// ------------------------------------------------------------------ writing

/// Something to put into a compound file.
#[derive(Clone, Debug)]
pub(crate) enum Node {
    Stream(String, Vec<u8>),
    Storage(String, Vec<Node>),
}

impl Node {
    fn name(&self) -> &str {
        match self {
            Node::Stream(name, _) | Node::Storage(name, _) => name,
        }
    }
}

struct NewEntry {
    name: String,
    kind: u8,
    red: bool,
    left: u32,
    right: u32,
    child: u32,
    data: Vec<u8>,
}

/// The order of the entries of a storage: shorter names first, then by
/// their upper-case letters.
fn name_order(a: &str, b: &str) -> std::cmp::Ordering {
    let (a16, b16): (Vec<u16>, Vec<u16>) = (
        a.to_uppercase().encode_utf16().collect(),
        b.to_uppercase().encode_utf16().collect(),
    );
    a16.len().cmp(&b16.len()).then(a16.cmp(&b16))
}

/// Make a balanced tree of the entries `ids` (already in order) and
/// return its root. The entries of the deepest level are red when that
/// level is not full, which is what makes the tree a red-black one.
fn link(entries: &mut [NewEntry], ids: &[u32], depth: u32, height: u32, full: bool) -> u32 {
    if ids.is_empty() {
        return NO_STREAM;
    }
    let mid = ids.len() / 2;
    let id = ids[mid];
    let left = link(entries, &ids[..mid], depth + 1, height, full);
    let right = link(entries, &ids[mid + 1..], depth + 1, height, full);
    let entry = &mut entries[id as usize];
    entry.left = left;
    entry.right = right;
    entry.red = !full && depth == height;
    id
}

fn add(entries: &mut Vec<NewEntry>, nodes: Vec<Node>) -> u32 {
    let mut nodes = nodes;
    nodes.sort_by(|a, b| name_order(a.name(), b.name()));
    let mut ids = Vec::new();
    for node in nodes {
        let id = entries.len() as u32;
        ids.push(id);
        match node {
            Node::Stream(name, data) => entries.push(NewEntry {
                name,
                kind: KIND_STREAM,
                red: false,
                left: NO_STREAM,
                right: NO_STREAM,
                child: NO_STREAM,
                data,
            }),
            Node::Storage(name, children) => {
                entries.push(NewEntry {
                    name,
                    kind: KIND_STORAGE,
                    red: false,
                    left: NO_STREAM,
                    right: NO_STREAM,
                    child: NO_STREAM,
                    data: Vec::new(),
                });
                let child = add(entries, children);
                entries[id as usize].child = child;
            }
        }
    }
    // A tree of n entries has ⌊log2 n⌋ + 1 levels, the last one full when
    // n + 1 is a power of two.
    let count = ids.len() as u32;
    let height = 32 - count.leading_zeros();
    link(entries, &ids, 1, height, (count + 1).is_power_of_two())
}

fn pad(bytes: &mut Vec<u8>, unit: usize) {
    let rest = bytes.len() % unit;
    if rest != 0 {
        bytes.resize(bytes.len() + unit - rest, 0);
    }
}

/// A compound file holding `nodes` under its root.
pub(crate) fn write(nodes: Vec<Node>) -> Vec<u8> {
    const SECTOR: usize = 512;
    let mut entries = vec![NewEntry {
        name: "Root Entry".to_string(),
        kind: KIND_ROOT,
        red: false,
        left: NO_STREAM,
        right: NO_STREAM,
        child: NO_STREAM,
        data: Vec::new(),
    }];
    let child = add(&mut entries, nodes);
    entries[0].child = child;

    // Small streams go one after the other into the mini stream.
    let mut mini = Vec::new();
    let mut mini_fat: Vec<u32> = Vec::new();
    let mut starts = vec![END_OF_CHAIN; entries.len()];
    for (id, entry) in entries.iter().enumerate() {
        if entry.kind != KIND_STREAM || entry.data.is_empty() || entry.data.len() >= MINI_CUTOFF {
            continue;
        }
        let first = mini_fat.len() as u32;
        starts[id] = first;
        mini.extend_from_slice(&entry.data);
        pad(&mut mini, MINI_SECTOR);
        let count = entry.data.len().div_ceil(MINI_SECTOR) as u32;
        mini_fat.extend((1..count).map(|k| first + k));
        mini_fat.push(END_OF_CHAIN);
    }

    // The sectors, in this order: the allocation table, the directory,
    // the mini allocation table, the mini stream, the large streams.
    let sectors_of = |len: usize| len.div_ceil(SECTOR);
    let directory_sectors = sectors_of(entries.len() * 128);
    let mini_fat_sectors = sectors_of(mini_fat.len() * 4);
    let mini_sectors = sectors_of(mini.len());
    let large: Vec<usize> = (0..entries.len())
        .filter(|id| entries[*id].kind == KIND_STREAM && entries[*id].data.len() >= MINI_CUTOFF)
        .collect();
    let large_sectors: usize = large
        .iter()
        .map(|id| sectors_of(entries[*id].data.len()))
        .sum();
    let others = directory_sectors + mini_fat_sectors + mini_sectors + large_sectors;
    let mut fat_sectors = 1;
    while (fat_sectors + others) > fat_sectors * (SECTOR / 4) {
        fat_sectors += 1;
    }
    assert!(
        fat_sectors <= 109,
        "too large for a compound file without a chain of table sectors"
    );

    let mut fat: Vec<u32> = vec![FAT_SECTOR; fat_sectors];
    let chain = |fat: &mut Vec<u32>, count: usize| -> u32 {
        if count == 0 {
            return END_OF_CHAIN;
        }
        let first = fat.len() as u32;
        fat.extend((1..count as u32).map(|k| first + k));
        fat.push(END_OF_CHAIN);
        first
    };
    let directory_start = chain(&mut fat, directory_sectors);
    let mini_fat_start = chain(&mut fat, mini_fat_sectors);
    starts[0] = chain(&mut fat, mini_sectors);
    for id in &large {
        starts[*id] = chain(&mut fat, sectors_of(entries[*id].data.len()));
    }

    let mut out = Vec::with_capacity((1 + fat_sectors + others) * SECTOR);
    out.extend_from_slice(&SIGNATURE);
    out.extend_from_slice(&[0; 16]);
    out.extend_from_slice(&0x003Eu16.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes());
    out.extend_from_slice(&0xFFFEu16.to_le_bytes());
    out.extend_from_slice(&9u16.to_le_bytes());
    out.extend_from_slice(&6u16.to_le_bytes());
    out.extend_from_slice(&[0; 6]);
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(fat_sectors as u32).to_le_bytes());
    out.extend_from_slice(&directory_start.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(MINI_CUTOFF as u32).to_le_bytes());
    out.extend_from_slice(&mini_fat_start.to_le_bytes());
    out.extend_from_slice(&(mini_fat_sectors as u32).to_le_bytes());
    out.extend_from_slice(&END_OF_CHAIN.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for i in 0..109u32 {
        let sector = if (i as usize) < fat_sectors {
            i
        } else {
            FREE_SECTOR
        };
        out.extend_from_slice(&sector.to_le_bytes());
    }
    debug_assert_eq!(out.len(), SECTOR);

    fat.resize(fat_sectors * (SECTOR / 4), FREE_SECTOR);
    for value in &fat {
        out.extend_from_slice(&value.to_le_bytes());
    }
    for (id, entry) in entries.iter().enumerate() {
        let mut raw = [0u8; 128];
        let units: Vec<u16> = entry.name.encode_utf16().take(31).collect();
        for (i, unit) in units.iter().enumerate() {
            raw[2 * i..2 * i + 2].copy_from_slice(&unit.to_le_bytes());
        }
        raw[0x40..0x42].copy_from_slice(&((units.len() as u16 + 1) * 2).to_le_bytes());
        raw[0x42] = entry.kind;
        raw[0x43] = if entry.red { 0 } else { 1 };
        raw[0x44..0x48].copy_from_slice(&entry.left.to_le_bytes());
        raw[0x48..0x4C].copy_from_slice(&entry.right.to_le_bytes());
        raw[0x4C..0x50].copy_from_slice(&entry.child.to_le_bytes());
        let start = if entry.kind == KIND_STORAGE {
            0
        } else {
            starts[id]
        };
        raw[0x74..0x78].copy_from_slice(&start.to_le_bytes());
        let size = if id == 0 {
            mini.len()
        } else {
            entry.data.len()
        };
        raw[0x78..0x80].copy_from_slice(&(size as u64).to_le_bytes());
        out.extend_from_slice(&raw);
    }
    // Unused entries of the last directory sector: no stream, no siblings.
    while out.len() % SECTOR != 0 {
        let mut raw = [0u8; 128];
        for at in [0x44, 0x48, 0x4C] {
            raw[at..at + 4].copy_from_slice(&NO_STREAM.to_le_bytes());
        }
        out.extend_from_slice(&raw);
    }
    for value in &mini_fat {
        out.extend_from_slice(&value.to_le_bytes());
    }
    while out.len() % SECTOR != 0 {
        out.extend_from_slice(&FREE_SECTOR.to_le_bytes());
    }
    out.extend_from_slice(&mini);
    pad(&mut out, SECTOR);
    for id in &large {
        out.extend_from_slice(&entries[*id].data);
        pad(&mut out, SECTOR);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(len: usize, seed: u8) -> Vec<u8> {
        (0..len)
            .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
            .collect()
    }

    #[test]
    fn what_is_written_is_read_back() {
        // Small and large streams, an empty one, a storage in a storage,
        // and enough entries for a tree of several levels.
        let mut inner: Vec<Node> = (0..11)
            .map(|i| Node::Stream(format!("Module{i}"), bytes(100 + 700 * i, i as u8)))
            .collect();
        inner.push(Node::Stream("dir".to_string(), bytes(5000, 7)));
        inner.push(Node::Stream("empty".to_string(), Vec::new()));
        inner.push(Node::Storage(
            "Forms".to_string(),
            vec![Node::Stream("f".to_string(), bytes(64, 1))],
        ));
        let file = write(vec![
            Node::Stream("PROJECT".to_string(), bytes(300, 3)),
            Node::Stream("PROJECTwm".to_string(), bytes(4096, 4)),
            Node::Storage("VBA".to_string(), inner),
        ]);
        assert_eq!(file.len() % 512, 0);
        let read = Compound::parse(&file).expect("a compound file");
        assert_eq!(read.stream(&["PROJECT"]), Some(bytes(300, 3)));
        assert_eq!(read.stream(&["projectWM"]), Some(bytes(4096, 4)));
        assert_eq!(read.stream(&["VBA", "dir"]), Some(bytes(5000, 7)));
        assert_eq!(read.stream(&["VBA", "empty"]), Some(Vec::new()));
        assert_eq!(read.stream(&["VBA", "Forms", "f"]), Some(bytes(64, 1)));
        for i in 0..11 {
            assert_eq!(
                read.stream(&["VBA", &format!("Module{i}")]),
                Some(bytes(100 + 700 * i, i as u8)),
                "Module{i}"
            );
        }
        assert_eq!(read.stream_names(&["VBA"]).len(), 13);
        // A storage is not a stream, and what is not there is not found.
        assert_eq!(read.stream(&["VBA"]), None);
        assert_eq!(read.stream(&["VBA", "Module99"]), None);
    }

    #[test]
    fn the_tree_of_a_storage_is_a_red_black_tree() {
        for count in 1..40u32 {
            let nodes: Vec<Node> = (0..count)
                .map(|i| Node::Stream(format!("s{i:02}"), vec![1]))
                .collect();
            let mut entries = vec![NewEntry {
                name: "Root Entry".to_string(),
                kind: KIND_ROOT,
                red: false,
                left: NO_STREAM,
                right: NO_STREAM,
                child: NO_STREAM,
                data: Vec::new(),
            }];
            let root = add(&mut entries, nodes);
            // Every path to a leaf crosses as many black entries, no red
            // entry has a red child, names are in order.
            fn check(
                entries: &[NewEntry],
                id: u32,
                parent_red: bool,
                lo: Option<&str>,
                hi: Option<&str>,
            ) -> u32 {
                if id == NO_STREAM {
                    return 1;
                }
                let entry = &entries[id as usize];
                assert!(!(parent_red && entry.red));
                assert!(lo.is_none_or(|lo| name_order(lo, &entry.name).is_lt()));
                assert!(hi.is_none_or(|hi| name_order(&entry.name, hi).is_lt()));
                let left = check(entries, entry.left, entry.red, lo, Some(&entry.name));
                let right = check(entries, entry.right, entry.red, Some(&entry.name), hi);
                assert_eq!(left, right, "{} entries", entries.len() - 1);
                left + u32::from(!entry.red)
            }
            assert!(!entries[root as usize].red);
            check(&entries, root, false, None, None);
        }
    }

    #[test]
    fn damaged_files_are_refused_not_followed() {
        assert!(Compound::parse(b"PK\x03\x04").is_err());
        let file = write(vec![Node::Stream("a".to_string(), bytes(6000, 1))]);
        // A chain that turns in a circle.
        let mut circle = file.clone();
        let fat = 512;
        let first = Compound::parse(&file).unwrap().entries[1].start as usize;
        circle[fat + 4 * first..fat + 4 * first + 4].copy_from_slice(&(first as u32).to_le_bytes());
        assert_eq!(Compound::parse(&circle).unwrap().stream(&["a"]), None);
        // A file cut short.
        let cut = &file[..file.len() - 1024];
        assert!(
            Compound::parse(cut)
                .map(|c| c.stream(&["a"]))
                .unwrap_or(None)
                .is_none()
        );
        // Any byte changed: no panic.
        for at in (0..file.len()).step_by(7) {
            let mut bad = file.clone();
            bad[at] ^= 0xFF;
            if let Ok(read) = Compound::parse(&bad) {
                let _ = read.stream(&["a"]);
            }
        }
    }
}
