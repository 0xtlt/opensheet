//! Protection of sheets (`Sheet::protection`) and of the workbook
//! structure (`Workbook::protection`).
//!
//! Protection refuses actions, it changes nothing by itself: before acting
//! the application asks `Workbook::permits`. The editing functions of the
//! engine do not ask on their own, so that loading, undo and the engine's
//! own bookkeeping are never refused.
//!
//! Passwords are kept as the hashes files store, in both of Excel's
//! schemes, so that a sheet protected elsewhere can be unprotected here
//! and the other way round. They guard against accidents, not against
//! attackers: the cells are not encrypted.

use crate::ast::StructuralEdit;
use crate::features::CellShift;
use crate::locale::Locale;
use crate::refs::{MAX_COLS, MAX_ROWS, Rect};
use crate::style::StyleId;
use crate::validation::localized;
use crate::workbook::{Sheet, Workbook};
use std::collections::{BTreeMap, HashMap};

// ---- hashing -------------------------------------------------------------

const SHA512_K: [u64; 80] = [
    0x428a2f98d728ae22,
    0x7137449123ef65cd,
    0xb5c0fbcfec4d3b2f,
    0xe9b5dba58189dbbc,
    0x3956c25bf348b538,
    0x59f111f1b605d019,
    0x923f82a4af194f9b,
    0xab1c5ed5da6d8118,
    0xd807aa98a3030242,
    0x12835b0145706fbe,
    0x243185be4ee4b28c,
    0x550c7dc3d5ffb4e2,
    0x72be5d74f27b896f,
    0x80deb1fe3b1696b1,
    0x9bdc06a725c71235,
    0xc19bf174cf692694,
    0xe49b69c19ef14ad2,
    0xefbe4786384f25e3,
    0x0fc19dc68b8cd5b5,
    0x240ca1cc77ac9c65,
    0x2de92c6f592b0275,
    0x4a7484aa6ea6e483,
    0x5cb0a9dcbd41fbd4,
    0x76f988da831153b5,
    0x983e5152ee66dfab,
    0xa831c66d2db43210,
    0xb00327c898fb213f,
    0xbf597fc7beef0ee4,
    0xc6e00bf33da88fc2,
    0xd5a79147930aa725,
    0x06ca6351e003826f,
    0x142929670a0e6e70,
    0x27b70a8546d22ffc,
    0x2e1b21385c26c926,
    0x4d2c6dfc5ac42aed,
    0x53380d139d95b3df,
    0x650a73548baf63de,
    0x766a0abb3c77b2a8,
    0x81c2c92e47edaee6,
    0x92722c851482353b,
    0xa2bfe8a14cf10364,
    0xa81a664bbc423001,
    0xc24b8b70d0f89791,
    0xc76c51a30654be30,
    0xd192e819d6ef5218,
    0xd69906245565a910,
    0xf40e35855771202a,
    0x106aa07032bbd1b8,
    0x19a4c116b8d2d0c8,
    0x1e376c085141ab53,
    0x2748774cdf8eeb99,
    0x34b0bcb5e19b48a8,
    0x391c0cb3c5c95a63,
    0x4ed8aa4ae3418acb,
    0x5b9cca4f7763e373,
    0x682e6ff3d6b2b8a3,
    0x748f82ee5defb2fc,
    0x78a5636f43172f60,
    0x84c87814a1f0ab72,
    0x8cc702081a6439ec,
    0x90befffa23631e28,
    0xa4506cebde82bde9,
    0xbef9a3f7b2c67915,
    0xc67178f2e372532b,
    0xca273eceea26619c,
    0xd186b8c721c0c207,
    0xeada7dd6cde0eb1e,
    0xf57d4f7fee6ed178,
    0x06f067aa72176fba,
    0x0a637dc5a2c898a6,
    0x113f9804bef90dae,
    0x1b710b35131c471b,
    0x28db77f523047d84,
    0x32caab7b40c72493,
    0x3c9ebe0a15c9bebc,
    0x431d67c49c100d4c,
    0x4cc5d4becb3e42b6,
    0x597f299cfc657e2a,
    0x5fcb6fab3ad6faec,
    0x6c44198c4a475817,
];

const SHA512_INIT: [u64; 8] = [
    0x6a09e667f3bcc908,
    0xbb67ae8584caa73b,
    0x3c6ef372fe94f82b,
    0xa54ff53a5f1d36f1,
    0x510e527fade682d1,
    0x9b05688c2b3e6c1f,
    0x1f83d9abfb41bd6b,
    0x5be0cd19137e2179,
];

const SHA384_INIT: [u64; 8] = [
    0xcbbb9d5dc1059ed8,
    0x629a292a367cd507,
    0x9159015a3070dd17,
    0x152fecd8f70e5939,
    0x67332667ffc00b31,
    0x8eb44a8768581511,
    0xdb0c2e0d64f98fa7,
    0x47b5481dbefa4fa4,
];

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
    0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
    0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
    0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
    0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
    0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
    0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
    0xc67178f2,
];

const SHA256_INIT: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

/// The message followed by the padding all these hashes share: a `0x80`
/// byte, zeros up to a whole number of blocks, the length in bits at the
/// end (big-endian, in `length_bytes` bytes).
fn padded(data: &[u8], block: usize, length_bytes: usize) -> Vec<u8> {
    let mut out = data.to_vec();
    out.push(0x80);
    while !(out.len() + length_bytes).is_multiple_of(block) {
        out.push(0);
    }
    let bits = (data.len() as u128) * 8;
    out.extend_from_slice(&bits.to_be_bytes()[16 - length_bytes..]);
    out
}

/// SHA-512, or SHA-384 with its own starting state.
fn sha512_core(data: &[u8], mut state: [u64; 8]) -> [u64; 8] {
    for block in padded(data, 128, 16).as_chunks::<128>().0 {
        let mut w = [0u64; 80];
        for (word, bytes) in w.iter_mut().zip(block.as_chunks::<8>().0) {
            *word = u64::from_be_bytes(*bytes);
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for i in 0..80 {
            let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
            let choice = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(choice)
                .wrapping_add(SHA512_K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(majority);
            (h, g, f, e, d, c, b, a) = (g, f, e, d.wrapping_add(t1), c, b, a, t1.wrapping_add(t2));
        }
        for (s, v) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *s = s.wrapping_add(v);
        }
    }
    state
}

fn sha512(data: &[u8]) -> Vec<u8> {
    sha512_core(data, SHA512_INIT)
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect()
}

fn sha384(data: &[u8]) -> Vec<u8> {
    sha512_core(data, SHA384_INIT)[..6]
        .iter()
        .flat_map(|w| w.to_be_bytes())
        .collect()
}

fn sha256(data: &[u8]) -> Vec<u8> {
    let mut state = SHA256_INIT;
    for block in padded(data, 64, 8).as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (word, bytes) in w.iter_mut().zip(block.as_chunks::<4>().0) {
            *word = u32::from_be_bytes(*bytes);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let choice = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(choice)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let majority = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(majority);
            (h, g, f, e, d, c, b, a) = (g, f, e, d.wrapping_add(t1), c, b, a, t1.wrapping_add(t2));
        }
        for (s, v) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *s = s.wrapping_add(v);
        }
    }
    state.iter().flat_map(|w| w.to_be_bytes()).collect()
}

fn sha1(data: &[u8]) -> Vec<u8> {
    let mut state: [u32; 5] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476, 0xc3d2e1f0];
    for block in padded(data, 64, 8).as_chunks::<64>().0 {
        let mut w = [0u32; 80];
        for (word, bytes) in w.iter_mut().zip(block.as_chunks::<4>().0) {
            *word = u32::from_be_bytes(*bytes);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = state;
        for (i, word) in w.iter().enumerate() {
            let (mix, k) = match i / 20 {
                0 => ((b & c) | (!b & d), 0x5a827999),
                1 => (b ^ c ^ d, 0x6ed9eba1),
                2 => ((b & c) | (b & d) | (c & d), 0x8f1bbcdc),
                _ => (b ^ c ^ d, 0xca62c1d6u32),
            };
            let next = a
                .rotate_left(5)
                .wrapping_add(mix)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            (e, d, c, b, a) = (d, c, b.rotate_left(30), a, next);
        }
        for (s, v) in state.iter_mut().zip([a, b, c, d, e]) {
            *s = s.wrapping_add(v);
        }
    }
    state.iter().flat_map(|w| w.to_be_bytes()).collect()
}

type Digest = fn(&[u8]) -> Vec<u8>;

/// The hash function a file names in `algorithmName`; `None` for those not
/// implemented (MD2, MD4, MD5, RIPEMD, WHIRLPOOL: Excel writes none).
fn digest(algorithm: &str) -> Option<Digest> {
    match algorithm.to_ascii_uppercase().replace('-', "").as_str() {
        "SHA512" => Some(sha512),
        "SHA384" => Some(sha384),
        "SHA256" => Some(sha256),
        "SHA1" => Some(sha1),
        _ => None,
    }
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Base64 as files write `hashValue` and `saltValue`.
fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = chunk.iter().fold(0u32, |n, b| (n << 8) | *b as u32) << (8 * (3 - chunk.len()));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(BASE64[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `None` when the text is not Base64.
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut bits, mut count) = (0u32, 0);
    for byte in text.bytes().filter(|b| !b.is_ascii_whitespace() && *b != b'=') {
        bits = (bits << 6) | BASE64.iter().position(|c| *c == byte)? as u32;
        count += 1;
        if count == 4 {
            out.extend_from_slice(&bits.to_be_bytes()[1..]);
            (bits, count) = (0, 0);
        }
    }
    match count {
        0 => {}
        2 => out.push((bits >> 4) as u8),
        3 => out.extend_from_slice(&[(bits >> 10) as u8, (bits >> 2) as u8]),
        _ => return None,
    }
    Some(out)
}

/// The byte Windows uses for a character in western European text; `?`
/// for characters it has none for. The old password hash works on these
/// bytes.
fn ansi_byte(c: char) -> u8 {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘',
        '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
    ];
    match c as u32 {
        code @ (0..=0x7f | 0xa0..=0xff) => code as u8,
        _ => HIGH.iter().position(|h| *h == c).map_or(b'?', |i| 0x80 + i as u8),
    }
}

/// The 16-bit hash older files keep in `password`.
fn legacy_hash(password: &str) -> u16 {
    let bytes: Vec<u8> = password.chars().map(ansi_byte).collect();
    let rotate = |hash: u16| ((hash >> 14) & 1) | ((hash << 1) & 0x7fff);
    let hash = bytes.iter().rev().fold(0u16, |hash, byte| rotate(hash) ^ *byte as u16);
    rotate(hash) ^ bytes.len() as u16 ^ 0xce4b
}

/// The salted hash of current files: the hash of salt + password (UTF-16),
/// hashed again `spin_count` times with the number of the round appended.
fn spun_hash(algorithm: &str, password: &str, salt: &[u8], spin_count: u32) -> Option<Vec<u8>> {
    let hash = digest(algorithm)?;
    let mut input = salt.to_vec();
    input.extend(password.encode_utf16().flat_map(u16::to_le_bytes));
    let mut value = hash(&input);
    for round in 0..spin_count {
        value.extend_from_slice(&round.to_le_bytes());
        value = hash(&value);
    }
    Some(value)
}

