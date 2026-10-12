//! What slicers and timelines decide without a window: where their parts
//! are, what a click selects, the colours of their styles.
//!
//! Everything here is measured in pixels at 100 %, from the top-left
//! corner of the object.

use opensheet_engine::slicer::{Period, SlicerItem, TimeLevel};
use opensheet_engine::table::{OFFICE_THEME, tint};
use opensheet_engine::{Color, Locale};

/// A box: x0, y0, x1, y1.
pub(super) type Box4 = (f32, f32, f32, f32);

pub(super) fn inside(b: Box4, x: f32, y: f32) -> bool {
    x >= b.0 && x < b.2 && y >= b.1 && y < b.3
}

// ---- slicers ---------------------------------------------------------------

/// Room between the frame and the buttons, between two buttons, and the
/// height of the header.
pub(super) const PAD: f32 = 8.0;
pub(super) const GAP: f32 = 2.0;
pub(super) const COLUMN_GAP: f32 = 4.0;
pub(super) const HEADER_H: f32 = 30.0;
/// Width of the scroll bar and of the two buttons of the header.
pub(super) const SCROLL_W: f32 = 12.0;
pub(super) const TOOL: f32 = 20.0;

/// Where the parts of a slicer are.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SlicerLayout {
    pub header: Option<Box4>,
    /// The "Multi-Select" and "Clear Filter" buttons of the header.
    pub multi: Option<Box4>,
    pub clear: Option<Box4>,
    /// The buttons in view: index of the item, box.
    pub buttons: Vec<(usize, Box4)>,
    /// Where the buttons are drawn (what is outside is cut off).
    pub body: Box4,
    /// The scroll bar, when the items do not all fit: track and thumb.
    pub scroll: Option<(Box4, Box4)>,
    /// Rows of buttons, those that fit, and the first one shown.
    pub rows: u32,
    pub rows_shown: u32,
    pub first_row: u32,
}

/// The width of a button of a slicer `width` wide with `columns` columns.
pub(super) fn button_width(width: f32, columns: u32, scrolling: bool) -> f32 {
    let columns = columns.max(1) as f32;
    let room = width - 2.0 * PAD - if scrolling { SCROLL_W + 2.0 } else { 0.0 };
    ((room - (columns - 1.0) * COLUMN_GAP) / columns).max(4.0)
}

/// The width a slicer needs for its buttons to be `button` wide.
pub(super) fn width_for_buttons(button: f32, columns: u32, scrolling: bool) -> f32 {
    let columns = columns.max(1) as f32;
    button * columns + (columns - 1.0) * COLUMN_GAP + 2.0 * PAD + if scrolling { SCROLL_W + 2.0 } else { 0.0 }
}

/// Lay a slicer out. `first_row` is where it is scrolled to; it comes
/// back clamped in the layout.
pub(super) fn slicer_layout(
    size: (f32, f32),
    show_header: bool,
    columns: u32,
    button_height: f32,
    items: usize,
    first_row: u32,
) -> SlicerLayout {
    let (w, h) = size;
    let columns = columns.max(1);
    let header = show_header.then_some((0.0, 0.0, w, HEADER_H.min(h)));
    let top = if show_header { HEADER_H + 4.0 } else { PAD };
    let body = (PAD, top.min(h), (w - PAD).max(PAD), (h - PAD).max(top.min(h)));
    let step = button_height.max(4.0) + GAP;
    let rows = (items as u32).div_ceil(columns);
    let rows_shown = (((body.3 - body.1 + GAP) / step).floor() as u32).max(1);
    let scrolling = rows > rows_shown;
    let first_row = if scrolling { first_row.min(rows - rows_shown) } else { 0 };
    let bw = button_width(w, columns, scrolling);
    let mut buttons = Vec::new();
    for row in first_row..(first_row + rows_shown + 1).min(rows) {
        for col in 0..columns {
            let index = (row * columns + col) as usize;
            if index >= items {
                break;
            }
            let x0 = body.0 + col as f32 * (bw + COLUMN_GAP);
            let y0 = body.1 + (row - first_row) as f32 * step;
            if y0 >= body.3 {
                break;
            }
            buttons.push((index, (x0, y0, x0 + bw, y0 + button_height.max(4.0))));
        }
    }
    let scroll = scrolling.then(|| {
        let track = (body.2 - SCROLL_W, body.1, body.2, body.3);
        let length = track.3 - track.1;
        let thumb = (length * rows_shown as f32 / rows as f32).max(16.0).min(length);
        let at = (length - thumb) * first_row as f32 / (rows - rows_shown) as f32;
        (track, (track.0, track.1 + at, track.2, track.1 + at + thumb))
    });
    let tool = |k: f32| {
        let x1 = w - PAD - k * (TOOL + 2.0);
        (x1 - TOOL, (HEADER_H - TOOL) / 2.0, x1, (HEADER_H + TOOL) / 2.0)
    };
    SlicerLayout {
        header,
        multi: show_header.then(|| tool(1.0)),
        clear: show_header.then(|| tool(0.0)),
        buttons,
        body,
        scroll,
        rows,
        rows_shown,
        first_row,
    }
}

