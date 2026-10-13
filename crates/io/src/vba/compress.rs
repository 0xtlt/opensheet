//! The compression of MS-OVBA (section 2.4.1): the `dir` stream and the
//! source of each module are stored as a *compressed container*, a
//! signature byte followed by chunks of at most 4096 bytes of text, each
//! either raw or packed with back references into itself.

const CHUNK: usize = 4096;

/// How a copy token splits its 16 bits between distance and length, at a
/// position inside the decompressed chunk: the further in, the more bits
/// the distance takes. Returns (bits of the distance, mask of the length).
fn token_layout(position: usize) -> (u32, u16) {
    let mut bits = 4;
    while (1usize << bits) < position {
        bits += 1;
    }
    (bits, 0xFFFF >> bits)
}

/// The content of a compressed container; `None` when it is damaged.
pub(crate) fn decompress(data: &[u8]) -> Option<Vec<u8>> {
    if data.first() != Some(&1) {
        return None;
    }
    let mut out = Vec::new();
    let mut at = 1;
    while at < data.len() {
        let header = u16::from_le_bytes([data[at], *data.get(at + 1)?]);
        let size = (header & 0x0FFF) as usize + 3;
        let compressed = header & 0x8000 != 0;
        let end = (at + size).min(data.len());
        at += 2;
        let start = out.len();
        if !compressed {
            // A raw chunk is 4096 bytes whatever its header says.
            let end = (at + CHUNK).min(data.len());
            out.extend_from_slice(&data[at..end]);
            at = end;
            continue;
        }
        while at < end {
            let flags = data[at];
            at += 1;
            for bit in 0..8 {
                if at >= end {
                    break;
                }
                if flags & (1 << bit) == 0 {
                    out.push(data[at]);
                    at += 1;
                    continue;
                }
                let token = u16::from_le_bytes([data[at], *data.get(at + 1)?]);
                at += 2;
                let (bits, length_mask) = token_layout(out.len() - start);
                let length = (token & length_mask) as usize + 3;
                let distance = (token >> (16 - bits)) as usize + 1;
                if distance > out.len() - start {
                    return None;
                }
                for _ in 0..length {
                    out.push(out[out.len() - distance]);
                }
            }
        }
        if out.len() - start > CHUNK {
            return None;
        }
    }
    Some(out)
}

/// One chunk of text, packed; `None` when packing does not make it
/// smaller than the 4096 bytes a chunk may hold.
fn pack(chunk: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(chunk.len());
    let mut at = 0;
    while at < chunk.len() {
        let flag_at = out.len();
        out.push(0u8);
        for bit in 0..8 {
            if at >= chunk.len() {
                break;
            }
            let (bits, length_mask) = token_layout(at);
            let longest = (length_mask as usize + 3).min(chunk.len() - at);
            // The longest run that also starts earlier in the chunk; of
            // equal runs, the nearest.
            let (mut best, mut best_at) = (0, 0);
            for candidate in (0..at).rev() {
                let mut length = 0;
                while length < longest && chunk[candidate + length] == chunk[at + length] {
                    length += 1;
                }
                if length > best {
                    best = length;
                    best_at = candidate;
                    if best == longest {
                        break;
                    }
                }
            }
            if best >= 3 {
                let token = (((at - best_at - 1) as u16) << (16 - bits)) | (best - 3) as u16;
                out.extend_from_slice(&token.to_le_bytes());
                out[flag_at] |= 1 << bit;
                at += best;
            } else {
                out.push(chunk[at]);
                at += 1;
            }
        }
        if out.len() > CHUNK {
            return None;
        }
    }
    Some(out)
}