/// Sixteen bytes that differ every time: enough for a salt.
fn random_salt() -> [u8; 16] {
    use std::hash::{BuildHasher, Hasher};
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut salt = [0u8; 16];
    for half in salt.as_chunks_mut::<8>().0 {
        // Every `RandomState` carries keys drawn from the system.
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(now);
        *half = hasher.finish().to_le_bytes();
    }
    salt
}

/// A password as files keep it: never the text, only a hash of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PasswordHash {
    /// The 16-bit hash of older files: `password="83AF"` (hexadecimal),
    /// `workbookPassword` for a workbook.
    Legacy(u16),
    /// `algorithmName`, `hashValue`, `saltValue` and `spinCount`
    /// (`workbookAlgorithmName`… for a workbook); hash and salt in Base64,
    /// exactly as written in the file.
    Salted {
        algorithm: String,
        hash: String,
        salt: String,
        spin_count: u32,
    },
}

impl PasswordHash {
    /// Hash a password the way Excel does today: SHA-512, a fresh salt of
    /// 16 bytes, 100 000 rounds.
    pub fn new(password: &str) -> PasswordHash {
        PasswordHash::salted(password, &random_salt(), 100_000)
    }

    /// SHA-512 with a given salt and number of rounds.
    pub fn salted(password: &str, salt: &[u8], spin_count: u32) -> PasswordHash {
        let hash = spun_hash("SHA-512", password, salt, spin_count).unwrap_or_default();
        PasswordHash::Salted {
            algorithm: "SHA-512".to_string(),
            hash: base64_encode(&hash),
            salt: base64_encode(salt),
            spin_count,
        }
    }

    /// The 16-bit hash, for files older applications must unprotect.
    pub fn legacy(password: &str) -> PasswordHash {
        PasswordHash::Legacy(legacy_hash(password))
    }

    /// Whether `password` is the one that was hashed. A hash made with an
    /// algorithm this module does not know matches nothing.
    pub fn verify(&self, password: &str) -> bool {
        match self {
            PasswordHash::Legacy(hash) => legacy_hash(password) == *hash,
            PasswordHash::Salted {
                algorithm,
                hash,
                salt,
                spin_count,
            } => {
                let expected = base64_decode(hash);
                let found = base64_decode(salt).and_then(|salt| spun_hash(algorithm, password, &salt, *spin_count));
                expected.is_some() && expected == found
            }
        }
    }
}

// ---- model ---------------------------------------------------------------

/// What the users of a protected sheet may still do: the check boxes of
/// the Protect Sheet dialog, in its order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Permission {
    SelectLockedCells,
    SelectUnlockedCells,
    FormatCells,
    FormatColumns,
    FormatRows,
    InsertColumns,
    InsertRows,
    InsertHyperlinks,
    DeleteColumns,
    DeleteRows,
    Sort,
    AutoFilter,
    PivotTables,
    EditObjects,
    EditScenarios,
}

impl Permission {
    pub const ALL: [Permission; 15] = [
        Permission::SelectLockedCells,
        Permission::SelectUnlockedCells,
        Permission::FormatCells,
        Permission::FormatColumns,
        Permission::FormatRows,
        Permission::InsertColumns,
        Permission::InsertRows,
        Permission::InsertHyperlinks,
        Permission::DeleteColumns,
        Permission::DeleteRows,
        Permission::Sort,
        Permission::AutoFilter,
        Permission::PivotTables,
        Permission::EditObjects,
        Permission::EditScenarios,
    ];

    /// The attribute of `<sheetProtection>`. It says the opposite of the
    /// check box: `1` forbids, `0` allows.
    pub fn ooxml_name(self) -> &'static str {
        match self {
            Permission::SelectLockedCells => "selectLockedCells",
            Permission::SelectUnlockedCells => "selectUnlockedCells",
            Permission::FormatCells => "formatCells",
            Permission::FormatColumns => "formatColumns",
            Permission::FormatRows => "formatRows",
            Permission::InsertColumns => "insertColumns",
            Permission::InsertRows => "insertRows",
            Permission::InsertHyperlinks => "insertHyperlinks",
            Permission::DeleteColumns => "deleteColumns",
            Permission::DeleteRows => "deleteRows",
            Permission::Sort => "sort",
            Permission::AutoFilter => "autoFilter",
            Permission::PivotTables => "pivotTables",
            Permission::EditObjects => "objects",
            Permission::EditScenarios => "scenarios",
        }
    }

    /// Whether the action is allowed when a file leaves the attribute out:
    /// selecting cells, objects and scenarios are, the rest is not.
    pub fn allowed_when_absent(self) -> bool {
        matches!(
            self,
            Permission::SelectLockedCells
                | Permission::SelectUnlockedCells
                | Permission::EditObjects
                | Permission::EditScenarios
        )
    }

    /// The text of the check box.
    pub fn label(self, locale: &Locale) -> &'static str {
        localized(
            locale,
            match self {
                Permission::SelectLockedCells => [
                    "Select locked cells",
                    "Sélectionner les cellules verrouillées",
                    "Seleccionar celdas bloqueadas",
                    "Gesperrte Zellen auswählen",
                    "Selezionare celle bloccate",
                ],
                Permission::SelectUnlockedCells => [
                    "Select unlocked cells",
                    "Sélectionner les cellules déverrouillées",
                    "Seleccionar celdas desbloqueadas",
                    "Nicht gesperrte Zellen auswählen",
                    "Selezionare celle sbloccate",
                ],
                Permission::FormatCells => [
                    "Format cells",
                    "Mettre en forme les cellules",
                    "Aplicar formato a celdas",
                    "Zellen formatieren",
                    "Formattare celle",
                ],
                Permission::FormatColumns => [
                    "Format columns",
                    "Mettre en forme les colonnes",
                    "Aplicar formato a columnas",
                    "Spalten formatieren",
                    "Formattare colonne",
                ],
                Permission::FormatRows => [
                    "Format rows",
                    "Mettre en forme les lignes",
                    "Aplicar formato a filas",
                    "Zeilen formatieren",
                    "Formattare righe",
                ],
                Permission::InsertColumns => [
                    "Insert columns",
                    "Insérer des colonnes",
                    "Insertar columnas",
                    "Spalten einfügen",
                    "Inserire colonne",
                ],
                Permission::InsertRows => [
                    "Insert rows",
                    "Insérer des lignes",
                    "Insertar filas",
                    "Zeilen einfügen",
                    "Inserire righe",
                ],
                Permission::InsertHyperlinks => [
                    "Insert hyperlinks",
                    "Insérer des liens hypertexte",
                    "Insertar hipervínculos",
                    "Links einfügen",
                    "Inserire collegamenti ipertestuali",
                ],
                Permission::DeleteColumns => [
                    "Delete columns",
                    "Supprimer des colonnes",
                    "Eliminar columnas",
                    "Spalten löschen",
                    "Eliminare colonne",
                ],
                Permission::DeleteRows => [
                    "Delete rows",
                    "Supprimer des lignes",
                    "Eliminar filas",
                    "Zeilen löschen",
                    "Eliminare righe",
                ],
                Permission::Sort => ["Sort", "Trier", "Ordenar", "Sortieren", "Ordinare"],
                Permission::AutoFilter => [
                    "Use AutoFilter",
                    "Utiliser le filtre automatique",
                    "Usar filtro automático",
                    "AutoFilter verwenden",
                    "Usa filtro automatico",
                ],
                Permission::PivotTables => [
                    "Use PivotTable and PivotChart",
                    "Utiliser des tableaux croisés dynamiques et PivotChart",
                    "Usar tabla dinámica y gráfico dinámico",
                    "PivotTable und PivotChart verwenden",
                    "Usa tabella pivot e grafico pivot",
                ],
                Permission::EditObjects => [
                    "Edit objects",
                    "Modifier les objets",
                    "Editar objetos",
                    "Objekte bearbeiten",
                    "Modificare oggetti",
                ],
                Permission::EditScenarios => [
                    "Edit scenarios",
                    "Modifier des scénarios",
                    "Editar escenarios",
                    "Szenarien bearbeiten",
                    "Modificare scenari",
                ],
            },
        )
    }
}

/// A protected sheet (`<sheetProtection sheet="1" …>`): locked cells
/// cannot be changed, and the permissions say what else is still allowed.
/// A sheet that is not protected has none (`Sheet::protection` is `None`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SheetProtection {
    /// `None`: anyone may unprotect the sheet.
    pub password: Option<PasswordHash>,
    /// One bit per `Permission`, set when the action is allowed.
    allowed: u16,
}

/// What the Protect Sheet dialog first proposes: selecting cells and
/// nothing else.
impl Default for SheetProtection {
    fn default() -> SheetProtection {
        SheetProtection::allowing(&[Permission::SelectLockedCells, Permission::SelectUnlockedCells])
    }
}

impl SheetProtection {
    /// Protection without a password that allows these actions only.
    pub fn allowing(permissions: &[Permission]) -> SheetProtection {
        SheetProtection {
            password: None,
            allowed: permissions.iter().fold(0, |bits, p| bits | 1 << *p as u16),
        }
    }

    /// The protection a `<sheetProtection>` element describes, password
    /// apart. `forbids` gives the value of one of its attributes (`1`
    /// forbids the action), `None` when the element leaves it out.
    pub fn from_attributes(mut forbids: impl FnMut(&str) -> Option<bool>) -> SheetProtection {
        let mut protection = SheetProtection::allowing(&[]);
        for permission in Permission::ALL {
            let allowed =
                forbids(permission.ooxml_name()).map_or(permission.allowed_when_absent(), |forbidden| !forbidden);
            protection.set(permission, allowed);
        }
        protection
    }

    /// The attributes to write, next to `sheet="1"` and the password: for
    /// each permission its name and whether it is forbidden. (Those equal
    /// to what `allowed_when_absent` implies may be left out.)
    pub fn attributes(&self) -> impl Iterator<Item = (&'static str, bool)> + '_ {
        Permission::ALL
            .into_iter()
            .map(|permission| (permission.ooxml_name(), !self.allows(permission)))
    }

    pub fn allows(&self, permission: Permission) -> bool {
        self.allowed & (1 << permission as u16) != 0
    }

    pub fn set(&mut self, permission: Permission, allowed: bool) {
        if allowed {
            self.allowed |= 1 << permission as u16;
        } else {
            self.allowed &= !(1 << permission as u16);
        }
    }
}

/// Protection of the workbook (`<workbookProtection>`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkbookProtection {
    /// `lockStructure`: sheets cannot be added, deleted, renamed, moved,
    /// copied, hidden or shown.
    pub lock_structure: bool,
    /// `lockWindows`: the windows keep their size and position. Kept for
    /// files; nothing here depends on it.
    pub lock_windows: bool,
    /// `None`: anyone may remove the protection.
    pub password: Option<PasswordHash>,
}

impl WorkbookProtection {
    pub fn is_protected(&self) -> bool {
        self.lock_structure || self.lock_windows
    }
}

