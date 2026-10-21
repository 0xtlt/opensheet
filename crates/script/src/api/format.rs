//! `RangeFormat` and its parts: fill, font, borders, protection.

use serde_json::Value as J;

use opensheet_engine::features::Edit;
use opensheet_engine::format::format_value;
use opensheet_engine::metrics::FontMetrics;
use opensheet_engine::protect::Action;
use opensheet_engine::refs::Rect;
use opensheet_engine::style::{
    Border, BorderStyle, Borders, Font, HAlign, Style, Underline, VAlign,
};
use opensheet_engine::workbook::Content;

use super::util::{color_arg, color_text, format_rects, this_range};
use super::{ApiClass, Call, Res, Ret, class, enum_value, object};
use crate::host::Host;

/// Pixels (the unit of the engine, at 96 per inch) per point.
const PX_PER_PT: f64 = 96.0 / 72.0;
/// Past this many cells a property is read from the used part of the
/// range only: the cells beyond are all alike.
const SCAN_LIMIT: u64 = 200_000;

/// A property of the formatting of a range: its value when every cell
/// has the same, null when they differ, as in Excel.
fn uniform(h: &Host, call: &Call, read: fn(&Style) -> J) -> Res<J> {
    let (sheet, rect) = this_range(h, call)?;
    let rect = if rect.rows() as u64 * rect.cols() as u64 > SCAN_LIMIT {
        let used = h.wb.sheets[sheet].used_rect().unwrap_or(Rect::cell(0, 0));
        Rect::new(
            rect.r0,
            rect.c0,
            rect.r1.min(used.r1.max(rect.r0)),
            rect.c1.min(used.c1.max(rect.c0)),
        )
    } else {
        rect
    };
    h.sized(call, rect)?;
    let first = read(h.wb.style_at(sheet, rect.r0, rect.c0));
    let same = (rect.r0..=rect.r1)
        .all(|row| (rect.c0..=rect.c1).all(|col| read(h.wb.style_at(sheet, row, col)) == first));
    Ok(if same { first } else { J::Null })
}

fn get(h: &mut Host, call: &Call, read: fn(&Style) -> J) -> Res<Ret> {
    Ok(Ret::Value(uniform(h, call, read)?))
}

/// A colour property: `#RRGGBB`, the empty text when the cells differ.
fn get_color(h: &mut Host, call: &Call, read: fn(&Style) -> J) -> Res<Ret> {
    Ok(Ret::Value(match uniform(h, call, read)? {
        J::Null => J::from(""),
        color => color,
    }))
}

/// Change the formatting of every cell of the range, as the Format Cells
/// dialog does.
fn set(h: &mut Host, call: &Call, change: &dyn Fn(&mut Style)) -> Res<Ret> {
    let (sheet, rects) = format_rects(h, call)?;
    h.permit_edit(call, &Edit::Format { sheet })?;
    for rect in rects {
        h.wb.update_style(sheet, rect, change);
    }
    Ok(Ret::Nothing)
}

// ---- borders -----------------------------------------------------------------

const SIDES: [&str; 6] = [
    "EdgeTop",
    "EdgeBottom",
    "EdgeLeft",
    "EdgeRight",
    "InsideVertical",
    "InsideHorizontal",
];

/// The side takes the sixth place of the state; the further ranges of a
/// RangeAreas follow it.
fn border_object(call: &Call, side: &str) -> J {
    let mut state = call.this.to_vec();
    match state.get_mut(5) {
        Some(slot) => *slot = side.into(),
        None => state.push(side.into()),
    }
    object("RangeBorder", state)
}

fn side_of<'a>(call: &Call<'a>) -> &'a str {
    call.this.get(5).and_then(J::as_str).unwrap_or("EdgeTop")
}

/// The line of a border as the API describes it: a style and a weight.
fn line_names(style: BorderStyle) -> (&'static str, &'static str) {
    match style {
        BorderStyle::None => ("None", "Thin"),
        BorderStyle::Hair => ("Continuous", "Hairline"),
        BorderStyle::Thin => ("Continuous", "Thin"),
        BorderStyle::Medium => ("Continuous", "Medium"),
        BorderStyle::Thick => ("Continuous", "Thick"),
        BorderStyle::Dashed => ("Dash", "Thin"),
        BorderStyle::Dotted => ("Dot", "Thin"),
        BorderStyle::Double => ("Double", "Thick"),
    }
}

