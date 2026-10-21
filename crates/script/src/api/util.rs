//! What the classes share: addresses, colours, the wording of failures.

use serde_json::Value as J;

use opensheet_engine::ast::write_sheet_name;
use opensheet_engine::refs::{Rect, col_to_letters};
use opensheet_engine::style::Color;
use opensheet_engine::table::TableError;

use super::{Call, Fail, Res, state_of};
use crate::host::Host;

/// `Sheet1!A1:B2`, the sheet quoted when its name needs it.
pub(crate) fn address(h: &Host, sheet: usize, rect: Rect) -> String {
    let mut out = String::new();
    write_sheet_name(&h.wb.sheets[sheet].name, &mut out);
    out.push('!');
    out.push_str(&rect.name());
    out
}

/// `$A$1:$B$2`, `$A:$C`, `$1:$3`.
pub(crate) fn absolute(rect: Rect) -> String {
    let cell = |row: u32, col: u32| format!("${}${}", col_to_letters(col), row + 1);
    if rect.is_whole_cols() && !rect.is_whole_rows() {
        format!("${}:${}", col_to_letters(rect.c0), col_to_letters(rect.c1))
    } else if rect.is_whole_rows() && !rect.is_whole_cols() {
        format!("${}:${}", rect.r0 + 1, rect.r1 + 1)
    } else if rect.is_single() {
        cell(rect.r0, rect.c0)
    } else {
        format!("{}:{}", cell(rect.r0, rect.c0), cell(rect.r1, rect.c1))
    }
}

/// A reference to a range as formulas write it: `Sheet1!$A$1:$B$2`.
pub(crate) fn absolute_address(h: &Host, sheet: usize, rect: Rect) -> String {
    let mut out = String::new();
    write_sheet_name(&h.wb.sheets[sheet].name, &mut out);
    out.push('!');
    out.push_str(&absolute(rect));
    out
}

/// The range an address means on a sheet: `A1`, `B2:C5`, `A:C`,
/// `Sheet2!A1`, a defined name, the name of a table.
pub(crate) fn resolve(h: &mut Host, call: &Call, sheet: usize, text: &str) -> Res<(usize, Rect)> {
    if let Some(rect) = Rect::parse(text) {
        return Ok((sheet, rect));
    }
    let areas = h.english(|wb| wb.reference_areas(sheet, 0, 0, text));
    match areas.as_deref() {
        Some([one]) => Ok((one.sheet, one.rect)),
        Some(_) => Err(call.unsupported("A range of several areas (RangeAreas)")),
        None => Err(call.invalid()),
    }
}

/// The ranges addresses separated by commas mean on a sheet (`A1:B2,
/// D4`), all of one worksheet; a defined name may stand for several.
pub(crate) fn resolve_areas(
    h: &mut Host,
    call: &Call,
    sheet: usize,
    text: &str,
) -> Res<(usize, Vec<Rect>)> {
    let mut on = None;
    let mut rects = Vec::new();
    for piece in text.split(',').map(str::trim) {
        let found: Vec<(usize, Rect)> = match Rect::parse(piece) {
            Some(rect) => vec![(sheet, rect)],
            None => {
                let areas = h
                    .english(|wb| wb.reference_areas(sheet, 0, 0, piece))
                    .ok_or_else(|| call.invalid())?;
                areas.iter().map(|area| (area.sheet, area.rect)).collect()
            }
        };
        for (of, rect) in found {
            if *on.get_or_insert(of) != of {
                return Err(call.fail("The ranges must be on one worksheet."));
            }
            rects.push(rect);
        }
    }
    match on {
        Some(sheet) if !rects.is_empty() => Ok((sheet, rects)),
        _ => Err(call.invalid()),
    }
}

/// The sheet and the rectangle of a Range object, from its state.
pub(crate) fn range_from(h: &Host, call: &Call, state: &[J]) -> Res<(usize, Rect)> {
    let number = |index: usize| state.get(index).and_then(J::as_u64).map(|n| n as u32);
    let id = state
        .first()
        .and_then(J::as_i64)
        .ok_or_else(|| call.invalid())?;
    let sheet = h.sheet(call, id)?;
    match (number(1), number(2), number(3), number(4)) {
        (Some(r0), Some(c0), Some(r1), Some(c1)) => Ok((sheet, Rect::new(r0, c0, r1, c1))),
        _ => Err(call.invalid()),
    }
}

/// The ranges formatting is set on: the range of the object, and the
/// further ranges of a RangeAreas, which follow the sixth place of its
/// state.
pub(crate) fn format_rects(h: &Host, call: &Call) -> Res<(usize, Vec<Rect>)> {
    let (sheet, first) = this_range(h, call)?;
    let rest: Vec<u32> = call
        .this
        .iter()
        .skip(6)
        .filter_map(|n| n.as_u64().map(|n| n as u32))
        .collect();
    let mut rects = vec![first];
    rects.extend(
        rest.chunks_exact(4)
            .map(|n| Rect::new(n[0], n[1], n[2], n[3])),
    );
    Ok((sheet, rects))
}