/// A compressed container holding `data`.
///
/// A chunk that cannot be packed is stored raw, and a raw chunk is always
/// 4096 bytes: the format has no way to say that the last one is shorter,
/// so such an ending is filled with zeros (text always packs).
pub(crate) fn compress(data: &[u8]) -> Vec<u8> {
    let mut out = vec![1u8];
    for chunk in data.chunks(CHUNK) {
        match pack(chunk) {
            Some(packed) => {
                let header = 0xB000 | (packed.len() + 2 - 3) as u16;
                out.extend_from_slice(&header.to_le_bytes());
                out.extend_from_slice(&packed);
            }
            None => {
                out.extend_from_slice(&0x3FFFu16.to_le_bytes());
                out.extend_from_slice(chunk);
                out.resize(out.len() + CHUNK - chunk.len(), 0);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_examples_of_the_specification() {
        // MS-OVBA 3.2.1: no compression possible.
        let plain = b"abcdefghijklmnopqrstuv.";
        let stored = [
            0x01, 0x19, 0xB0, 0x00, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x00, 0x69, 0x6A, 0x6B, 0x6C, 0x6D, 0x6E, 0x6F, 0x70,
            0x00, 0x71, 0x72, 0x73, 0x74, 0x75, 0x76, 0x2E,
        ];
        assert_eq!(decompress(&stored).as_deref(), Some(&plain[..]));
        assert_eq!(compress(plain), stored);
        // 3.2.2: normal compression.
        let text = b"#aaabcdefaaaaghijaaaaaklaaamnopqaaaaaaaaaaaarstuvwxyzaaa";
        let stored = [
            0x01, 0x2F, 0xB0, 0x00, 0x23, 0x61, 0x61, 0x61, 0x62, 0x63, 0x64, 0x65, 0x82, 0x66, 0x00, 0x70, 0x61, 0x67, 0x68, 0x69, 0x6A,
            0x01, 0x38, 0x08, 0x61, 0x6B, 0x6C, 0x00, 0x30, 0x6D, 0x6E, 0x6F, 0x70, 0x06, 0x71, 0x02, 0x70, 0x04, 0x10, 0x72, 0x73, 0x74,
            0x75, 0x76, 0x10, 0x77, 0x78, 0x79, 0x7A, 0x00, 0x3C,
        ];
        assert_eq!(decompress(&stored).as_deref(), Some(&text[..]));
        assert_eq!(decompress(&compress(text)).as_deref(), Some(&text[..]));
        // 3.2.3: maximum compression.
        let run = [b'a'; 73];
        let stored = [0x01, 0x03, 0xB0, 0x02, 0x61, 0x45, 0x00];
        assert_eq!(decompress(&stored).as_deref(), Some(&run[..]));
        assert_eq!(compress(&run), stored);
    }

    #[test]
    fn text_of_any_length_survives() {
        let line = "Sub Macro1()\r\n    Range(\"A1\").Value = \"été\" & Chr(10)\r\nEnd Sub\r\n";
        for repeats in [0, 1, 3, 64, 65, 200, 700] {
            let text: Vec<u8> = line.repeat(repeats).into_bytes();
            let packed = compress(&text);
            assert_eq!(decompress(&packed), Some(text.clone()), "{repeats} lines");
            // Source text repeats itself: it gets much smaller.
            assert!(repeats < 3 || packed.len() < text.len() / 2, "{repeats} lines");
        }
        // Exactly one chunk, and one byte more.
        for len in [4095, 4096, 4097, 8192] {
            let text: Vec<u8> = (0..len).map(|i| b"ab\r\n "[i % 5]).collect();
            assert_eq!(decompress(&compress(&text)), Some(text));
        }
    }

    #[test]
    fn what_does_not_pack_is_stored_raw() {
        // Bytes without any repeat: a whole chunk goes in as it is.
        let mut state = 12345u32;
        let noise: Vec<u8> = (0..CHUNK)
            .map(|_| {
                state = state.wrapping_mul(1_103_515_245).wrapping_add(12345);
                (state >> 16) as u8
            })
            .collect();
        let stored = compress(&noise);
        assert_eq!(stored.len(), 1 + 2 + CHUNK);
        assert_eq!(decompress(&stored), Some(noise));
    }

    #[test]
    fn damaged_containers_are_refused() {
        assert_eq!(decompress(&[]), None);
        assert_eq!(decompress(&[2, 0, 0]), None);
        // A copy from before the start of the chunk.
        assert_eq!(decompress(&[0x01, 0x02, 0xB0, 0x01, 0x00, 0x10]), None);
        // Cut in the middle of a token: no panic.
        let good = compress(b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaabbbbbbbbbbbbbbbbbbbb");
        for cut in 0..good.len() {
            let _ = decompress(&good[..cut]);
        }
    }
}