/// A password that was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasswordError {
    /// Not the password the sheet or the workbook was protected with.
    Wrong,
    /// The password typed again to confirm differs from the first.
    Mismatch,
}

impl PasswordError {
    /// Excel's message for the error.
    pub fn message(self, locale: &Locale) -> &'static str {
        localized(
            locale,
            match self {
                PasswordError::Wrong => [
                    "The password you supplied is not correct. Verify that the CAPS LOCK key is off and be sure to use the correct capitalization.",
                    "Mot de passe non valide. Vérifiez que la touche VERR.MAJ n'est pas activée et que vous respectez la bonne casse.",
                    "La contraseña es incorrecta. Compruebe que la tecla BLOQ MAYÚS está desactivada y que escribió correctamente las mayúsculas y minúsculas del texto.",
                    "Das eingegebene Kennwort ist ungültig. Stellen Sie sicher, dass die FESTSTELLTASTE nicht aktiviert ist und dass Sie die korrekte Groß-/Kleinschreibung verwenden.",
                    "La password fornita non è corretta. Verificare che il tasto BLOC MAIUSC non sia attivato e che le lettere maiuscole siano corrette.",
                ],
                PasswordError::Mismatch => [
                    "Confirmation password is not identical.",
                    "Mot de passe de confirmation différent.",
                    "La contraseña de confirmación no es idéntica.",
                    "Bestätigungseingabe ist nicht identisch.",
                    "La password di conferma non è identica.",
                ],
            },
        )
    }
}

// ---- the question --------------------------------------------------------

/// A range of a protected sheet that may be edited all the same
/// (`<protectedRange>`): by anyone when it has no password, else by who
/// gives its password, once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtectedRange {
    /// Tells the ranges of a session apart, for the passwords given; not
    /// kept in files.
    pub id: u64,
    /// The title, unique on its sheet.
    pub name: String,
    /// The cells (`sqref`).
    pub ranges: Vec<Rect>,
    pub password: Option<PasswordHash>,
    /// The Windows accounts allowed without a password, as files hold
    /// them; kept untouched.
    pub security_descriptor: Option<String>,
}

impl ProtectedRange {
    /// A range with a fresh `id`.
    pub fn new(name: &str, ranges: Vec<Rect>, password: Option<PasswordHash>) -> ProtectedRange {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        ProtectedRange {
            id: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            name: name.to_string(),
            ranges,
            password,
            security_descriptor: None,
        }
    }
}

/// Why a range that may be edited cannot be made or changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeError {
    /// The title is empty or not a valid one.
    BadTitle,
    /// Another range of the sheet has this title.
    TitleTaken,
    /// No cells.
    NoCells,
}

/// What is left of `rect` outside `hole`: up to four rectangles.
fn subtract(rect: Rect, hole: Rect) -> Vec<Rect> {
    if hole.r1 < rect.r0 || hole.r0 > rect.r1 || hole.c1 < rect.c0 || hole.c0 > rect.c1 {
        return vec![rect];
    }
    let mut rest = Vec::new();
    if hole.r0 > rect.r0 {
        rest.push(Rect::new(rect.r0, rect.c0, hole.r0 - 1, rect.c1));
    }
    if hole.r1 < rect.r1 {
        rest.push(Rect::new(hole.r1 + 1, rect.c0, rect.r1, rect.c1));
    }
    let (top, bottom) = (rect.r0.max(hole.r0), rect.r1.min(hole.r1));
    if hole.c0 > rect.c0 {
        rest.push(Rect::new(top, rect.c0, bottom, hole.c0 - 1));
    }
    if hole.c1 < rect.c1 {
        rest.push(Rect::new(top, hole.c1 + 1, bottom, rect.c1));
    }
    rest
}

impl Workbook {
    /// Whether a range that may be edited is open: it has no password, or
    /// its password was given.
    fn range_open(&self, range: &ProtectedRange) -> bool {
        range.password.is_none() || self.unlocked_ranges.contains(&range.id)
    }

    /// Whether every cell of `rect` on a protected sheet can be changed:
    /// it is unlocked, or inside an open range.
    fn editable(&self, sheet: &Sheet, rect: Rect) -> bool {
        if self.all_unlocked(sheet, rect) {
            return true;
        }
        let mut rest = vec![rect];
        for range in sheet.protected_ranges.iter().filter(|range| self.range_open(range)) {
            for open in &range.ranges {
                rest = rest.into_iter().flat_map(|piece| subtract(piece, *open)).collect();
            }
        }
        rest.len() < 64 && rest.iter().all(|piece| self.all_unlocked(sheet, *piece))
    }

    /// The range whose password would let `rects` be edited: the first
    /// one still closed that holds some of those cells, when the sheet is
    /// protected and refuses the edit.
    pub fn range_awaiting_password(&self, sheet: usize, rects: &[Rect]) -> Option<usize> {
        let sh = self.sheets.get(sheet)?;
        sh.protection.as_ref()?;
        if rects.iter().all(|rect| self.editable(sh, *rect)) {
            return None;
        }
        sh.protected_ranges
            .iter()
            .position(|range| !self.range_open(range) && range.ranges.iter().any(|open| rects.iter().any(|rect| open.intersects(rect))))
    }

    /// Give the password of a range: it stays open until the workbook is
    /// closed. Not an undo step.
    pub fn unlock_range(&mut self, sheet: usize, index: usize, password: &str) -> Result<(), PasswordError> {
        let range = self.sheets.get(sheet).and_then(|sh| sh.protected_ranges.get(index)).ok_or(PasswordError::Wrong)?;
        if range.password.as_ref().is_some_and(|hash| !hash.verify(password)) {
            return Err(PasswordError::Wrong);
        }
        if !self.unlocked_ranges.contains(&range.id) {
            self.unlocked_ranges.push(range.id);
        }
        Ok(())
    }

    /// A title no range of `sheet` has yet: "Range1", "Range2"…
    pub fn protected_range_title(&self, sheet: usize, base: &str) -> String {
        let taken = |title: &str| self.sheets.get(sheet).is_some_and(|sh| sh.protected_ranges.iter().any(|range| range.name.eq_ignore_ascii_case(title)));
        (1..).map(|n| format!("{base}{n}")).find(|title| !taken(title)).unwrap_or_default()
    }

    fn check_protected_range(&self, sheet: usize, range: &ProtectedRange, except: Option<usize>) -> Result<(), RangeError> {
        // What Excel accepts as a title: letters, digits and underscores,
        // not starting with a digit.
        let valid = range.name.chars().next().is_some_and(|ch| ch.is_alphabetic() || ch == '_') && range.name.chars().all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '.');
        if !valid {
            return Err(RangeError::BadTitle);
        }
        if range.ranges.is_empty() {
            return Err(RangeError::NoCells);
        }
        let key = range.name.to_lowercase();
        let taken = self.sheets.get(sheet).is_some_and(|sh| sh.protected_ranges.iter().enumerate().any(|(index, other)| Some(index) != except && other.name.to_lowercase() == key));
        if taken { Err(RangeError::TitleTaken) } else { Ok(()) }
    }

    /// Whether `range` could be added to `sheet`, or replace the one at
    /// `except`.
    pub fn protected_range_valid(&self, sheet: usize, range: &ProtectedRange, except: Option<usize>) -> Result<(), RangeError> {
        self.check_protected_range(sheet, range, except)
    }

    /// Replace the ranges of `sheet` that may be edited (the list of the
    /// Allow Edit Ranges dialog, confirmed). The caller commits.
    pub fn set_protected_ranges(&mut self, sheet: usize, ranges: Vec<ProtectedRange>) {
        if sheet < self.sheets.len() {
            self.snapshot();
            self.sheets[sheet].protected_ranges = ranges;
        }
    }
}

/// Something the user is about to do, as far as protection cares.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action<'a> {
    /// Change what cells hold: typing, clearing, pasting, filling,
    /// moving cells away or onto them.
    EditCells {
        sheet: usize,
        ranges: &'a [Rect],
    },
    /// Change the formatting of cells. (The Locked and Hidden flags
    /// themselves cannot be changed on a protected sheet: `ChangeSheet`.)
    FormatCells {
        sheet: usize,
    },
    /// Change column widths, hide or show columns.
    FormatColumns {
        sheet: usize,
    },
    /// Change row heights, hide or show rows.
    FormatRows {
        sheet: usize,
    },
    InsertColumns {
        sheet: usize,
    },
    InsertRows {
        sheet: usize,
    },
    InsertHyperlink {
        sheet: usize,
        ranges: &'a [Rect],
    },
    /// Delete the columns `first..=last`.
    DeleteColumns {
        sheet: usize,
        first: u32,
        last: u32,
    },
    /// Delete the rows `first..=last`.
    DeleteRows {
        sheet: usize,
        first: u32,
        last: u32,
    },
    Sort {
        sheet: usize,
        range: Rect,
    },
    /// Choose what a filter shows with its drop-down buttons.
    UseAutoFilter {
        sheet: usize,
    },
    /// Put the filter buttons on a range or take them off.
    ToggleAutoFilter {
        sheet: usize,
    },
    /// Change, refresh or create a pivot table.
    UsePivotTable {
        sheet: usize,
    },
    /// Add, change, move or delete a chart, a picture or a shape.
    /// `locked` is the object's own lock (true for one being added).
    EditObject {
        sheet: usize,
        locked: bool,
    },
    /// Add, change or delete a note or a comment.
    EditComment {
        sheet: usize,
    },
    EditScenarios {
        sheet: usize,
    },
    /// Create, resize or restyle a table.
    ChangeTable {
        sheet: usize,
    },
    /// Anything else that changes a sheet and that protection never
    /// allows: merging cells, locking or unlocking them, data validation,
    /// conditional formats, outlines, the tab colour.
    ChangeSheet {
        sheet: usize,
    },
    InsertSheet,
    DeleteSheet,
    RenameSheet,
    MoveSheet,
    CopySheet,
    HideSheet,
    UnhideSheet,
}

/// Why an action may not proceed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Denied {
    /// The sheet is protected.
    Sheet,
    /// The structure of the workbook is protected.
    Workbook,
    /// The rows (or the columns) to delete hold locked cells.
    LockedCells { rows: bool },
    /// Filters are not available on the protected sheet.
    Filter,
    /// Notes and comments of the protected sheet cannot be changed.
    Comment,
    /// Tables of the protected sheet cannot be changed.
    Table,
}