fn line_of(call: &Call, style: &str, weight: &str) -> Res<BorderStyle> {
    Ok(match (style, weight) {
        ("None", _) => BorderStyle::None,
        ("Continuous", "Hairline") => BorderStyle::Hair,
        ("Continuous", "Thin") => BorderStyle::Thin,
        ("Continuous", "Medium") => BorderStyle::Medium,
        ("Continuous", "Thick") => BorderStyle::Thick,
        ("Double", _) => BorderStyle::Double,
        ("Dash", "Thin" | "Hairline") => BorderStyle::Dashed,
        ("Dot", "Thin" | "Hairline") => BorderStyle::Dotted,
        ("Dash" | "Dot", heavy) => {
            return Err(call.unsupported(format!(
                "A {} border of weight {heavy}",
                style.to_lowercase()
            )));
        }
        (other, _) => {
            return Err(call.unsupported(format!(
                "ExcelScript.BorderLineStyle.{}{}",
                other[..1].to_lowercase(),
                &other[1..]
            )));
        }
    })
}

/// The cells a border of a range runs along, with the side of each cell
/// it is: an inside line is the bottom of a cell and the top of the next.
fn border_parts(rect: Rect, side: &str) -> Vec<(Rect, fn(&mut Borders) -> &mut Border)> {
    fn top(b: &mut Borders) -> &mut Border {
        &mut b.top
    }
    fn bottom(b: &mut Borders) -> &mut Border {
        &mut b.bottom
    }
    fn left(b: &mut Borders) -> &mut Border {
        &mut b.left
    }
    fn right(b: &mut Borders) -> &mut Border {
        &mut b.right
    }
    match side {
        "EdgeTop" => vec![(Rect::new(rect.r0, rect.c0, rect.r0, rect.c1), top)],
        "EdgeBottom" => vec![(Rect::new(rect.r1, rect.c0, rect.r1, rect.c1), bottom)],
        "EdgeLeft" => vec![(Rect::new(rect.r0, rect.c0, rect.r1, rect.c0), left)],
        "EdgeRight" => vec![(Rect::new(rect.r0, rect.c1, rect.r1, rect.c1), right)],
        "InsideHorizontal" if rect.rows() > 1 => vec![
            (Rect::new(rect.r0, rect.c0, rect.r1 - 1, rect.c1), bottom),
            (Rect::new(rect.r0 + 1, rect.c0, rect.r1, rect.c1), top),
        ],
        "InsideVertical" if rect.cols() > 1 => vec![
            (Rect::new(rect.r0, rect.c0, rect.r1, rect.c1 - 1), right),
            (Rect::new(rect.r0, rect.c0 + 1, rect.r1, rect.c1), left),
        ],
        _ => Vec::new(),
    }
}

/// The border a RangeBorder object stands for, as its first cell has it.
fn border_now(h: &Host, call: &Call) -> Res<Border> {
    let (sheet, rect) = this_range(h, call)?;
    let rect = h.wb.clamp_to_used(sheet, rect);
    Ok(match border_parts(rect, side_of(call)).first() {
        Some((cells, pick)) => {
            let mut borders = h.wb.style_at(sheet, cells.r0, cells.c0).borders;
            *pick(&mut borders)
        }
        None => Border::NONE,
    })
}

fn set_border(h: &mut Host, call: &Call, change: &dyn Fn(&mut Border)) -> Res<Ret> {
    let (sheet, rects) = format_rects(h, call)?;
    h.permit_edit(call, &Edit::Format { sheet })?;
    for rect in rects {
        let rect = h.wb.clamp_to_used(sheet, rect);
        h.sized(call, rect)?;
        for (cells, pick) in border_parts(rect, side_of(call)) {
            h.wb.update_style(sheet, cells, &|style| change(pick(&mut style.borders)));
        }
    }
    Ok(Ret::Nothing)
}