/// The range a call is made on.
pub(crate) fn this_range(h: &Host, call: &Call) -> Res<(usize, Rect)> {
    range_from(h, call, call.this)
}

/// An argument that is a Range object or an address, read from `sheet`.
pub(crate) fn range_arg(
    h: &mut Host,
    call: &Call,
    index: usize,
    sheet: usize,
) -> Res<(usize, Rect)> {
    match call.arg(index) {
        J::String(text) => resolve(h, call, sheet, text),
        other => match state_of(other, "Range") {
            Some(state) => range_from(h, call, state),
            None => Err(call.invalid()),
        },
    }
}

pub(crate) fn table_fail(call: &Call, error: TableError) -> Fail {
    call.fail(match error {
        TableError::NotFound => "The requested resource doesn't exist.",
        TableError::InvalidRange => "The argument is invalid or missing or has an incorrect format.",
        TableError::Overlap => "A table can't overlap another table.",
        TableError::MergedCells => "A table can't be made over merged cells.",
        TableError::InvalidName => "The name is not valid for a table.",
        TableError::NameTaken => "A table or a defined name already has this name.",
        TableError::InvalidResize => "The table headers must remain in the same row, and the resulting table range must overlap the original table range.",
        TableError::Blocked => "This operation is not allowed. The operation is attempting to shift cells in a table on your worksheet.",
        TableError::NoHeaderRow => "The table has no header row.",
    })
}

// ---- colours -----------------------------------------------------------------

/// The colour names of HTML, which Excel accepts beside `#RRGGBB`.
const NAMED_COLORS: &[(&str, u32)] = &[
    ("aliceblue", 0xF0F8FF),
    ("antiquewhite", 0xFAEBD7),
    ("aqua", 0x00FFFF),
    ("aquamarine", 0x7FFFD4),
    ("azure", 0xF0FFFF),
    ("beige", 0xF5F5DC),
    ("bisque", 0xFFE4C4),
    ("black", 0x000000),
    ("blanchedalmond", 0xFFEBCD),
    ("blue", 0x0000FF),
    ("blueviolet", 0x8A2BE2),
    ("brown", 0xA52A2A),
    ("burlywood", 0xDEB887),
    ("cadetblue", 0x5F9EA0),
    ("chartreuse", 0x7FFF00),
    ("chocolate", 0xD2691E),
    ("coral", 0xFF7F50),
    ("cornflowerblue", 0x6495ED),
    ("cornsilk", 0xFFF8DC),
    ("crimson", 0xDC143C),
    ("cyan", 0x00FFFF),
    ("darkblue", 0x00008B),
    ("darkcyan", 0x008B8B),
    ("darkgoldenrod", 0xB8860B),
    ("darkgray", 0xA9A9A9),
    ("darkgreen", 0x006400),
    ("darkgrey", 0xA9A9A9),
    ("darkkhaki", 0xBDB76B),
    ("darkmagenta", 0x8B008B),
    ("darkolivegreen", 0x556B2F),
    ("darkorange", 0xFF8C00),
    ("darkorchid", 0x9932CC),
    ("darkred", 0x8B0000),
    ("darksalmon", 0xE9967A),
    ("darkseagreen", 0x8FBC8F),
    ("darkslateblue", 0x483D8B),
    ("darkslategray", 0x2F4F4F),
    ("darkslategrey", 0x2F4F4F),
    ("darkturquoise", 0x00CED1),
    ("darkviolet", 0x9400D3),
    ("deeppink", 0xFF1493),
    ("deepskyblue", 0x00BFFF),
    ("dimgray", 0x696969),
    ("dimgrey", 0x696969),
    ("dodgerblue", 0x1E90FF),
    ("firebrick", 0xB22222),
    ("floralwhite", 0xFFFAF0),
    ("forestgreen", 0x228B22),
    ("fuchsia", 0xFF00FF),
    ("gainsboro", 0xDCDCDC),
    ("ghostwhite", 0xF8F8FF),
    ("gold", 0xFFD700),
    ("goldenrod", 0xDAA520),
    ("gray", 0x808080),
    ("green", 0x008000),
    ("greenyellow", 0xADFF2F),
    ("grey", 0x808080),
    ("honeydew", 0xF0FFF0),
    ("hotpink", 0xFF69B4),
    ("indianred", 0xCD5C5C),
    ("indigo", 0x4B0082),
    ("ivory", 0xFFFFF0),
    ("khaki", 0xF0E68C),
    ("lavender", 0xE6E6FA),
    ("lavenderblush", 0xFFF0F5),
    ("lawngreen", 0x7CFC00),
    ("lemonchiffon", 0xFFFACD),
    ("lightblue", 0xADD8E6),
    ("lightcoral", 0xF08080),
    ("lightcyan", 0xE0FFFF),
    ("lightgoldenrodyellow", 0xFAFAD2),
    ("lightgray", 0xD3D3D3),
    ("lightgreen", 0x90EE90),
    ("lightgrey", 0xD3D3D3),
    ("lightpink", 0xFFB6C1),
    ("lightsalmon", 0xFFA07A),
    ("lightseagreen", 0x20B2AA),
    ("lightskyblue", 0x87CEFA),
    ("lightslategray", 0x778899),
    ("lightslategrey", 0x778899),
    ("lightsteelblue", 0xB0C4DE),
    ("lightyellow", 0xFFFFE0),
    ("lime", 0x00FF00),
    ("limegreen", 0x32CD32),
    ("linen", 0xFAF0E6),
    ("magenta", 0xFF00FF),
    ("maroon", 0x800000),
    ("mediumaquamarine", 0x66CDAA),
    ("mediumblue", 0x0000CD),
    ("mediumorchid", 0xBA55D3),
    ("mediumpurple", 0x9370DB),
    ("mediumseagreen", 0x3CB371),
    ("mediumslateblue", 0x7B68EE),
    ("mediumspringgreen", 0x00FA9A),
    ("mediumturquoise", 0x48D1CC),
    ("mediumvioletred", 0xC71585),
    ("midnightblue", 0x191970),
    ("mintcream", 0xF5FFFA),
    ("mistyrose", 0xFFE4E1),
    ("moccasin", 0xFFE4B5),
    ("navajowhite", 0xFFDEAD),
    ("navy", 0x000080),
    ("oldlace", 0xFDF5E6),
    ("olive", 0x808000),
    ("olivedrab", 0x6B8E23),
    ("orange", 0xFFA500),
    ("orangered", 0xFF4500),
    ("orchid", 0xDA70D6),
    ("palegoldenrod", 0xEEE8AA),
    ("palegreen", 0x98FB98),
    ("paleturquoise", 0xAFEEEE),
    ("palevioletred", 0xDB7093),
    ("papayawhip", 0xFFEFD5),
    ("peachpuff", 0xFFDAB9),
    ("peru", 0xCD853F),
    ("pink", 0xFFC0CB),
    ("plum", 0xDDA0DD),
    ("powderblue", 0xB0E0E6),
    ("purple", 0x800080),
    ("rebeccapurple", 0x663399),
    ("red", 0xFF0000),
    ("rosybrown", 0xBC8F8F),
    ("royalblue", 0x4169E1),
    ("saddlebrown", 0x8B4513),
    ("salmon", 0xFA8072),
    ("sandybrown", 0xF4A460),
    ("seagreen", 0x2E8B57),
    ("seashell", 0xFFF5EE),
    ("sienna", 0xA0522D),
    ("silver", 0xC0C0C0),
    ("skyblue", 0x87CEEB),
    ("slateblue", 0x6A5ACD),
    ("slategray", 0x708090),
    ("slategrey", 0x708090),
    ("snow", 0xFFFAFA),
    ("springgreen", 0x00FF7F),
    ("steelblue", 0x4682B4),
    ("tan", 0xD2B48C),
    ("teal", 0x008080),
    ("thistle", 0xD8BFD8),
    ("tomato", 0xFF6347),
    ("turquoise", 0x40E0D0),
    ("violet", 0xEE82EE),
    ("wheat", 0xF5DEB3),
    ("white", 0xFFFFFF),
    ("whitesmoke", 0xF5F5F5),
    ("yellow", 0xFFFF00),
    ("yellowgreen", 0x9ACD32),
];