impl Denied {
    /// What Excel tells the user.
    pub fn message(self, locale: &Locale) -> String {
        let text = |texts: [&'static str; 5]| localized(locale, texts).to_string();
        match self {
            Denied::Sheet => text([
                "The cell or chart you're trying to change is on a protected sheet. To make a change, unprotect the sheet. You might be requested to enter a password.",
                "La cellule ou le graphique que vous essayez de modifier se trouve sur une feuille protégée. Pour apporter une modification à celui-ci, vous devez ôter la protection de la feuille. Vous serez peut-être invité à entrer un mot de passe.",
                "La celda o el gráfico que intenta cambiar están en una hoja protegida. Para realizar un cambio, desproteja la hoja. Es posible que se le pida que escriba una contraseña.",
                "Die Zelle oder das Diagramm, die bzw. das Sie ändern möchten, befindet sich auf einem schreibgeschützten Blatt. Um eine Änderung vorzunehmen, heben Sie den Schutz des Blatts auf. Möglicherweise werden Sie aufgefordert, ein Kennwort einzugeben.",
                "La cella o il grafico che si sta provando a modificare si trova in un foglio protetto. Per apportare una modifica, rimuovere la protezione dal foglio. Potrebbe essere richiesta l'immissione di una password.",
            ]),
            Denied::Workbook => text([
                "Workbook is protected and cannot be changed.",
                "Ce classeur est protégé et ne peut pas être modifié.",
                "El libro está protegido y no se puede modificar.",
                "Arbeitsmappe ist geschützt und kann nicht geändert werden.",
                "Impossibile modificare una cartella di lavoro protetta.",
            ]),
            Denied::LockedCells { rows } => {
                let what = if rows {
                    ["a row", "une ligne", "una fila", "eine Zeile", "una riga"]
                } else {
                    ["a column", "une colonne", "una columna", "eine Spalte", "una colonna"]
                };
                text([
                    "You are trying to delete | that contains a locked cell. Locked cells cannot be deleted while the worksheet is protected.\n\nTo delete a locked cell, first remove protection using the Unprotect Sheet command (Review tab, Protect group). You may be prompted for a password.",
                    "Vous essayez de supprimer | qui contient une cellule verrouillée. Les cellules verrouillées ne peuvent pas être supprimées tant que la feuille de calcul est protégée.\n\nPour supprimer une cellule verrouillée, supprimez d’abord la protection à l’aide de la commande ôter la protection de la feuille (onglet révision, protéger le groupe). Vous devrez peut-être entrer un mot de passe.",
                    "Está intentando eliminar | que contenga una celda bloqueada. Las celdas bloqueadas no se pueden eliminar mientras la hoja de cálculo esté protegida.\n\nPara eliminar una celda bloqueada, elimine primero la protección con el comando Desproteger hoja (ficha revisar, proteger grupo). Puede que se le pida una contraseña.",
                    "Sie versuchen, | zu löschen. die eine gesperrte Zelle enthält. Gesperrte Zellen können nicht gelöscht werden, während das Arbeitsblatt geschützt ist.\n\nUm eine gesperrte Zelle zu löschen, entfernen Sie zuerst den Schutz mit dem Befehl \"Schutz des Blatts aufheben\" (Registerkarte \"Überprüfen\", Gruppe \"Schützen\"). Möglicherweise werden Sie zur Eingabe eines Kennworts aufgefordert.",
                    "Si sta tentando di eliminare | contenente una cella bloccata. Impossibile eliminare le celle bloccate mentre il foglio di lavoro è protetto.\n\nPer eliminare una cella bloccata, rimuovere prima la protezione utilizzando il comando Rimuovi protezione foglio (scheda Revisione, gruppo Proteggi). È possibile che venga richiesta una password.",
                ])
                .replace('|', localized(locale, what))
            }
            Denied::Filter => text([
                "You can't use filters on this protected sheet. To remove protection, click the Unprotect Sheet button on the Review tab.",
                "Désolé... Vous ne pouvez pas utiliser de filtres sur cette feuille protégée. Pour enlever la protection, cliquez sur Ôter la protection de la feuille sous l’onglet Révision.",
                "No puede usar filtros en esta hoja protegida. Para quitar la protección, haga clic en el botón Desproteger hoja de la pestaña Revisar.",
                "Sie können auf dieses geschützte Blatt keine Filter anwenden. Klicken Sie auf der Registerkarte \"Überprüfen\" auf die Schaltfläche \"Blattschutz aufheben\", um den Schutz aufzuheben.",
                "Non è possibile usare filtri in un foglio protetto. Per rimuovere la protezione, fare clic sul pulsante Rimuovi protezione foglio nella scheda Revisione.",
            ]),
            Denied::Comment => text([
                "This comment is on a protected sheet. To make a change, unprotect the sheet. You might be requested to enter a password.",
                "Ce commentaire se trouve sur une feuille protégée. Pour y apporter une modification, vous devez ôter la protection de la feuille. Vous serez peut-être invité à entrer un mot de passe.",
                "Este comentario está en una hoja protegida. Para realizar un cambio, desproteja la hoja. Es posible que se le pida que escriba una contraseña.",
                "Dieser Kommentar befindet sich auf einem schreibgeschützten Blatt. Um eine Änderung vorzunehmen, heben Sie den Schutz des Blatts auf. Möglicherweise werden Sie aufgefordert, ein Kennwort einzugeben.",
                "Questo commento è stato aggiunto in un foglio protetto. Per apportare una modifica, rimuovere la protezione dal foglio. Potrebbe essere richiesto di immettere una password.",
            ]),
            Denied::Table => text([
                "Table features aren't available because the sheet is protected.",
                "Les fonctionnalités du tableau ne sont pas disponibles, car la feuille est protégée.",
                "Las características de la tabla no están disponibles porque la hoja está protegida.",
                "Tabellenfunktionen sind nicht verfügbar, weil das Blatt geschützt ist.",
                "Le caratteristiche tabella non sono disponibili perché il foglio è protetto.",
            ]),
        }
    }
}

impl Workbook {
    /// Whether the Locked flag of every cell of `rect` is `locked`. Cells
    /// that were never touched take the format of their row, else of their
    /// column, else the default one, which is locked.
    fn all_locked_as(&self, sheet: &Sheet, rect: Rect, locked: bool) -> bool {
        let fits = |id: StyleId| self.styles.get(id).locked == locked;
        // Rows with a format of their own: (fits, cells present).
        let mut styled: BTreeMap<u32, (bool, u32)> = sheet
            .row_styles
            .range(rect.r0..=rect.r1)
            .map(|(row, id)| (*row, (fits(*id), 0)))
            .collect();
        // Cells present per column in the other rows.
        let mut plain: HashMap<u32, u32> = HashMap::new();
        for ((row, col), cell) in sheet.cells.range((rect.r0, 0)..=(rect.r1, u32::MAX)) {
            if *col < rect.c0 || *col > rect.c1 {
                continue;
            }
            if !fits(cell.style) {
                return false;
            }
            match styled.get_mut(row) {
                Some((_, present)) => *present += 1,
                None => *plain.entry(*col).or_default() += 1,
            }
        }
        if styled
            .values()
            .any(|(fitting, present)| !fitting && *present < rect.cols())
        {
            return false;
        }
        let plain_rows = rect.rows() - styled.len() as u32;
        let default_fits = fits(StyleId::default());
        plain_rows == 0
            || (rect.c0..=rect.c1).all(|col| {
                sheet.col_styles.get(&col).map_or(default_fits, |id| fits(*id))
                    || plain.get(&col).copied().unwrap_or(0) == plain_rows
            })
    }

    /// Whether no cell of `rect` is locked.
    fn all_unlocked(&self, sheet: &Sheet, rect: Rect) -> bool {
        self.all_locked_as(sheet, rect, false)
    }

    /// May this action proceed? `Err` says why not; its message is the
    /// one to show. Sheets that are not protected allow everything, and
    /// sheet protection says nothing about the sheet as a whole (it can
    /// be renamed or deleted unless the workbook structure is protected).
    pub fn permits(&self, action: &Action) -> Result<(), Denied> {
        use Permission as P;
        let sheet_index = match *action {
            Action::InsertSheet
            | Action::DeleteSheet
            | Action::RenameSheet
            | Action::MoveSheet
            | Action::CopySheet
            | Action::HideSheet
            | Action::UnhideSheet => {
                return if self.protection.lock_structure {
                    Err(Denied::Workbook)
                } else {
                    Ok(())
                };
            }
            Action::EditCells { sheet, .. }
            | Action::FormatCells { sheet }
            | Action::FormatColumns { sheet }
            | Action::FormatRows { sheet }
            | Action::InsertColumns { sheet }
            | Action::InsertRows { sheet }
            | Action::InsertHyperlink { sheet, .. }
            | Action::DeleteColumns { sheet, .. }
            | Action::DeleteRows { sheet, .. }
            | Action::Sort { sheet, .. }
            | Action::UseAutoFilter { sheet }
            | Action::ToggleAutoFilter { sheet }
            | Action::UsePivotTable { sheet }
            | Action::EditObject { sheet, .. }
            | Action::EditComment { sheet }
            | Action::EditScenarios { sheet }
            | Action::ChangeTable { sheet }
            | Action::ChangeSheet { sheet } => sheet,
        };
        let Some(sh) = self.sheets.get(sheet_index) else {
            return Ok(());
        };
        let Some(protection) = &sh.protection else {
            return Ok(());
        };
        let unlocked = |ranges: &[Rect]| ranges.iter().all(|r| self.editable(sh, *r));
        let when = |allowed: bool, denied: Denied| if allowed { Ok(()) } else { Err(denied) };
        match *action {
            Action::EditCells { ranges, .. } => when(unlocked(ranges), Denied::Sheet),
            Action::FormatCells { .. } => when(protection.allows(P::FormatCells), Denied::Sheet),
            Action::FormatColumns { .. } => when(protection.allows(P::FormatColumns), Denied::Sheet),
            Action::FormatRows { .. } => when(protection.allows(P::FormatRows), Denied::Sheet),
            Action::InsertColumns { .. } => when(protection.allows(P::InsertColumns), Denied::Sheet),
            Action::InsertRows { .. } => when(protection.allows(P::InsertRows), Denied::Sheet),
            // Hyperlinks go into cells: the cells must be unlocked too.
            Action::InsertHyperlink { ranges, .. } => when(
                protection.allows(P::InsertHyperlinks) && unlocked(ranges),
                Denied::Sheet,
            ),
            // Even with the permission, locked cells cannot be deleted
            // or sorted.
            Action::DeleteColumns { first, last, .. } => {
                when(protection.allows(P::DeleteColumns), Denied::Sheet)?;
                let columns = Rect::new(0, first, MAX_ROWS - 1, last);
                when(unlocked(&[columns]), Denied::LockedCells { rows: false })
            }
            Action::DeleteRows { first, last, .. } => {
                when(protection.allows(P::DeleteRows), Denied::Sheet)?;
                let rows = Rect::new(first, 0, last, MAX_COLS - 1);
                when(unlocked(&[rows]), Denied::LockedCells { rows: true })
            }
            Action::Sort { range, .. } => when(protection.allows(P::Sort) && unlocked(&[range]), Denied::Sheet),
            Action::UseAutoFilter { .. } => when(protection.allows(P::AutoFilter), Denied::Filter),
            Action::ToggleAutoFilter { .. } => Err(Denied::Filter),
            Action::UsePivotTable { .. } => when(protection.allows(P::PivotTables), Denied::Sheet),
            Action::EditObject { locked, .. } => when(protection.allows(P::EditObjects) || !locked, Denied::Sheet),
            Action::EditComment { .. } => when(protection.allows(P::EditObjects), Denied::Comment),
            Action::EditScenarios { .. } => when(protection.allows(P::EditScenarios), Denied::Sheet),
            Action::ChangeTable { .. } => Err(Denied::Table),
            _ => Err(Denied::Sheet),
        }
    }