// ---- sizes -------------------------------------------------------------------

/// The width a text takes, estimated without the font: the digits of the
/// default font are the unit, as for column widths.
fn text_width(text: &str, font: &Font, digit: f64) -> f64 {
    let units: f64 = text
        .chars()
        .map(|ch| match ch {
            'i' | 'j' | 'l' | 'I' | '.' | ',' | ':' | ';' | '\'' | '|' | '!' | ' ' => 0.5,
            'f' | 't' | 'r' | '(' | ')' | '[' | ']' | '-' | '"' | '*' | '/' => 0.65,
            'm' | 'w' | 'M' | 'W' | '@' | '%' => 1.5,
            c if c.is_ascii_uppercase() => 1.2,
            c if c.is_ascii() => 1.0,
            c if c.is_alphabetic() && (c as u32) < 0x2E80 => 1.05,
            _ => 1.9,
        })
        .sum();
    units * digit * font.size_pt() as f64 / 11.0 * if font.bold { 1.07 } else { 1.0 }
}

/// What a cell shows, with its font, when it shows something.
fn shown<'a>(h: &'a Host, sheet: usize, row: u32, col: u32) -> Option<(String, &'a Style)> {
    let cell =
        h.wb.cell(sheet, row, col)
            .filter(|cell| !matches!(cell.content, Content::Empty))?;
    let style = h.wb.styles.get(cell.style);
    let text = format_value(&cell.value, &style.num_fmt, h.wb.locale).text;
    (!text.is_empty()).then_some((text, style))
}

