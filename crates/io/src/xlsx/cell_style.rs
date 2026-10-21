//! Cell styles in `styles.xml`: `<cellStyles>` names them, each with a
//! format in `<cellStyleXfs>` whose `apply…="0"` attributes say which
//! parts the style leaves out, and every cell format names its style by
//! `xfId`. A built-in style is known by its `builtinId`, whatever the
//! language of the name the file gives it.

use super::xml::push_attr;
use opensheet_engine::Workbook;
use opensheet_engine::ast::same_name;
use opensheet_engine::cell_style::{CellStyle, NORMAL, StyleParts, builtin_styles};
use opensheet_engine::style::{Style, StyleId};
use std::fmt::Write as _;

/// The styles a workbook is written with, Normal first: those it defines
/// and the built-in ones the formats in use (`used`, by style id) name.
pub(crate) fn written(wb: &Workbook, used: &[bool]) -> Vec<CellStyle> {
    let all = wb.all_cell_styles();
    let mut out: Vec<CellStyle> = all
        .iter()
        .filter(|style| style.builtin == Some(0))
        .cloned()
        .collect();
    let named = |name: &str| {
        used.iter()
            .enumerate()
            .filter(|(_, used)| **used)
            .filter_map(|(id, _)| wb.styles.get(StyleId(id as u32)).cell_style.as_deref())
            .any(|own| same_name(own, name))
    };
    for style in all {
        let defined = wb
            .cell_styles
            .iter()
            .any(|own| same_name(&own.name, &style.name));
        if style.builtin != Some(0) && (defined || named(&style.name)) {
            out.push(style);
        }
    }
    out
}

/// The `xfId` of a cell format: the place of its style among those
/// written (Normal, 0, when it has none or one that is gone).
pub(crate) fn xf_id(written: &[CellStyle], style: &Style) -> u32 {
    let Some(name) = style.cell_style.as_deref() else {
        return 0;
    };
    written
        .iter()
        .position(|known| same_name(&known.name, name))
        .unwrap_or(0) as u32
}

/// Append `<cellStyles>`.
pub(crate) fn write(x: &mut String, written: &[CellStyle]) {
    // What Excel's own version of each built-in style is: a changed one
    // is marked, or Excel would put its own back.
    let stock = builtin_styles(
        &written
            .first()
            .map(|normal| normal.style.clone())
            .unwrap_or_default(),
    );
    w!(x, "<cellStyles count=\"{}\">", written.len());
    for (index, style) in written.iter().enumerate() {
        x.push_str("<cellStyle name=\"");
        push_attr(x, &style.name);
        w!(x, "\" xfId=\"{index}\"");
        if let Some(id) = style.builtin {
            w!(x, " builtinId=\"{id}\"");
            if id != 0
                && stock
                    .iter()
                    .any(|known| known.builtin == Some(id) && known != style)
            {
                x.push_str(" customBuiltin=\"1\"");
            }
        }
        x.push_str("/>");
    }
    x.push_str("</cellStyles>");
}

/// A `<cellStyle>` as a file has it.
pub(crate) struct RawCellStyle {
    pub name: String,
    pub xf: usize,
    pub builtin: Option<u32>,
}

/// What the cell styles of a file come to: the name each `xfId` stands
/// for in the cell formats (`None` for Normal), and the styles the
/// workbook must keep because Excel's catalogue does not have them as
/// they are in the file.
pub(crate) struct LoadedStyles {
    pub names: Vec<Option<String>>,
    pub kept: Vec<CellStyle>,
}

/// `formats[i]` is the format and the included parts of `cellStyleXfs[i]`;
/// `normal` the format of the workbook's Normal style.
pub(crate) fn load(
    raw: Vec<RawCellStyle>,
    formats: &[(Style, StyleParts)],
    normal: &Style,
) -> LoadedStyles {
    let stock = builtin_styles(normal);
    let mut names: Vec<Option<String>> = vec![None; formats.len()];
    let mut kept: Vec<CellStyle> = Vec::new();
    for style in raw {
        let Some((format, includes)) = formats.get(style.xf) else {
            continue;
        };
        // A built-in style goes by its English name here.
        let known = style
            .builtin
            .and_then(|id| stock.iter().find(|known| known.builtin == Some(id)));
        let name = match known {
            Some(known) => known.name.clone(),
            None => style.name.trim().to_string(),
        };
        if name.is_empty() || style.builtin == Some(0) || same_name(&name, NORMAL) {
            continue;
        }
        // The first style of a name wins, as the first of an `xfId` does.
        if kept.iter().any(|own| same_name(&own.name, &name))
            || names.iter().flatten().any(|taken| same_name(taken, &name))
        {
            continue;
        }
        let loaded = CellStyle {
            name: name.clone(),
            builtin: known.and_then(|known| known.builtin),
            style: format.clone(),
            includes: *includes,
        };
        if known != Some(&loaded) {
            kept.push(loaded);
        }
        if names[style.xf].is_none() {
            names[style.xf] = Some(name);
        }
    }
    LoadedStyles { names, kept }
}