    /// Whether a cell may be selected (clicked, reached with the keyboard).
    pub fn can_select_cell(&self, sheet: usize, row: u32, col: u32) -> bool {
        let Some(protection) = self.sheets.get(sheet).and_then(|sh| sh.protection.as_ref()) else {
            return true;
        };
        if self.style_at(sheet, row, col).locked {
            protection.allows(Permission::SelectLockedCells)
        } else {
            protection.allows(Permission::SelectUnlockedCells)
        }
    }

    /// Whether a whole range may be selected: every cell of it may be.
    pub fn can_select_range(&self, sheet: usize, rect: Rect) -> bool {
        let Some(sh) = self.sheets.get(sheet) else {
            return true;
        };
        let Some(protection) = &sh.protection else {
            return true;
        };
        match (
            protection.allows(Permission::SelectLockedCells),
            protection.allows(Permission::SelectUnlockedCells),
        ) {
            (true, true) => true,
            (false, false) => false,
            (false, true) => self.all_locked_as(sh, rect, false),
            (true, false) => self.all_locked_as(sh, rect, true),
        }
    }

    /// The first unlocked cell of a row at or after column `from` (at or
    /// before it when going backwards).
    fn unlocked_in_row(&self, sheet: &Sheet, row: u32, from: u32, forward: bool) -> Option<u32> {
        let open = |id: StyleId| !self.styles.get(id).locked;
        // A cell of the row that exists has its own format.
        let own = |col: u32| sheet.cells.get(&(row, col)).map(|cell| open(cell.style));
        let typed = if forward {
            sheet.cells.range((row, from)..=(row, MAX_COLS - 1))
        } else {
            sheet.cells.range((row, 0)..=(row, from))
        };
        let pick = |mut cols: Box<dyn DoubleEndedIterator<Item = u32> + '_>| {
            if forward { cols.next() } else { cols.next_back() }
        };
        let typed_open = pick(Box::new(
            typed.filter(|(_, cell)| open(cell.style)).map(|((_, col), _)| *col),
        ));
        match sheet.row_styles.get(&row) {
            // An unlocked row: the first cell that is not locked by itself.
            Some(id) if open(*id) => {
                let cols: Box<dyn DoubleEndedIterator<Item = u32>> = if forward {
                    Box::new(from..MAX_COLS)
                } else {
                    Box::new(0..=from)
                };
                pick(Box::new(cols.filter(|col| own(*col).unwrap_or(true))))
            }
            Some(_) => typed_open,
            None => {
                let styled = if forward {
                    sheet.col_styles.range(from..)
                } else {
                    sheet.col_styles.range(..=from)
                };
                let column_open = pick(Box::new(
                    styled
                        .filter(|(col, id)| open(**id) && own(**col).unwrap_or(true))
                        .map(|(col, _)| *col),
                ));
                match (typed_open, column_open) {
                    (Some(a), Some(b)) => Some(if forward { a.min(b) } else { a.max(b) }),
                    (a, b) => a.or(b),
                }
            }
        }
    }

    /// The unlocked cell that follows `from` in reading order (or comes
    /// before it), going round the sheet: where Tab and Shift+Tab lead on
    /// a protected sheet. `None` when no other cell is unlocked.
    pub fn next_unlocked_cell(&self, sheet: usize, from: (u32, u32), forward: bool) -> Option<(u32, u32)> {
        let sh = self.sheets.get(sheet)?;
        let (row, col) = from;
        let rest = if forward {
            (col + 1 < MAX_COLS).then_some(col + 1)
        } else {
            col.checked_sub(1)
        };
        if let Some(found) = rest.and_then(|start| self.unlocked_in_row(sh, row, start, forward)) {
            return Some((row, found));
        }
        // With an unlocked column every row may hold an unlocked cell;
        // otherwise only rows with cells or a format of their own do.
        let any_column = sh.col_styles.values().any(|id| !self.styles.get(*id).locked);
        let following = |after: Option<u32>| -> Option<u32> {
            if forward {
                let start = after.map_or(0, |row| row + 1);
                if start >= MAX_ROWS {
                    return None;
                }
                if any_column {
                    return Some(start);
                }
                let typed = sh.cells.range((start, 0)..).next().map(|((row, _), _)| *row);
                let styled = sh.row_styles.range(start..).next().map(|(row, _)| *row);
                typed.into_iter().chain(styled).min()
            } else {
                let end = match after {
                    Some(0) => return None,
                    Some(row) => row - 1,
                    None => MAX_ROWS - 1,
                };
                if any_column {
                    return Some(end);
                }
                let typed = sh.cells.range(..=(end, u32::MAX)).next_back().map(|((row, _), _)| *row);
                let styled = sh.row_styles.range(..=end).next_back().map(|(row, _)| *row);
                typed.into_iter().chain(styled).max()
            }
        };
        let edge = if forward { 0 } else { MAX_COLS - 1 };
        let mut at = Some(row);
        let mut wrapped = false;
        loop {
            let next = match following(at) {
                Some(next) => next,
                None if !wrapped => {
                    wrapped = true;
                    at = None;
                    continue;
                }
                None => return None,
            };
            // Back where it started: only the cells before `from` are left.
            if wrapped && (if forward { next > row } else { next < row }) {
                return None;
            }
            if let Some(found) = self.unlocked_in_row(sh, next, edge, forward) {
                return ((next, found) != from).then_some((next, found));
            }
            at = Some(next);
        }
    }

    /// Whether the content of a cell is kept out of the formula bar: its
    /// format says Hidden and its sheet is protected.
    pub fn is_formula_hidden(&self, sheet: usize, row: u32, col: u32) -> bool {
        self.is_sheet_protected(sheet) && self.style_at(sheet, row, col).hidden
    }

    /// What the formula bar shows for a cell: `edit_text`, or nothing for
    /// a hidden cell of a protected sheet.
    pub fn formula_bar_text(&self, sheet: usize, row: u32, col: u32) -> String {
        if self.is_formula_hidden(sheet, row, col) {
            String::new()
        } else if let Some(table) = self.data_table_text(sheet, row, col) {
            // A result of a data table reads as Excel's array formula.
            table
        } else {
            self.edit_text(sheet, row, col)
        }
    }

    pub fn is_sheet_protected(&self, sheet: usize) -> bool {
        self.sheets.get(sheet).is_some_and(|sh| sh.protection.is_some())
    }

    /// Whether unprotecting the sheet takes a password.
    pub fn sheet_needs_password(&self, sheet: usize) -> bool {
        self.sheets
            .get(sheet)
            .and_then(|sh| sh.protection.as_ref())
            .is_some_and(|p| p.password.is_some())
    }

    /// Protect a sheet with the given permissions; an empty password
    /// means none (the one in `protection` is replaced either way).
    /// Like Excel, this cannot be undone and empties the undo history.
    pub fn protect_sheet(&mut self, sheet: usize, mut protection: SheetProtection, password: &str) -> bool {
        if sheet >= self.sheets.len() {
            return false;
        }
        protection.password = (!password.is_empty()).then(|| PasswordHash::new(password));
        self.sheets[sheet].protection = Some(protection);
        self.forget_history();
        true
    }

    /// Remove the protection of a sheet. Cannot be undone either.
    pub fn unprotect_sheet(&mut self, sheet: usize, password: &str) -> Result<(), PasswordError> {
        let Some(protection) = self.sheets.get(sheet).and_then(|sh| sh.protection.as_ref()) else {
            return Ok(());
        };
        if protection.password.as_ref().is_some_and(|hash| !hash.verify(password)) {
            return Err(PasswordError::Wrong);
        }
        self.sheets[sheet].protection = None;
        self.forget_history();
        Ok(())
    }

    /// Protect a sheet (`Some`) or remove its protection (`None`) as a
    /// part of the current transaction: what a script does, whose changes
    /// are all dropped when it fails. A rollback brings the protection
    /// back with the rest; a commit keeps it and forgets the history, as
    /// `protect_sheet` and `unprotect_sheet` forget it at once. The
    /// password of `protection` is kept as it is given, and no password
    /// is asked to remove one: the caller has checked it.
    pub fn set_sheet_protection(&mut self, sheet: usize, protection: Option<SheetProtection>) -> bool {
        if sheet >= self.sheets.len() {
            return false;
        }
        if self.sheets[sheet].protection == protection {
            return true;
        }
        self.snapshot();
        self.sheets[sheet].protection = protection;
        self.protection_pending = true;
        true
    }

    /// Whether removing the workbook protection takes a password.
    pub fn workbook_needs_password(&self) -> bool {
        self.protection.is_protected() && self.protection.password.is_some()
    }

    /// Protect the structure and / or the windows of the workbook; an
    /// empty password means none. Cannot be undone.
    pub fn protect_workbook(&mut self, structure: bool, windows: bool, password: &str) {
        self.protection = WorkbookProtection {
            lock_structure: structure,
            lock_windows: windows,
            password: (!password.is_empty()).then(|| PasswordHash::new(password)),
        };
        self.forget_history();
    }

    /// Remove the protection of the workbook.
    pub fn unprotect_workbook(&mut self, password: &str) -> Result<(), PasswordError> {
        if self
            .protection
            .password
            .as_ref()
            .is_some_and(|hash| !hash.verify(password))
        {
            return Err(PasswordError::Wrong);
        }
        self.protection = WorkbookProtection::default();
        self.forget_history();
        Ok(())
    }

    /// Protection is not part of what undo restores on purpose, but the
    /// snapshots of the history hold whole sheets, protection included:
    /// going back to one would silently undo it. Excel empties the
    /// history at this point too.
    fn forget_history(&mut self) {
        self.clear_history();
        self.modified = true;
    }
}

/// See `features::on_structural_edit`: protection covers the whole sheet,
/// but the ranges that may be edited follow the rows and columns of
/// theirs; one whose cells are all deleted is forgotten.
pub(crate) fn on_structural_edit(sheet: &mut Sheet, edit: StructuralEdit, edited_sheet: &str) {
    if sheet.name != edited_sheet {
        return;
    }
    sheet.protected_ranges.retain_mut(|range| {
        range.ranges.retain_mut(|rect| {
            let mut moved = crate::filter::AutoFilter::new(*rect);
            let kept = moved.adjust(edit);
            *rect = moved.range;
            kept
        });
        !range.ranges.is_empty()
    });
}

/// See `features::on_sheet_renamed`: protection names no sheet.
pub(crate) fn on_sheet_renamed(_sheet: &mut Sheet, _old: &str, _new: &str) {}

/// See `features::on_sheet_duplicated`: the copy of a protected sheet is
/// protected with the same password, which the clone already is.
pub(crate) fn on_sheet_duplicated(_wb: &mut Workbook, _copy: usize, _original: &str) {}

/// See `features::on_sheet_deleted`: the protection of a sheet goes with
/// it (whether the sheet may be deleted is asked before, see `permits`).
pub(crate) fn on_sheet_deleted(_wb: &mut Workbook, _deleted: &Sheet, _order: &[&str]) {}

/// See `features::on_sheet_moved`: protection is kept on the sheet.
pub(crate) fn on_sheet_moved(_wb: &mut Workbook, _moved: &str, _before: &[&str], _after: &[&str]) {}