pub(super) const RANGE_FORMAT: ApiClass = class!("RangeFormat", "The formatting of a range, or of the ranges of a RangeAreas: setters of the fill, the font, the borders, the alignment and the protection then reach them all, while getters and sizes read and set the first. A getter gives null when the cells differ.", {
    getFill "(): RangeFill" "The background of the cells." => |h, c| {
        this_range(h, c)?;
        Ok(Ret::object("RangeFill", c.this.to_vec()))
    };
    getFont "(): RangeFont" "The font of the cells." => |h, c| {
        this_range(h, c)?;
        Ok(Ret::object("RangeFont", c.this.to_vec()))
    };
    getProtection "(): FormatProtection" "Whether the cells are locked and their formulas hidden when the sheet is protected." => |h, c| {
        this_range(h, c)?;
        Ok(Ret::object("FormatProtection", c.this.to_vec()))
    };
    getRangeBorder "(index: ExcelScript.BorderIndex): RangeBorder" "One border of the range: an edge, or the lines inside." => |h, c| {
        this_range(h, c)?;
        let side = enum_value(c, 0, "BorderIndex")?.ok_or_else(|| c.invalid())?;
        if !SIDES.contains(&side) {
            return Err(c.unsupported(format!("ExcelScript.BorderIndex.{}{} (diagonal borders)", side[..1].to_lowercase(), &side[1..])));
        }
        Ok(Ret::Object(border_object(c, side)))
    };
    getBorders "(): RangeBorder[]" "The four edges and the two kinds of inside lines (OpenSheet has no diagonal borders)." => |h, c| {
        this_range(h, c)?;
        Ok(Ret::Object(J::from(SIDES.iter().map(|side| border_object(c, side)).collect::<Vec<J>>())))
    };
    getHorizontalAlignment "(): ExcelScript.HorizontalAlignment" "How the cells are aligned between left and right." => |h, c| {
        get(h, c, |style| match style.h_align {
            HAlign::General => "General",
            HAlign::Left => "Left",
            HAlign::Center => "Center",
            HAlign::Right => "Right",
            HAlign::Fill => "Fill",
            HAlign::Justify => "Justify",
            HAlign::CenterAcross => "CenterAcrossSelection",
        }.into())
    };
    setHorizontalAlignment "(horizontalAlignment: ExcelScript.HorizontalAlignment): void" "Aligns the cells: General, Left, Center, Right, Fill, Justify or CenterAcrossSelection." => |h, c| {
        let align = match enum_value(c, 0, "HorizontalAlignment")?.ok_or_else(|| c.invalid())? {
            "General" => HAlign::General,
            "Left" => HAlign::Left,
            "Center" => HAlign::Center,
            "Right" => HAlign::Right,
            "Fill" => HAlign::Fill,
            "Justify" => HAlign::Justify,
            "CenterAcrossSelection" => HAlign::CenterAcross,
            _ => return Err(c.unsupported("ExcelScript.HorizontalAlignment.distributed")),
        };
        set(h, c, &|style| style.h_align = align)
    };
    getVerticalAlignment "(): ExcelScript.VerticalAlignment" "How the cells are aligned between top and bottom." => |h, c| {
        get(h, c, |style| match style.v_align {
            VAlign::Top => "Top",
            VAlign::Center => "Center",
            VAlign::Bottom => "Bottom",
        }.into())
    };
    setVerticalAlignment "(verticalAlignment: ExcelScript.VerticalAlignment): void" "Aligns the cells: Top, Center or Bottom." => |h, c| {
        let align = match enum_value(c, 0, "VerticalAlignment")?.ok_or_else(|| c.invalid())? {
            "Top" => VAlign::Top,
            "Center" => VAlign::Center,
            "Bottom" => VAlign::Bottom,
            other => return Err(c.unsupported(format!("ExcelScript.VerticalAlignment.{}", other.to_lowercase()))),
        };
        set(h, c, &|style| style.v_align = align)
    };
    getWrapText "(): boolean" "Whether text goes on several lines in the cells." => |h, c| get(h, c, |style| style.wrap.into());
    setWrapText "(wrapText: boolean): void" "Wraps text in the cells or not." => |h, c| {
        let wrap = c.bool(0)?;
        set(h, c, &|style| style.wrap = wrap)
    };
    getShrinkToFit "(): boolean" "Whether text is made smaller to fit the width of the cells." => |h, c| get(h, c, |style| style.shrink_to_fit.into());
    setShrinkToFit "(shrinkToFit: boolean): void" "Shrinks text to the width of the cells or not." => |h, c| {
        let shrink = c.bool(0)?;
        set(h, c, &|style| style.shrink_to_fit = shrink)
    };
    getIndentLevel "(): number" "The indent of the cells, 0 to 250." => |h, c| get(h, c, |style| style.indent.into());
    setIndentLevel "(indentLevel: number): void" "Indents the cells, 0 to 250." => |h, c| {
        let indent = u8::try_from(c.int(0)?).ok().filter(|n| *n <= 250).ok_or_else(|| c.invalid())?;
        set(h, c, &|style| style.indent = indent)
    };
    getTextOrientation "(): number" "The rotation of the text in degrees, -90 to 90." => |h, c| get(h, c, |style| style.rotation.into());
    setTextOrientation "(textOrientation: number): void" "Rotates the text, -90 to 90 degrees." => |h, c| {
        let degrees = c.int(0)?;
        if degrees == 180 {
            return Err(c.unsupported("RangeFormat.setTextOrientation(180): vertical text"));
        }
        if !(-90..=90).contains(&degrees) {
            return Err(c.invalid());
        }
        set(h, c, &|style| style.rotation = degrees as i16)
    };
    getColumnWidth "(): number" "The width of the columns in points; null when they differ." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let sh = &h.wb.sheets[sheet];
        let width = |col: u32| sh.col_widths.get(&col).copied().unwrap_or(sh.default_col_width);
        let first = width(rect.c0);
        let same = (rect.c0..=rect.c1).all(|col| width(col) == first);
        Ok(Ret::Value(if same { J::from(first as f64 / PX_PER_PT) } else { J::Null }))
    };
    setColumnWidth "(columnWidth: number): void" "Gives the columns a width in points." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let points = c.num(0)?;
        if !(0.0..=1350.0).contains(&points) {
            return Err(c.invalid());
        }
        h.permit_edit(c, &Edit::SizeCols { sheet })?;
        for col in rect.c0..=rect.c1 {
            h.wb.set_col_width(sheet, col, Some((points * PX_PER_PT) as f32));
        }
        Ok(Ret::Nothing)
    };
    getRowHeight "(): number" "The height of the rows in points; null when they differ." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let sh = &h.wb.sheets[sheet];
        let mut sized = sh.row_heights.range(rect.r0..=rect.r1).map(|(_, height)| *height);
        let height = match sized.next() {
            None => Some(sh.default_row_height),
            Some(first) => {
                let all = sh.row_heights.range(rect.r0..=rect.r1).count() as u32 == rect.rows();
                (sized.all(|other| other == first) && (all || first == sh.default_row_height)).then_some(first)
            }
        };
        Ok(Ret::Value(height.map_or(J::Null, |px| J::from(px as f64 / PX_PER_PT))))
    };
    setRowHeight "(rowHeight: number): void" "Gives the rows a height in points." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let points = c.num(0)?;
        if !(0.0..=409.5).contains(&points) {
            return Err(c.invalid());
        }
        h.permit_edit(c, &Edit::SizeRows { sheet })?;
        h.sized(c, Rect::new(rect.r0, 0, rect.r1, 0))?;
        for row in rect.r0..=rect.r1 {
            h.wb.set_row_height(sheet, row, Some((points * PX_PER_PT) as f32));
        }
        Ok(Ret::Nothing)
    };
    autofitColumns "(): void" "Fits the width of the columns to what their cells show." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        h.permit_edit(c, &Edit::SizeCols { sheet })?;
        h.calculate();
        let digit = h.wb.font_metrics().digit_width;
        let mut widths: std::collections::BTreeMap<u32, f64> = std::collections::BTreeMap::new();
        for (row, col) in h.wb.sheets[sheet].cells.keys() {
            if *col < rect.c0 || *col > rect.c1 || h.wb.sheets[sheet].merge_at(*row, *col).is_some() {
                continue;
            }
            if let Some((text, style)) = shown(h, sheet, *row, *col) {
                let longest = text.lines().map(|line| text_width(line, &style.font, digit)).fold(0.0, f64::max);
                let width = widths.entry(*col).or_default();
                *width = width.max(longest + style.indent as f64 * 9.0);
            }
        }
        for col in rect.c0..=h.wb.clamp_to_used(sheet, rect).c1.max(rect.c0).min(rect.c1) {
            // As when the edge of a column header is double-clicked: an
            // empty column goes back to the standard width.
            let fitted = widths.get(&col).map(|width| ((width + 9.0).ceil() as f32).clamp(12.0, 1800.0));
            h.wb.set_col_width(sheet, col, fitted);
        }
        h.autofits.push((c.this_int(0), true, rect.c0, rect.c1));
        Ok(Ret::Nothing)
    };
    autofitRows "(): void" "Fits the height of the rows to what their cells show." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        h.permit_edit(c, &Edit::SizeRows { sheet })?;
        h.calculate();
        let digit = h.wb.font_metrics().digit_width;
        let standard = h.wb.sheets[sheet].default_row_height;
        let mut heights: std::collections::BTreeMap<u32, f32> = std::collections::BTreeMap::new();
        for (row, col) in h.wb.sheets[sheet].cells.range((rect.r0, 0)..=(rect.r1, u32::MAX)).map(|(key, _)| *key) {
            if h.wb.sheets[sheet].merge_at(row, col).is_some() {
                continue;
            }
            if let Some((text, style)) = shown(h, sheet, row, col) {
                let room = (h.wb.sheets[sheet].col_width(col) as f64 - 5.0).max(digit);
                let lines: f64 = text
                    .lines()
                    .map(|line| if style.wrap { (text_width(line, &style.font, digit) / room).ceil().max(1.0) } else { 1.0 })
                    .sum();
                let lines = if style.wrap { lines.max(1.0) } else { 1.0 };
                let line = FontMetrics::of(&style.font).row_height.max(standard);
                let height = heights.entry(row).or_insert(standard);
                *height = height.max(line + (lines as f32 - 1.0) * (line - 2.0));
            }
        }
        // The rows that hold something, and those that were sized.
        let sized: Vec<u32> = h.wb.sheets[sheet].row_heights.range(rect.r0..=rect.r1).map(|(row, _)| *row).collect();
        let rows: std::collections::BTreeSet<u32> = sized.into_iter().chain(heights.keys().copied()).collect();
        for row in rows {
            // A row of the standard height is a row nobody sized.
            let fitted = heights.get(&row).copied().filter(|height| *height > standard);
            h.wb.set_row_height(sheet, row, fitted);
        }
        h.autofits.push((c.this_int(0), false, rect.r0, rect.r1));
        Ok(Ret::Nothing)
    };
});