impl SlicerLayout {
    /// The item whose button is at a point.
    pub fn button_at(&self, x: f32, y: f32) -> Option<usize> {
        if !inside(self.body, x, y) {
            return None;
        }
        self.buttons.iter().find(|(_, b)| inside(*b, x, y)).map(|(index, _)| *index)
    }

    /// The first row shown when the thumb of the scroll bar was dragged
    /// `dy` pixels from where the list was at `from`.
    pub fn scrolled(&self, from: u32, dy: f32) -> u32 {
        let Some((track, thumb)) = self.scroll else {
            return 0;
        };
        let free = (track.3 - track.1) - (thumb.3 - thumb.1);
        let hidden = self.rows.saturating_sub(self.rows_shown) as f32;
        if free <= 0.0 {
            return from;
        }
        (from as f32 + dy / free * hidden).round().clamp(0.0, hidden) as u32
    }
}

/// How a button was pressed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pick {
    /// ⌘-click, or a click while Multi-Select is on: add or remove it.
    Toggle,
    /// A click, a drag, or a click with Shift: every item from this one
    /// (an index into the items) to the one under the pointer, which is
    /// the same one for a plain click.
    Through(usize),
}

/// The items selected once the button of item `index` was picked, as
/// `SlicerItem::x`; empty when every item is (the filter is cleared).
pub(super) fn picked(items: &[SlicerItem], index: usize, pick: Pick) -> Vec<u32> {
    let Some(item) = items.get(index) else {
        return items.iter().filter(|i| i.selected).map(|i| i.x).collect();
    };
    let chosen: Vec<u32> = match pick {
        Pick::Through(from) => {
            let (a, b) = (from.min(index), from.max(index).min(items.len() - 1));
            items[a..=b].iter().map(|i| i.x).collect()
        }
        Pick::Toggle => {
            let mut now: Vec<u32> = items.iter().filter(|i| i.selected).map(|i| i.x).collect();
            match now.iter().position(|x| *x == item.x) {
                Some(at) => {
                    now.remove(at);
                }
                None => now.push(item.x),
            }
            now
        }
    };
    // Nothing selected shows everything, as does everything selected.
    if chosen.len() == items.len() { Vec::new() } else { chosen }
}

/// The colours of one state of a button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Tile {
    pub fill: u32,
    pub text: u32,
    pub border: u32,
}

/// The colours of a slicer style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SlicerLook {
    pub background: u32,
    pub border: u32,
    pub header_fill: Option<u32>,
    pub header_text: u32,
    /// The line under the header.
    pub header_rule: u32,
    pub selected: Tile,
    pub selected_no_data: Tile,
    pub unselected: Tile,
    pub unselected_no_data: Tile,
}