/// See `features::on_cells_shifted`: protection covers the whole sheet,
/// and whether a cell is locked is part of its format, which moved with it.
pub(crate) fn on_cells_shifted(_wb: &mut Workbook, _sheet: usize, _shift: CellShift) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::wb;

    fn rect(name: &str) -> Rect {
        Rect::parse(name).unwrap()
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn digests() {
        let long = vec![b'a'; 200];
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(hex(&sha1(b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(hex(&sha1(&long)), "e61cfffe0d9195a525fc6cf06ca2d77119c24a40");
        assert_eq!(
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            hex(&sha256(&long)),
            "c2a908d98f5df987ade41b5fce213067efbcc21ef2240212a41e54b5e7c28ae5"
        );
        assert_eq!(
            hex(&sha384(b"abc")),
            "cb00753f45a35e8bb5a03d699ac65007272c32ab0eded1631a8b605a43ff5bed8086072ba1e7cc2358baeca134c825a7"
        );
        assert_eq!(
            hex(&sha384(&long)),
            "0691b6e978614b67d60557b2a2cddd53406508522efa21c624dbbfa8ab6e726d5c586b489c7c09f24109a64c10211d48"
        );
        assert_eq!(
            hex(&sha512(b"")),
            "cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e"
        );
        assert_eq!(
            hex(&sha512(b"abc")),
            "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
        );
        assert_eq!(
            hex(&sha512(&long)),
            "4b11459c33f52a22ee8236782714c150a3b2c60994e9acee17fe68947a3e6789f31e7668394592da7bef827cddca88c4e6f86e4df7ed1ae6cba71f3e98faee9f"
        );
    }

    #[test]
    fn base64() {
        for (plain, coded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64_encode(plain.as_bytes()), coded);
            assert_eq!(base64_decode(coded).as_deref(), Some(plain.as_bytes()));
        }
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(base64_decode(&base64_encode(&all)), Some(all));
        assert_eq!(base64_decode("Zm9v\nYmFy"), Some(b"foobar".to_vec()));
        assert_eq!(base64_decode("Zm9*"), None);
        assert_eq!(base64_decode("Z"), None);
    }

    #[test]
    fn legacy_passwords() {
        // The hashes Excel writes for these passwords.
        assert_eq!(legacy_hash("password"), 0x83AF);
        assert_eq!(legacy_hash("test"), 0xCBEB);
        assert_eq!(legacy_hash("abcdefghij"), 0xFEF1);
        assert_eq!(legacy_hash("1"), 0xCE28);
        // Accents and the euro sign count as their Windows bytes.
        assert_eq!(legacy_hash("é€"), 0xCD9B);
        let hash = PasswordHash::legacy("test");
        assert_eq!(hash, PasswordHash::Legacy(0xCBEB));
        assert!(hash.verify("test"));
        assert!(!hash.verify("Test"));
    }

    #[test]
    fn salted_passwords() {
        let salt: Vec<u8> = (0..16).collect();
        let stored = |algorithm: &str, hash: &str| PasswordHash::Salted {
            algorithm: algorithm.to_string(),
            hash: hash.to_string(),
            salt: "AAECAwQFBgcICQoLDA0ODw==".to_string(),
            spin_count: 100_000,
        };
        // Reference values computed with another implementation of the
        // scheme (salt, then UTF-16 password; the round number appended).
        let sha512 = stored(
            "SHA-512",
            "mmBHtmwi+QU5pb8+NdoZW9VrYJskpMZTAyO4EyEVfXYhfS7DhrLwjD2t24tinBr0U4S0hSw2wOF+7wVQp2j8PA==",
        );
        assert_eq!(PasswordHash::salted("Sésame 1", &salt, 100_000), sha512);
        assert!(sha512.verify("Sésame 1"));
        assert!(!sha512.verify("Sesame 1"));
        assert!(!sha512.verify(""));
        assert!(
            stored(
                "SHA-384",
                "R+2S52eDsEybwfBmdC+E76XqDUwyRRYFbrkeFPFo3XrKb+hLaHsHjN+BoIetQJFU"
            )
            .verify("Sésame 1")
        );
        assert!(stored("SHA-256", "xns+C6nFWjrxFfu9RjvjRa32OWN8xwcNAW8jmqlel1A=").verify("Sésame 1"));
        assert!(stored("SHA-1", "tMk2uy8ay0kixpP6kGG3Og2BVx4=").verify("Sésame 1"));
        assert!(!stored("MD5", "tMk2uy8ay0kixpP6kGG3Og2BVx4=").verify("Sésame 1"));
        assert!(!stored("SHA-512", "not base64 !").verify("Sésame 1"));
        let one_round = PasswordHash::salted("abc", &salt, 1);
        let PasswordHash::Salted { hash, .. } = &one_round else {
            panic!("not salted");
        };
        assert_eq!(
            hash,
            "mp/PSGZ8RNlC+0EBEbnJOe+9wA/jTysUly1FpY9/+EKx278kUhkkOGzY3U+LvLDLWe1bSoxWpWaehCJmhiTEuw=="
        );
        // Fresh hashes are salted differently every time and verify.
        let (a, b) = (PasswordHash::new("abc"), PasswordHash::new("abc"));
        assert_ne!(a, b);
        assert!(a.verify("abc") && b.verify("abc") && !a.verify("abd"));
        assert!(matches!(a, PasswordHash::Salted { ref algorithm, spin_count: 100_000, .. } if algorithm == "SHA-512"));
    }

    #[test]
    fn permissions() {
        let mut p = SheetProtection::default();
        assert!(p.allows(Permission::SelectLockedCells) && p.allows(Permission::SelectUnlockedCells));
        assert!(Permission::ALL[2..].iter().all(|permission| !p.allows(*permission)));
        p.set(Permission::Sort, true);
        p.set(Permission::SelectLockedCells, false);
        assert!(p.allows(Permission::Sort) && !p.allows(Permission::SelectLockedCells));
        assert_eq!(
            p,
            SheetProtection::allowing(&[Permission::Sort, Permission::SelectUnlockedCells])
        );
        let names: Vec<&str> = Permission::ALL.iter().map(|p| p.ooxml_name()).collect();
        assert_eq!(names[0], "selectLockedCells");
        assert_eq!(names[13..], ["objects", "scenarios"]);
        assert_eq!(Permission::ALL.iter().filter(|p| p.allowed_when_absent()).count(), 4);
        assert_eq!(
            Permission::AutoFilter.label(Locale::fr()),
            "Utiliser le filtre automatique"
        );
        // <sheetProtection sheet="1" objects="1" scenarios="1" formatCells="0" selectLockedCells="1"/>
        let read = SheetProtection::from_attributes(|name| match name {
            "objects" | "scenarios" | "selectLockedCells" => Some(true),
            "formatCells" => Some(false),
            _ => None,
        });
        assert_eq!(
            read,
            SheetProtection::allowing(&[Permission::SelectUnlockedCells, Permission::FormatCells])
        );
        let written: Vec<(&str, bool)> = read.attributes().filter(|(_, forbidden)| !forbidden).collect();
        assert_eq!(written, [("selectUnlockedCells", false), ("formatCells", false)]);
        assert_eq!(
            SheetProtection::from_attributes(|name| read.attributes().find(|a| a.0 == name).map(|a| a.1)),
            read
        );
        // What Excel's dialog proposes is what a file says by naming nothing but objects and scenarios.
        let excel = SheetProtection::from_attributes(|name| matches!(name, "objects" | "scenarios").then_some(true));
        assert_eq!(excel, SheetProtection::default());
    }

    /// A sheet where B2:C3 and the whole column E are unlocked.
    fn form() -> Workbook {
        let mut w = wb(&[("A1", "Title"), ("B2", "1"), ("A5", "=B2*2")]);
        w.update_style(0, rect("B2:C3"), &|s| s.locked = false);
        w.update_style(0, rect("E:E"), &|s| s.locked = false);
        w.update_style(0, rect("A5"), &|s| s.hidden = true);
        w.commit("format");
        w
    }

    #[test]
    fn locked_and_unlocked_cells() {
        let mut w = form();
        let edit = |w: &Workbook, range: &str| {
            w.permits(&Action::EditCells {
                sheet: 0,
                ranges: &[rect(range)],
            })
        };
        // Nothing is refused before the sheet is protected.
        assert_eq!(edit(&w, "A1"), Ok(()));
        assert_eq!(w.formula_bar_text(0, 4, 0), "=B2*2");
        assert!(w.protect_sheet(0, SheetProtection::default(), ""));
        assert!(w.is_sheet_protected(0) && !w.sheet_needs_password(0));
        assert_eq!(edit(&w, "A1"), Err(Denied::Sheet));
        assert_eq!(edit(&w, "B2"), Ok(()));
        assert_eq!(edit(&w, "B2:C3"), Ok(()));
        assert_eq!(edit(&w, "B2:C4"), Err(Denied::Sheet));
        assert_eq!(edit(&w, "D7"), Err(Denied::Sheet));
        // The unlocked column, as a whole and where another format crosses it.
        assert_eq!(edit(&w, "E:E"), Ok(()));
        assert_eq!(edit(&w, "E10:E20"), Ok(()));
        assert_eq!(edit(&w, "D:E"), Err(Denied::Sheet));
        assert_eq!(edit(&w, "2:2"), Err(Denied::Sheet));
        assert_eq!(
            w.permits(&Action::EditCells {
                sheet: 0,
                ranges: &[rect("B2"), rect("E5"), rect("A1")]
            }),
            Err(Denied::Sheet)
        );
        // Hidden formulas stay out of the formula bar while protected.
        assert!(w.is_formula_hidden(0, 4, 0));
        assert_eq!(w.formula_bar_text(0, 4, 0), "");
        assert_eq!(w.formula_bar_text(0, 1, 1), "1");
        assert_eq!(w.value_at(0, 4, 0), crate::Value::Number(2.0));
        // Other sheets are not concerned.
        let other = w.add_sheet(1);
        assert_eq!(
            w.permits(&Action::EditCells {
                sheet: other,
                ranges: &[rect("A1")]
            }),
            Ok(())
        );
        assert_eq!(
            w.permits(&Action::EditCells {
                sheet: 9,
                ranges: &[rect("A1")]
            }),
            Ok(())
        );
        w.unprotect_sheet(0, "").unwrap();
        assert_eq!(edit(&w, "A1"), Ok(()));
        assert_eq!(w.formula_bar_text(0, 4, 0), "=B2*2");
    }

    #[test]
    fn row_formats_decide_for_untouched_cells() {
        let mut w = wb(&[]);
        w.update_style(0, rect("3:4"), &|s| s.locked = false);
        w.update_style(0, rect("B4"), &|s| s.locked = true);
        w.update_style(0, rect("G:G"), &|s| s.locked = false);
        w.commit("format");
        // A row format laid over the column without a cell of its own, as
        // a file may hold it.
        let mut bold = crate::Style::default();
        bold.font.bold = true;
        let bold = w.styles.intern(bold);
        w.sheets[0].row_styles.insert(8, bold);
        w.protect_sheet(0, SheetProtection::default(), "");
        let edit = |range: &str| {
            w.permits(&Action::EditCells {
                sheet: 0,
                ranges: &[rect(range)],
            })
            .is_ok()
        };
        assert!(edit("3:3") && edit("A3:Z3") && edit("C4:F4"));
        assert!(!edit("3:4") && !edit("A4:C4") && !edit("A2:A3"));
        // The bold row is locked, even where it crosses the unlocked column.
        assert!(edit("G1:G8") && !edit("G:G") && !edit("G9") && !edit("A9"));
    }

    #[test]
    fn selecting_ranges() {
        let mut w = form();
        assert!(w.can_select_range(0, rect("A1:Z9")));
        w.protect_sheet(0, SheetProtection::default(), "");
        assert!(w.can_select_range(0, rect("A1:Z9")) && w.can_select_range(0, rect("A:XFD")));
        // Unlocked cells only.
        w.protect_sheet(0, SheetProtection::allowing(&[Permission::SelectUnlockedCells]), "");
        assert!(w.can_select_range(0, rect("B2:C3")) && w.can_select_range(0, rect("E:E")));
        assert!(!w.can_select_range(0, rect("B2:C4")) && !w.can_select_range(0, rect("A1")));
        assert!(!w.can_select_range(0, rect("D:E")) && !w.can_select_range(0, rect("2:2")));
        // Locked cells only, as a file may ask.
        w.protect_sheet(0, SheetProtection::allowing(&[Permission::SelectLockedCells]), "");
        assert!(w.can_select_range(0, rect("A1:A9")) && w.can_select_range(0, rect("F:XFD")));
        assert!(!w.can_select_range(0, rect("A1:B2")) && !w.can_select_range(0, rect("E7")));
        assert!(!w.can_select_range(0, rect("1:1")));
        // Nothing at all.
        w.protect_sheet(0, SheetProtection::allowing(&[]), "");
        assert!(!w.can_select_range(0, rect("B2")) && !w.can_select_range(0, rect("A1")));
        assert!(w.can_select_range(1, rect("A1")));
    }

    #[test]
    fn tab_walks_the_unlocked_cells() {
        let cell = |name: &str| crate::refs::parse_cell_name(name).unwrap();
        let walk = |w: &Workbook, from: &str, forward: bool| {
            w.next_unlocked_cell(0, cell(from), forward)
                .map(|(row, col)| crate::refs::cell_name(row, col))
        };
        // Cells unlocked one by one: reading order, round the sheet.
        let mut w = wb(&[("A1", "Title"), ("D9", "end")]);
        for name in ["B2", "D2", "C5"] {
            w.update_style(0, rect(name), &|s| s.locked = false);
        }
        w.commit("format");
        assert_eq!(walk(&w, "A1", true).as_deref(), Some("B2"));
        assert_eq!(walk(&w, "B2", true).as_deref(), Some("D2"));
        assert_eq!(walk(&w, "D2", true).as_deref(), Some("C5"));
        assert_eq!(walk(&w, "C5", true).as_deref(), Some("B2"));
        assert_eq!(walk(&w, "B2", false).as_deref(), Some("C5"));
        assert_eq!(walk(&w, "C5", false).as_deref(), Some("D2"));
        assert_eq!(walk(&w, "D2", false).as_deref(), Some("B2"));
        // From a locked cell between them, and from below them all.
        assert_eq!(walk(&w, "C2", true).as_deref(), Some("D2"));
        assert_eq!(walk(&w, "C2", false).as_deref(), Some("B2"));
        assert_eq!(walk(&w, "Z99", true).as_deref(), Some("B2"));
        assert_eq!(walk(&w, "Z99", false).as_deref(), Some("C5"));

        // A single unlocked cell leads nowhere from itself, none at all
        // leads nowhere from anywhere.
        let mut one = wb(&[("A1", "x")]);
        assert_eq!(walk(&one, "A1", true), None);
        one.update_style(0, rect("C3"), &|s| s.locked = false);
        one.commit("format");
        assert_eq!(walk(&one, "C3", true), None);
        assert_eq!(walk(&one, "C3", false), None);
        assert_eq!(walk(&one, "A1", true).as_deref(), Some("C3"));
        assert_eq!(walk(&one, "D8", false).as_deref(), Some("C3"));

        // Unlocked rows and columns, with locked cells of their own.
        let w = form();
        assert_eq!(walk(&w, "C2", true).as_deref(), Some("E2"));
        assert_eq!(walk(&w, "E2", true).as_deref(), Some("B3"));
        assert_eq!(walk(&w, "E3", true).as_deref(), Some("E4"));
        assert_eq!(walk(&w, "E4", false).as_deref(), Some("E3"));
        assert_eq!(walk(&w, "B2", false).as_deref(), Some("E1"));
        assert_eq!(walk(&w, "E1", false).as_deref(), Some("E1048576"));
        let mut rows = wb(&[]);
        rows.update_style(0, rect("3:3"), &|s| s.locked = false);
        rows.update_style(0, rect("B3:C3"), &|s| s.locked = true);
        rows.commit("format");
        assert_eq!(walk(&rows, "A3", true).as_deref(), Some("D3"));
        assert_eq!(walk(&rows, "D3", false).as_deref(), Some("A3"));
        assert_eq!(walk(&rows, "XFD3", true).as_deref(), Some("A3"));
        assert_eq!(walk(&rows, "A3", false).as_deref(), Some("XFD3"));
    }

    #[test]
    fn every_option() {
        let mut w = form();
        let rows = Action::DeleteRows {
            sheet: 0,
            first: 1,
            last: 2,
        };
        let columns = Action::DeleteColumns {
            sheet: 0,
            first: 4,
            last: 4,
        };
        let sort_open = Action::Sort {
            sheet: 0,
            range: rect("B2:C3"),
        };
        let sort_locked = Action::Sort {
            sheet: 0,
            range: rect("A1:C3"),
        };
        let link_open = Action::InsertHyperlink {
            sheet: 0,
            ranges: &[rect("B2")],
        };
        let link_locked = Action::InsertHyperlink {
            sheet: 0,
            ranges: &[rect("A1")],
        };
        let by_permission = [
            (Permission::FormatCells, Action::FormatCells { sheet: 0 }),
            (Permission::FormatColumns, Action::FormatColumns { sheet: 0 }),
            (Permission::FormatRows, Action::FormatRows { sheet: 0 }),
            (Permission::InsertColumns, Action::InsertColumns { sheet: 0 }),
            (Permission::InsertRows, Action::InsertRows { sheet: 0 }),
            (Permission::InsertHyperlinks, link_open),
            (Permission::DeleteColumns, columns),
            (Permission::Sort, sort_open),
            (Permission::PivotTables, Action::UsePivotTable { sheet: 0 }),
            (Permission::EditObjects, Action::EditObject { sheet: 0, locked: true }),
            (Permission::EditScenarios, Action::EditScenarios { sheet: 0 }),
        ];
        w.protect_sheet(0, SheetProtection::default(), "");
        for (_, action) in &by_permission {
            assert_eq!(w.permits(action), Err(Denied::Sheet), "{action:?}");
        }
        assert_eq!(w.permits(&Action::UseAutoFilter { sheet: 0 }), Err(Denied::Filter));
        assert_eq!(w.permits(&Action::EditComment { sheet: 0 }), Err(Denied::Comment));
        assert_eq!(w.permits(&rows), Err(Denied::Sheet));
        // An object that was unlocked beforehand stays editable.
        assert_eq!(
            w.permits(&Action::EditObject {
                sheet: 0,
                locked: false
            }),
            Ok(())
        );
        for (permission, action) in &by_permission {
            w.protect_sheet(0, SheetProtection::allowing(&[*permission]), "");
            assert_eq!(w.permits(action), Ok(()), "{action:?}");
            for (other, forbidden) in &by_permission {
                assert_eq!(
                    w.permits(forbidden).is_ok(),
                    other == permission,
                    "{forbidden:?} with {permission:?}"
                );
            }
        }
        // Locked cells are neither sorted, linked nor deleted, whatever is allowed.
        w.protect_sheet(0, SheetProtection::allowing(&Permission::ALL), "");
        assert_eq!(w.permits(&sort_locked), Err(Denied::Sheet));
        assert_eq!(w.permits(&link_locked), Err(Denied::Sheet));
        assert_eq!(w.permits(&rows), Err(Denied::LockedCells { rows: true }));
        assert_eq!(
            w.permits(&Action::DeleteColumns {
                sheet: 0,
                first: 1,
                last: 4
            }),
            Err(Denied::LockedCells { rows: false })
        );
        assert_eq!(w.permits(&columns), Ok(()));
        assert_eq!(w.permits(&Action::UseAutoFilter { sheet: 0 }), Ok(()));
        assert_eq!(w.permits(&Action::EditComment { sheet: 0 }), Ok(()));
        // Never allowed on a protected sheet.
        assert_eq!(w.permits(&Action::ToggleAutoFilter { sheet: 0 }), Err(Denied::Filter));
        assert_eq!(w.permits(&Action::ChangeTable { sheet: 0 }), Err(Denied::Table));
        assert_eq!(w.permits(&Action::ChangeSheet { sheet: 0 }), Err(Denied::Sheet));
        // The sheet itself is the workbook's business.
        assert_eq!(w.permits(&Action::DeleteSheet), Ok(()));
        assert_eq!(w.permits(&Action::RenameSheet), Ok(()));
    }

    #[test]
    fn selecting() {
        let mut w = form();
        assert!(w.can_select_cell(0, 0, 0));
        w.protect_sheet(0, SheetProtection::default(), "");
        assert!(w.can_select_cell(0, 0, 0) && w.can_select_cell(0, 1, 1));
        w.protect_sheet(0, SheetProtection::allowing(&[Permission::SelectUnlockedCells]), "");
        assert!(!w.can_select_cell(0, 0, 0) && w.can_select_cell(0, 1, 1) && w.can_select_cell(0, 50, 4));
        w.protect_sheet(0, SheetProtection::allowing(&[]), "");
        assert!(!w.can_select_cell(0, 0, 0) && !w.can_select_cell(0, 1, 1));
    }

    #[test]
    fn passwords_and_undo() {
        let mut w = form();
        w.set_cell_text(0, 1, 1, "2").unwrap();
        w.commit("edit");
        w.insert_rows(0, 10, 1);
        w.commit("insert");
        assert!(w.can_undo());
        assert!(w.protect_sheet(0, SheetProtection::default(), "Sésame"));
        // Protecting is not an undo step, and nothing before it can be
        // undone into an unprotected sheet.
        assert!(!w.can_undo() && !w.can_redo() && w.modified);
        assert!(w.sheet_needs_password(0));
        assert_eq!(w.unprotect_sheet(0, "sésame"), Err(PasswordError::Wrong));
        assert_eq!(w.unprotect_sheet(0, ""), Err(PasswordError::Wrong));
        assert!(w.is_sheet_protected(0));
        // Edits made while protected undo without touching the protection.
        w.set_cell_text(0, 1, 1, "3").unwrap();
        w.commit("edit");
        w.insert_rows(0, 20, 1);
        w.commit("insert");
        w.undo();
        w.undo();
        assert!(w.is_sheet_protected(0));
        assert_eq!(w.value_at(0, 1, 1), crate::Value::Number(2.0));
        assert_eq!(w.unprotect_sheet(0, "Sésame"), Ok(()));
        assert!(!w.is_sheet_protected(0) && !w.can_undo() && !w.can_redo());
        assert_eq!(w.unprotect_sheet(0, "anything"), Ok(()));
        // A copy of a protected sheet is protected by the same password.
        w.protect_sheet(0, SheetProtection::default(), "abc");
        let copy = w.duplicate_sheet(0).unwrap();
        w.commit("copy");
        assert_eq!(w.unprotect_sheet(copy, "x"), Err(PasswordError::Wrong));
        assert_eq!(w.unprotect_sheet(copy, "abc"), Ok(()));
        assert!(w.is_sheet_protected(0));
        // A sheet protected by an older application.
        w.sheets[0].protection.as_mut().unwrap().password = Some(PasswordHash::Legacy(0x83AF));
        assert_eq!(w.unprotect_sheet(0, "Password"), Err(PasswordError::Wrong));
        assert_eq!(w.unprotect_sheet(0, "password"), Ok(()));
    }

    #[test]
    fn workbook_structure() {
        let mut w = wb(&[]);
        let structure = [
            Action::InsertSheet,
            Action::DeleteSheet,
            Action::RenameSheet,
            Action::MoveSheet,
            Action::CopySheet,
            Action::HideSheet,
            Action::UnhideSheet,
        ];
        assert!(structure.iter().all(|action| w.permits(action).is_ok()));
        w.protect_workbook(true, false, "");
        assert!(w.protection.is_protected() && !w.workbook_needs_password());
        assert!(
            structure
                .iter()
                .all(|action| w.permits(action) == Err(Denied::Workbook))
        );
        // Cells are not the structure.
        assert_eq!(
            w.permits(&Action::EditCells {
                sheet: 0,
                ranges: &[rect("A1")]
            }),
            Ok(())
        );
        assert_eq!(w.unprotect_workbook(""), Ok(()));
        assert!(w.permits(&Action::InsertSheet).is_ok());
        w.protect_workbook(true, true, "key");
        assert!(w.workbook_needs_password());
        assert_eq!(w.unprotect_workbook("Key"), Err(PasswordError::Wrong));
        assert!(w.protection.lock_structure && w.protection.lock_windows);
        assert_eq!(w.unprotect_workbook("key"), Ok(()));
        assert_eq!(w.protection, WorkbookProtection::default());
        // Protecting the windows alone leaves the sheets free.
        w.protect_workbook(false, true, "");
        assert!(w.permits(&Action::DeleteSheet).is_ok());
    }

    #[test]
    fn messages() {
        for locale in Locale::all() {
            for denied in [
                Denied::Sheet,
                Denied::Workbook,
                Denied::LockedCells { rows: true },
                Denied::LockedCells { rows: false },
                Denied::Filter,
                Denied::Comment,
                Denied::Table,
            ] {
                let message = denied.message(locale);
                assert!(message.len() > 20 && !message.contains('|'), "{message}");
            }
        }
        assert!(
            Denied::Sheet
                .message(Locale::en())
                .starts_with("The cell or chart you're trying to change is on a protected sheet.")
        );
        assert!(Denied::Sheet.message(Locale::fr()).starts_with(
            "La cellule ou le graphique que vous essayez de modifier se trouve sur une feuille protégée."
        ));
        assert_eq!(
            Denied::Workbook.message(Locale::fr()),
            "Ce classeur est protégé et ne peut pas être modifié."
        );
        assert!(
            Denied::LockedCells { rows: false }
                .message(Locale::en())
                .starts_with("You are trying to delete a column that contains a locked cell.")
        );
        assert!(
            Denied::LockedCells { rows: true }
                .message(Locale::it())
                .starts_with("Si sta tentando di eliminare una riga contenente")
        );
        assert!(
            PasswordError::Wrong
                .message(Locale::en())
                .starts_with("The password you supplied is not correct.")
        );
        assert_eq!(
            PasswordError::Mismatch.message(Locale::de()),
            "Bestätigungseingabe ist nicht identisch."
        );
    }

    #[test]
    fn protection_set_in_a_transaction_is_rolled_back_or_forgets_the_history() {
        let mut w = Workbook::new(Locale::en());
        w.set_cell_text(0, 0, 0, "1").unwrap();
        w.commit("typed");
        // Dropped with the rest of the transaction: nothing is forgotten.
        w.snapshot();
        w.set_cell_text(0, 0, 0, "2").unwrap();
        assert!(w.set_sheet_protection(0, Some(SheetProtection::default())));
        assert!(w.sheets[0].protection.is_some());
        w.rollback();
        assert!(w.sheets[0].protection.is_none() && w.can_undo());
        assert_eq!(w.value_at(0, 0, 0), crate::Value::Number(1.0));
        // Kept: the history goes, as when the sheet is protected by hand.
        w.set_cell_text(0, 0, 0, "3").unwrap();
        w.set_sheet_protection(0, Some(SheetProtection::default()));
        assert!(w.commit("script"));
        assert!(w.sheets[0].protection.is_some() && !w.can_undo() && !w.can_redo());
        // What comes after is undone as ever, and the sheet stays protected.
        w.set_cell_text(0, 1, 0, "later").unwrap();
        w.commit("typed");
        assert!(w.undo() && w.sheets[0].protection.is_some() && !w.can_undo());
        // Setting what is already there is no change.
        assert!(w.set_sheet_protection(0, Some(SheetProtection::default())) && !w.commit("nothing"));
        assert!(w.can_redo() && !w.set_sheet_protection(9, None));
        // A copy that was unprotected tells the workbook that takes it over.
        let mut copy = w.fork();
        copy.set_sheet_protection(0, None);
        copy.set_cell_text(0, 0, 0, "4").unwrap();
        w.adopt(copy);
        assert!(w.commit("Run Script"));
        assert!(w.sheets[0].protection.is_none() && !w.can_undo());
        assert_eq!(w.value_at(0, 0, 0), crate::Value::Number(4.0));
    }

    #[test]
    fn ranges_that_may_be_edited() {
        let mut w = wb(&[("A1", "1"), ("B2", "2"), ("D4", "4")]);
        let free = ProtectedRange::new("Free", vec![rect("A1:B2")], None);
        let secret = ProtectedRange::new("Secret", vec![rect("D4:E5"), rect("G1")], Some(PasswordHash::salted("pw", b"0123456789abcdef", 1000)));
        assert_eq!(w.protected_range_valid(0, &free, None), Ok(()));
        w.set_protected_ranges(0, vec![free.clone(), secret.clone()]);
        w.commit("Allow Edit Ranges");
        assert_eq!(w.protected_range_valid(0, &ProtectedRange::new("free", vec![rect("A1")], None), None), Err(RangeError::TitleTaken));
        assert_eq!(w.protected_range_valid(0, &ProtectedRange::new("free", vec![rect("A1")], None), Some(0)), Ok(()));
        assert_eq!(w.protected_range_valid(0, &ProtectedRange::new("1st", vec![rect("A1")], None), None), Err(RangeError::BadTitle));
        assert_eq!(w.protected_range_valid(0, &ProtectedRange::new("", vec![rect("A1")], None), None), Err(RangeError::BadTitle));
        assert_eq!(w.protected_range_valid(0, &ProtectedRange::new("Empty", Vec::new(), None), None), Err(RangeError::NoCells));
        assert_eq!(w.protected_range_title(0, "Range"), "Range1");

        let edit = |w: &Workbook, range: &str| w.permits(&Action::EditCells { sheet: 0, ranges: &[rect(range)] });
        // Nothing changes while the sheet is not protected.
        assert_eq!(edit(&w, "C3"), Ok(()));
        assert_eq!(w.range_awaiting_password(0, &[rect("D4")]), None);
        w.protect_sheet(0, SheetProtection::default(), "");
        // A range without a password is open to everyone.
        assert_eq!(edit(&w, "A1:B2"), Ok(()));
        assert_eq!(edit(&w, "B2"), Ok(()));
        assert_eq!(edit(&w, "B2:C2"), Err(Denied::Sheet));
        assert_eq!(edit(&w, "C3"), Err(Denied::Sheet));
        assert_eq!(w.range_awaiting_password(0, &[rect("C3")]), None);
        // One with a password asks for it, once.
        assert_eq!(edit(&w, "D4"), Err(Denied::Sheet));
        assert_eq!(w.range_awaiting_password(0, &[rect("D5:E5")]), Some(1));
        assert_eq!(w.unlock_range(0, 1, "wrong"), Err(PasswordError::Wrong));
        assert_eq!(edit(&w, "D4"), Err(Denied::Sheet));
        assert_eq!(w.unlock_range(0, 1, "pw"), Ok(()));
        assert_eq!(edit(&w, "D4:E5"), Ok(()));
        assert_eq!(edit(&w, "G1"), Ok(()));
        assert_eq!(w.range_awaiting_password(0, &[rect("D4")]), None);
        // Unlocked cells beside an open range make one editable block.
        w.unprotect_sheet(0, "").unwrap();
        w.update_style(0, rect("C1:C2"), &|s| s.locked = false);
        w.commit("unlock");
        w.protect_sheet(0, SheetProtection::default(), "");
        assert_eq!(edit(&w, "A1:C2"), Ok(()));
        assert_eq!(edit(&w, "A1:C3"), Err(Denied::Sheet));
        // The password stays given through an undo.
        w.set_cell_text(0, 3, 3, "changed").unwrap();
        w.commit("typed");
        w.undo();
        assert_eq!(edit(&w, "D4"), Ok(()));
    }

    #[test]
    fn ranges_follow_rows_and_columns() {
        let mut w = wb(&[("A1", "1")]);
        w.set_protected_ranges(0, vec![ProtectedRange::new("Mid", vec![rect("B2:C3")], None), ProtectedRange::new("Top", vec![rect("A1:B1"), rect("E5")], None)]);
        w.commit("Allow Edit Ranges");
        // A row inserted inside a range makes it taller.
        w.insert_rows(0, 2, 1);
        w.commit("Insert");
        assert_eq!(w.sheets[0].protected_ranges[0].ranges, [rect("B2:C4")]);
        assert_eq!(w.sheets[0].protected_ranges[1].ranges, [rect("A1:B1"), rect("E6")]);
        // Deleting all the cells of a range forgets it; a range keeps the
        // cells it has left.
        w.delete_rows(0, 0, 1);
        w.commit("Delete");
        let left: Vec<(&str, &[Rect])> = w.sheets[0].protected_ranges.iter().map(|range| (range.name.as_str(), range.ranges.as_slice())).collect();
        assert_eq!(left, [("Mid", &[rect("B1:C3")][..]), ("Top", &[rect("E5")][..])]);
        w.delete_cols(0, 1, 2);
        w.commit("Delete");
        assert_eq!(w.sheets[0].protected_ranges.len(), 1);
        assert_eq!(w.sheets[0].protected_ranges[0].ranges, [rect("C5")]);
        w.undo();
        assert_eq!(w.sheets[0].protected_ranges.len(), 2);
    }
}