pub(super) const RANGE_FILL: ApiClass = class!("RangeFill", "The background of the cells of a range.", {
    getColor "(): string" "The fill colour as #RRGGBB (#FFFFFF without a fill); empty text when the cells differ." => |h, c| {
        get_color(h, c, |style| style.fill.map_or("#FFFFFF".to_string(), color_text).into())
    };
    setColor "(color: string): void" "Fills the cells with a colour: #RRGGBB or a colour name (yellow, lightgray)." => |h, c| {
        let color = color_arg(c, 0)?;
        set(h, c, &|style| style.fill = color)
    };
    clear "(): void" "Removes the fill." => |h, c| set(h, c, &|style| style.fill = None);
});

pub(super) const RANGE_FONT: ApiClass = class!("RangeFont", "The font of the cells of a range.", {
    getBold "(): boolean" "Whether the font is bold." => |h, c| get(h, c, |style| style.font.bold.into());
    setBold "(bold: boolean): void" "Makes the font bold or not." => |h, c| {
        let bold = c.bool(0)?;
        set(h, c, &|style| style.font.bold = bold)
    };
    getItalic "(): boolean" "Whether the font is italic." => |h, c| get(h, c, |style| style.font.italic.into());
    setItalic "(italic: boolean): void" "Makes the font italic or not." => |h, c| {
        let italic = c.bool(0)?;
        set(h, c, &|style| style.font.italic = italic)
    };
    getUnderline "(): ExcelScript.RangeUnderlineStyle" "The underline: None, Single or Double." => |h, c| {
        get(h, c, |style| match style.font.underline {
            Underline::None => "None",
            Underline::Single => "Single",
            Underline::Double => "Double",
        }.into())
    };
    setUnderline "(underline: ExcelScript.RangeUnderlineStyle): void" "Underlines the text: None, Single or Double." => |h, c| {
        let underline = match enum_value(c, 0, "RangeUnderlineStyle")?.ok_or_else(|| c.invalid())? {
            "None" => Underline::None,
            "Single" => Underline::Single,
            "Double" => Underline::Double,
            other => return Err(c.unsupported(format!("ExcelScript.RangeUnderlineStyle.{}{}", other[..1].to_lowercase(), &other[1..]))),
        };
        set(h, c, &|style| style.font.underline = underline)
    };
    getStrikethrough "(): boolean" "Whether the text is struck through." => |h, c| get(h, c, |style| style.font.strike.into());
    setStrikethrough "(strikethrough: boolean): void" "Strikes the text through or not." => |h, c| {
        let strike = c.bool(0)?;
        set(h, c, &|style| style.font.strike = strike)
    };
    getColor "(): string" "The colour of the text as #RRGGBB (#000000 for automatic); empty text when the cells differ." => |h, c| {
        get_color(h, c, |style| style.font.color.map_or("#000000".to_string(), color_text).into())
    };
    setColor "(color: string): void" "Colours the text: #RRGGBB or a colour name." => |h, c| {
        let color = color_arg(c, 0)?.ok_or_else(|| c.invalid())?;
        set(h, c, &|style| style.font.color = Some(color))
    };
    getName "(): string" "The name of the font." => |h, c| get(h, c, |style| style.font.name.as_ref().into());
    setName "(name: string): void" "Changes the font." => |h, c| {
        let name = c.str(0)?.trim();
        if name.is_empty() {
            return Err(c.invalid());
        }
        set(h, c, &|style| style.font.set_name(name))
    };
    getSize "(): number" "The size of the font in points." => |h, c| get(h, c, |style| (style.font.size_pt() as f64).into());
    setSize "(size: number): void" "Changes the size of the font, in points (1 to 409)." => |h, c| {
        let size = c.num(0)?;
        if !(1.0..=409.0).contains(&size) {
            return Err(c.invalid());
        }
        set(h, c, &|style| style.font.set_size_pt(size as f32))
    };
});