impl SlicerLook {
    pub fn tile(&self, selected: bool, has_data: bool) -> Tile {
        match (selected, has_data) {
            (true, true) => self.selected,
            (true, false) => self.selected_no_data,
            (false, true) => self.unselected,
            (false, false) => self.unselected_no_data,
        }
    }
}

/// The built-in slicer styles, in the order of the gallery: the two rows
/// of light styles, then the dark ones.
pub(super) const SLICER_STYLES: [&str; 14] = [
    "SlicerStyleLight1",
    "SlicerStyleLight2",
    "SlicerStyleLight3",
    "SlicerStyleLight4",
    "SlicerStyleLight5",
    "SlicerStyleLight6",
    "SlicerStyleOther1",
    "SlicerStyleOther2",
    "SlicerStyleDark1",
    "SlicerStyleDark2",
    "SlicerStyleDark3",
    "SlicerStyleDark4",
    "SlicerStyleDark5",
    "SlicerStyleDark6",
];

pub(super) const TIMELINE_STYLES: [&str; 12] = [
    "TimeSlicerStyleLight1",
    "TimeSlicerStyleLight2",
    "TimeSlicerStyleLight3",
    "TimeSlicerStyleLight4",
    "TimeSlicerStyleLight5",
    "TimeSlicerStyleLight6",
    "TimeSlicerStyleDark1",
    "TimeSlicerStyleDark2",
    "TimeSlicerStyleDark3",
    "TimeSlicerStyleDark4",
    "TimeSlicerStyleDark5",
    "TimeSlicerStyleDark6",
];

fn shade(color: u32, amount: f64) -> u32 {
    tint(Color(color), amount).0
}

/// The family ("Light", "Dark", "Other") and the number of a style name.
pub(super) fn style_parts(name: &str) -> (&str, usize) {
    let rest = name.strip_prefix("TimeSlicerStyle").or_else(|| name.strip_prefix("SlicerStyle")).unwrap_or("Light1");
    let split = rest.find(|ch: char| ch.is_ascii_digit()).unwrap_or(rest.len());
    let (family, number) = rest.split_at(split);
    (family, number.parse().unwrap_or(1))
}

/// The accent colour a style is built on: styles 1 to 6 take the six
/// accents of the theme.
fn accent(number: usize) -> u32 {
    OFFICE_THEME[4 + (number.max(1) - 1) % 6]
}

/// The colours of a slicer style; an unknown name gets the first one.
pub(super) fn slicer_look(name: &str) -> SlicerLook {
    let (family, number) = style_parts(name);
    let plain = Tile {
        fill: 0xFFFFFF,
        text: 0x000000,
        border: 0xCCCCCC,
    };
    let faded = Tile {
        fill: 0xFFFFFF,
        text: 0x959595,
        border: 0xE0E0E0,
    };
    let light = |base: u32| SlicerLook {
        background: 0xFFFFFF,
        border: 0xCCCCCC,
        header_fill: None,
        header_text: 0x000000,
        header_rule: base,
        selected: Tile {
            fill: shade(base, 0.6),
            text: 0x000000,
            border: shade(base, 0.4),
        },
        selected_no_data: Tile {
            fill: shade(base, 0.8),
            text: 0x828282,
            border: shade(base, 0.8),
        },
        unselected: plain,
        unselected_no_data: faded,
    };
    match family {
        "Dark" => {
            let base = accent(number);
            SlicerLook {
                background: 0xFFFFFF,
                border: shade(base, -0.25),
                header_fill: Some(shade(base, -0.25)),
                header_text: 0xFFFFFF,
                header_rule: shade(base, -0.25),
                selected: Tile {
                    fill: base,
                    text: 0xFFFFFF,
                    border: base,
                },
                selected_no_data: Tile {
                    fill: shade(base, 0.6),
                    text: 0xFFFFFF,
                    border: shade(base, 0.6),
                },
                unselected: plain,
                unselected_no_data: faded,
            }
        }
        // The grey style, and the black and white one.
        "Other" if number == 1 => light(0x7F7F7F),
        "Other" => SlicerLook {
            selected: Tile {
                fill: 0x262626,
                text: 0xFFFFFF,
                border: 0x262626,
            },
            selected_no_data: Tile {
                fill: 0xA6A6A6,
                text: 0xFFFFFF,
                border: 0xA6A6A6,
            },
            ..light(0x000000)
        },
        _ => light(accent(number)),
    }
}