/// `#RRGGBB`, `#RGB`, `RRGGBB` or an HTML colour name.
pub(crate) fn parse_color(text: &str) -> Option<Color> {
    let text = text.trim();
    let lower = text.to_ascii_lowercase();
    if let Some((_, rgb)) = NAMED_COLORS.iter().find(|(name, _)| *name == lower) {
        return Some(Color(*rgb));
    }
    let digits = text.strip_prefix('#').unwrap_or(text);
    if !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match digits.len() {
        6 => u32::from_str_radix(digits, 16).ok().map(Color),
        3 if text.starts_with('#') => {
            let wide: String = digits.chars().flat_map(|c| [c, c]).collect();
            u32::from_str_radix(&wide, 16).ok().map(Color)
        }
        _ => None,
    }
}

/// A colour argument; `None` for the empty text, which clears.
pub(crate) fn color_arg(call: &Call, index: usize) -> Res<Option<Color>> {
    let text = call.str(index)?;
    if text.trim().is_empty() {
        return Ok(None);
    }
    parse_color(text).map(Some).ok_or_else(|| call.invalid())
}

/// `#RRGGBB`.
pub(crate) fn color_text(color: Color) -> String {
    format!("#{:06X}", color.value())
}

// ---- base64 --------------------------------------------------------------------

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Bytes as base64 text, padded.
pub(crate) fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (chunk.get(1).copied().unwrap_or(0) as u32) << 8
            | chunk.get(2).copied().unwrap_or(0) as u32;
        for k in 0..4 {
            if k <= chunk.len() {
                out.push(BASE64[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// The bytes of base64 text (white space and padding are skipped);
/// `None` when a character is not of the alphabet.
pub(crate) fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut bits, mut count) = (0u32, 0);
    for byte in text
        .bytes()
        .filter(|b| !b.is_ascii_whitespace() && *b != b'=')
    {
        let value = BASE64.iter().position(|c| *c == byte)? as u32;
        bits = bits << 6 | value;
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
        }
    }
    Some(out)
}