pub(super) const RANGE_BORDER: ApiClass = class!("RangeBorder", "One border of a range: an edge, or the lines between its cells.", {
    getSideIndex "(): ExcelScript.BorderIndex" "Which border this is." => |h, c| {
        this_range(h, c)?;
        Ok(side_of(c).into())
    };
    getStyle "(): ExcelScript.BorderLineStyle" "The line: None, Continuous, Dash, Dot or Double." => |h, c| Ok(line_names(border_now(h, c)?.style).0.into());
    setStyle "(style: ExcelScript.BorderLineStyle): void" "Draws the border with a line (None removes it): Continuous, Dash, Dot or Double." => |h, c| {
        let style = enum_value(c, 0, "BorderLineStyle")?.ok_or_else(|| c.invalid())?;
        // A border that was not there is drawn thin.
        let weight = match border_now(h, c)?.style {
            BorderStyle::None | BorderStyle::Double | BorderStyle::Dashed | BorderStyle::Dotted => "Thin",
            line => line_names(line).1,
        };
        let line = line_of(c, style, if style == "Continuous" { weight } else { "Thin" })?;
        set_border(h, c, &|border| {
            border.style = line;
            if line == BorderStyle::None {
                border.color = None;
            }
        })
    };
    getWeight "(): ExcelScript.BorderWeight" "The weight of the line: Hairline, Thin, Medium or Thick." => |h, c| Ok(line_names(border_now(h, c)?.style).1.into());
    setWeight "(weight: ExcelScript.BorderWeight): void" "Changes the weight of the line; a border that was not there is drawn continuous." => |h, c| {
        let weight = enum_value(c, 0, "BorderWeight")?.ok_or_else(|| c.invalid())?;
        let style = match line_names(border_now(h, c)?.style).0 {
            "None" => "Continuous",
            style => style,
        };
        let line = if style == "Double" { BorderStyle::Double } else { line_of(c, style, weight)? };
        set_border(h, c, &|border| border.style = line)
    };
    getColor "(): string" "The colour of the line as #RRGGBB (#000000 for automatic)." => |h, c| {
        Ok(border_now(h, c)?.color.map_or("#000000".to_string(), color_text).into())
    };
    setColor "(color: string): void" "Colours the line: #RRGGBB or a colour name. A border that was not there is drawn thin." => |h, c| {
        let color = color_arg(c, 0)?.ok_or_else(|| c.invalid())?;
        set_border(h, c, &|border| {
            border.color = Some(color);
            if border.style == BorderStyle::None {
                border.style = BorderStyle::Thin;
            }
        })
    };
});

pub(super) const FORMAT_PROTECTION: ApiClass = class!("FormatProtection", "What protecting the sheet does to the cells of a range.", {
    getLocked "(): boolean" "Whether the cells cannot be changed while the sheet is protected." => |h, c| get(h, c, |style| style.locked.into());
    setLocked "(locked: boolean): void" "Locks or unlocks the cells." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let locked = c.bool(0)?;
        h.permit(c, &Action::ChangeSheet { sheet })?;
        h.wb.update_style(sheet, rect, &|style| style.locked = locked);
        Ok(Ret::Nothing)
    };
    getFormulaHidden "(): boolean" "Whether the formulas do not show while the sheet is protected." => |h, c| get(h, c, |style| style.hidden.into());
    setFormulaHidden "(formulaHidden: boolean): void" "Hides the formulas of the cells while the sheet is protected, or not." => |h, c| {
        let (sheet, rect) = this_range(h, c)?;
        let hidden = c.bool(0)?;
        h.permit(c, &Action::ChangeSheet { sheet })?;
        h.wb.update_style(sheet, rect, &|style| style.hidden = hidden);
        Ok(Ret::Nothing)
    };
});