/// The colours of a timeline style.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TimelineLook {
    pub background: u32,
    pub border: u32,
    pub header_text: u32,
    /// The selection label and the level.
    pub label_text: u32,
    /// The names of the periods.
    pub period_text: u32,
    /// The bar: selected and unselected periods, and the handles.
    pub selected: u32,
    pub unselected: u32,
    pub handle: u32,
}

pub(super) fn timeline_look(name: &str) -> TimelineLook {
    let (family, number) = style_parts(name);
    let base = accent(number);
    if family == "Dark" {
        TimelineLook {
            background: shade(base, -0.5),
            border: shade(base, -0.5),
            header_text: 0xFFFFFF,
            label_text: 0xFFFFFF,
            period_text: 0xFFFFFF,
            selected: shade(base, 0.4),
            unselected: shade(base, -0.25),
            handle: 0xFFFFFF,
        }
    } else {
        TimelineLook {
            background: 0xFFFFFF,
            border: 0xCCCCCC,
            header_text: 0x000000,
            label_text: base,
            period_text: 0x595959,
            selected: base,
            unselected: 0xE1E1E1,
            handle: shade(base, -0.25),
        }
    }
}

// ---- timelines -------------------------------------------------------------

pub(super) const TL_PAD: f32 = 10.0;
pub(super) const TL_HEADER_H: f32 = 26.0;
pub(super) const TL_LABEL_H: f32 = 22.0;
pub(super) const TL_SCROLL_H: f32 = 10.0;
/// Height of the bar of periods and of the two lines of text above it.
pub(super) const TL_BAR_H: f32 = 16.0;
pub(super) const TL_TEXT_H: f32 = 16.0;

/// Width of the tile of one period.
pub(super) fn tile_width(level: TimeLevel) -> f32 {
    match level {
        TimeLevel::Years => 84.0,
        TimeLevel::Quarters => 72.0,
        TimeLevel::Months => 48.0,
        TimeLevel::Days => 26.0,
    }
}

/// What a timeline shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct TimelineShape {
    pub size: (f32, f32),
    pub header: bool,
    pub selection_label: bool,
    pub time_level: bool,
    pub scrollbar: bool,
    pub level: TimeLevel,
    /// Number of periods of the band, and how far it is scrolled.
    pub periods: usize,
    pub scroll: f32,
}

/// Where the parts of a timeline are.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TimelineLayout {
    pub header: Option<Box4>,
    pub clear: Option<Box4>,
    pub selection_label: Option<Box4>,
    pub level: Option<Box4>,
    /// The band: what is outside is cut off.
    pub band: Box4,
    /// The line of the groups (years), of the names of the periods, and
    /// the bar.
    pub groups_y: f32,
    pub names_y: f32,
    pub bar: (f32, f32),
    pub tile: f32,
    /// Offset of the first period from the left of the band (negative
    /// once scrolled).
    pub scroll: f32,
    pub max_scroll: f32,
    pub scrollbar: Option<(Box4, Box4)>,
}

pub(super) fn timeline_layout(shape: &TimelineShape) -> TimelineLayout {
    let (w, h) = shape.size;
    let mut y = 2.0;
    let header = shape.header.then(|| {
        let b = (0.0, y, w, y + TL_HEADER_H);
        y += TL_HEADER_H;
        b
    });
    let labels = shape.selection_label || shape.time_level;
    let level_w = 96.0;
    let selection_label = shape.selection_label.then_some((TL_PAD, y, (w - TL_PAD - level_w).max(TL_PAD), y + TL_LABEL_H));
    let level = shape.time_level.then_some(((w - TL_PAD - level_w).max(TL_PAD), y, w - TL_PAD, y + TL_LABEL_H));
    if labels {
        y += TL_LABEL_H;
    }
    let bottom = h - 4.0 - if shape.scrollbar { TL_SCROLL_H + 4.0 } else { 0.0 };
    let band = (TL_PAD, y.min(bottom), (w - TL_PAD).max(TL_PAD), bottom.max(y.min(bottom)));
    let tile = tile_width(shape.level);
    let total = tile * shape.periods as f32;
    let max_scroll = (total - (band.2 - band.0)).max(0.0);
    let scroll = shape.scroll.clamp(0.0, max_scroll);
    let scrollbar = (shape.scrollbar && max_scroll > 0.0).then(|| {
        let track = (band.0, h - 4.0 - TL_SCROLL_H, band.2, h - 4.0);
        let length = track.2 - track.0;
        let thumb = (length * length / total).max(20.0).min(length);
        let at = (length - thumb) * scroll / max_scroll;
        (track, (track.0 + at, track.1, track.0 + at + thumb, track.3))
    });
    let groups_y = band.1 + 2.0;
    TimelineLayout {
        header,
        clear: header.map(|b| (w - TL_PAD - TOOL, b.1 + 3.0, w - TL_PAD, b.1 + 3.0 + TOOL)),
        selection_label,
        level,
        band,
        groups_y,
        names_y: groups_y + TL_TEXT_H,
        bar: (groups_y + 2.0 * TL_TEXT_H + 2.0, groups_y + 2.0 * TL_TEXT_H + 2.0 + TL_BAR_H),
        tile,
        scroll,
        max_scroll,
        scrollbar,
    }
}

impl TimelineLayout {
    /// Left edge of period `index`.
    pub fn period_x(&self, index: usize) -> f32 {
        self.band.0 - self.scroll + index as f32 * self.tile
    }

    /// The period at a horizontal position, clamped to the band's.
    pub fn period_at(&self, x: f32, count: usize) -> usize {
        let at = ((x - self.band.0 + self.scroll) / self.tile).floor().max(0.0) as usize;
        at.min(count.saturating_sub(1))
    }

    /// How far the band is scrolled once the thumb of its scroll bar was
    /// dragged `dx` pixels from where it was at `from`.
    pub fn scrolled(&self, from: f32, dx: f32) -> f32 {
        let Some((track, thumb)) = self.scrollbar else {
            return 0.0;
        };
        let free = (track.2 - track.0) - (thumb.2 - thumb.0);
        if free <= 0.0 {
            return from;
        }
        (from + dx / free * self.max_scroll).clamp(0.0, self.max_scroll)
    }
}

/// The periods (indices, both included) a selection of days covers.
pub(super) fn selected_periods(periods: &[Period], selection: Option<(f64, f64)>) -> Option<(usize, usize)> {
    let (start, end) = selection?;
    let first = periods.iter().position(|p| p.end >= start)?;
    let last = periods.iter().rposition(|p| p.start <= end)?;
    (first <= last).then_some((first, last))
}

/// The days of the periods `a..=b`, in either order.
pub(super) fn span_days(periods: &[Period], a: usize, b: usize) -> Option<(f64, f64)> {
    let (first, last) = (periods.get(a.min(b))?, periods.get(a.max(b))?);
    Some((first.start, last.end))
}

/// Where a band starts scrolled so that period `index` shows, a little
/// after its left edge.
pub(super) fn scroll_to_show(layout: &TimelineLayout, index: usize) -> f32 {
    (index as f32 * layout.tile - layout.tile).clamp(0.0, layout.max_scroll)
}

/// The name Excel shows for a built-in style ("Slicer Style Light 1").
pub(super) fn style_title(name: &str, locale: &Locale) -> String {
    let timeline = name.starts_with("Time");
    let (family, number) = style_parts(name);
    let lang = ["en", "fr", "es", "de", "it"].iter().position(|id| *id == locale.id).unwrap_or(0);
    let pattern: [&str; 5] = match (timeline, family) {
        (false, "Dark") => [
            "Slicer Style Dark {}",
            "Style de segment foncé\u{a0}{}",
            "Estilo de segmentación de datos oscuro {}",
            "Datenschnittformat - Dunkel {}",
            "Stile filtro dati scuro {}",
        ],
        (false, "Other") => [
            "Slicer Style Other {}",
            "Autre style de segment {}",
            "Otro estilo de segmentación de datos {}",
            "Datenschnittformat Sonstige {}",
            "Stile filtro dei dati altro {}",
        ],
        (false, _) => [
            "Slicer Style Light {}",
            "Style de segment clair\u{a0}{}",
            "Estilo de segmentación de datos claro {}",
            "Datenschnittformat - Hell {}",
            "Stile filtro dati chiaro {}",
        ],
        (true, "Dark") => [
            "Timeline Style Dark {}",
            "Style de chronologie foncé\u{a0}{}",
            "Estilo de escala de tiempo oscuro {}",
            "Zeitachsenformat - Dunkel {}",
            "Stile sequenza temporale scuro {}",
        ],
        (true, _) => [
            "Timeline Style Light {}",
            "Style de chronologie clair\u{a0}{}",
            "Estilo de escala de tiempo claro {}",
            "Zeitachsenformat - Hell {}",
            "Stile sequenza temporale chiaro {}",
        ],
    };
    pattern[lang].replace("{}", &number.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use opensheet_engine::slicer::periods;

    fn items(selected: &[bool]) -> Vec<SlicerItem> {
        selected
            .iter()
            .enumerate()
            .map(|(k, s)| SlicerItem {
                x: 10 + k as u32,
                label: format!("item {k}"),
                selected: *s,
                has_data: true,
            })
            .collect()
    }

    #[test]
    fn a_slicer_lays_its_buttons_out_in_columns_and_scrolls() {
        let layout = slicer_layout((192.0, 252.0), true, 1, 25.0, 4, 0);
        assert_eq!(layout.buttons.len(), 4);
        assert_eq!(layout.buttons[0].1, (8.0, 34.0, 184.0, 59.0));
        assert_eq!(layout.buttons[1].1.1, 61.0);
        assert!(layout.scroll.is_none());
        assert_eq!(layout.button_at(20.0, 40.0), Some(0));
        assert_eq!(layout.button_at(20.0, 60.0), None);

        // Twenty items in two columns: ten rows, seven fit and a part of
        // the eighth shows.
        let layout = slicer_layout((192.0, 252.0), true, 2, 25.0, 20, 0);
        assert_eq!((layout.rows, layout.rows_shown), (10, 7));
        let (track, thumb) = layout.scroll.unwrap();
        assert_eq!(track, (172.0, 34.0, 184.0, 244.0));
        assert_eq!(thumb.1, 34.0);
        assert_eq!(layout.buttons[1].0, 1);
        assert!((layout.buttons[1].1.0 - (8.0 + button_width(192.0, 2, true) + 4.0)).abs() < 0.01);
        // Scrolled to the end, and no further.
        let end = slicer_layout((192.0, 252.0), true, 2, 25.0, 20, 99);
        assert_eq!(end.first_row, 3);
        assert_eq!(end.buttons[0].0, 6);
        assert_eq!(end.scroll.unwrap().1.3, 244.0);
        assert_eq!(layout.scrolled(0, 1000.0), 3);
        assert_eq!(layout.scrolled(3, -1000.0), 0);
        // Without its header the buttons start at the top.
        assert_eq!(slicer_layout((192.0, 252.0), false, 1, 25.0, 4, 0).buttons[0].1.1, 8.0);
        let w = width_for_buttons(60.0, 3, false);
        assert!((button_width(w, 3, false) - 60.0).abs() < 0.01);
    }

    #[test]
    fn what_a_click_on_a_button_selects() {
        let all = items(&[true, true, true, true]);
        assert_eq!(picked(&all, 1, Pick::Through(1)), [11]);
        // ⌘-click on a slicer showing everything takes the item out.
        assert_eq!(picked(&all, 1, Pick::Toggle), [10, 12, 13]);
        let one = items(&[false, true, false, false]);
        assert_eq!(picked(&one, 3, Pick::Toggle), [11, 13]);
        // Unticking the last item shows everything again.
        assert_eq!(picked(&one, 1, Pick::Toggle), Vec::<u32>::new());
        assert_eq!(picked(&one, 3, Pick::Through(1)), [11, 12, 13]);
        assert_eq!(picked(&one, 0, Pick::Through(2)), [10, 11, 12]);
        assert_eq!(picked(&one, 3, Pick::Through(0)), Vec::<u32>::new());
    }

    #[test]
    fn styles_take_the_accents_of_the_theme() {
        let first = slicer_look("SlicerStyleLight1");
        assert_eq!(first.header_rule, OFFICE_THEME[4]);
        assert_eq!(first.tile(false, true).fill, 0xFFFFFF);
        assert_ne!(first.tile(true, true).fill, first.tile(true, false).fill);
        assert_eq!(slicer_look("SlicerStyleLight3").header_rule, OFFICE_THEME[6]);
        let dark = slicer_look("SlicerStyleDark2");
        assert_eq!((dark.selected.fill, dark.selected.text), (OFFICE_THEME[5], 0xFFFFFF));
        assert!(dark.header_fill.is_some());
        assert_eq!(slicer_look("nonsense"), first);
        assert_eq!(timeline_look("TimeSlicerStyleLight2").selected, OFFICE_THEME[5]);
        assert_eq!(style_title("SlicerStyleOther2", Locale::en()), "Slicer Style Other 2");
        assert_eq!(style_title("TimeSlicerStyleDark4", Locale::fr()), "Style de chronologie foncé\u{a0}4");
    }

    #[test]
    fn a_timeline_lays_its_band_out_and_finds_its_periods() {
        let months = periods(TimeLevel::Months, 44927.0, 45657.0, Locale::en());
        assert_eq!(months.len(), 24);
        let shape = TimelineShape {
            size: (333.0, 137.0),
            header: true,
            selection_label: true,
            time_level: true,
            scrollbar: true,
            level: TimeLevel::Months,
            periods: months.len(),
            scroll: 0.0,
        };
        let layout = timeline_layout(&shape);
        assert_eq!(layout.band, (10.0, 50.0, 323.0, 119.0));
        assert_eq!(layout.max_scroll, 24.0 * 48.0 - 313.0);
        assert_eq!(layout.period_at(10.0, 24), 0);
        assert_eq!(layout.period_at(10.0 + 48.0 * 2.5, 24), 2);
        assert_eq!(layout.period_at(5000.0, 24), 23);
        let (track, thumb) = layout.scrollbar.unwrap();
        assert_eq!((track.0, thumb.0), (10.0, 10.0));
        assert_eq!(layout.scrolled(0.0, 5000.0), layout.max_scroll);
        // Without the header and the labels the band takes their room.
        let bare = timeline_layout(&TimelineShape {
            header: false,
            selection_label: false,
            time_level: false,
            scrollbar: false,
            ..shape
        });
        assert_eq!((bare.band.1, bare.band.3), (2.0, 133.0));
        assert!(bare.scrollbar.is_none() && bare.clear.is_none());

        // March to May 2023.
        let days = span_days(&months, 4, 2).unwrap();
        assert_eq!(days, (months[2].start, months[4].end));
        assert_eq!(selected_periods(&months, Some(days)), Some((2, 4)));
        assert_eq!(selected_periods(&months, None), None);
        // A selection of days shows on the quarters it touches.
        let quarters = periods(TimeLevel::Quarters, 44927.0, 45657.0, Locale::en());
        assert_eq!(selected_periods(&quarters, Some(days)), Some((0, 1)));
        assert_eq!(scroll_to_show(&layout, 10), 9.0 * 48.0);
    }
}
